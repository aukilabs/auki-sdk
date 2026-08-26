use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use auki_network::{
    protocol_ids::STREAM_V0_2_0,
    stream_protocol::{
        CameraFrame, DeclineReason, EndReason, MAX_FRAME_BYTES, StreamEntry as WireStreamEntry,
        StreamMessage, StreamProtocolError, StreamRequest, audio, joint_encoders, point_cloud,
        pose, read_message, stream_message, stream_request_from_wire, stream_request_to_wire,
        write_message,
    },
    stream_runtime::{
        SourceStream, StreamDispatch, StreamEntry, StreamError, StreamProvider, StreamSubscription,
        decline_all_streams,
    },
};
use auki_p2p::PeerId;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, SinkExt, Stream, StreamExt, channel::mpsc};
use parking_lot::Mutex;
use prost::Message;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::{
    io_tasks::{DomainIoTaskError, DomainIoTaskLease, DomainIoTasks},
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
};

const STREAM_V2_MAX_CONCURRENCY: usize = 16;
const STREAM_PRE_LIVE_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_CONSUMER_QUEUE_CAPACITY: usize = 8;

/// Private authenticated adapter for the native typed stream 0.2.0 payload.
///
/// The provider and wire payload types remain the retained SDK types. This
/// service only replaces their transport owner: every inbound and outbound
/// application byte now follows mutual Domain authentication on the owning
/// [`DomainProtocols`] node.
#[derive(Clone)]
pub(crate) struct Streams {
    protocols: DomainProtocols,
    io_tasks: DomainIoTasks,
    lifecycle: CancellationToken,
    provider: Arc<Mutex<StreamProvider>>,
}

impl Streams {
    pub(super) fn new(
        protocols: DomainProtocols,
        io_tasks: DomainIoTasks,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            protocols,
            io_tasks,
            lifecycle,
            provider: Arc::new(Mutex::new(decline_all_streams())),
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, StreamsError> {
        let spec =
            DomainProtocolSpec::new(STREAM_V0_2_0, STREAM_V2_MAX_CONCURRENCY, MAX_FRAME_BYTES)?;
        let streams = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let streams = streams.clone();
                async move {
                    if let Err(error) = streams.handle(stream).await {
                        tracing::warn!(%error, "authenticated typed-stream session failed");
                    }
                }
            })
            .map_err(StreamsError::Protocol)
    }

    /// Replace the provider used for subsequent authenticated subscriptions.
    ///
    /// This is a private staging seam. P12 installs the provider composed by
    /// `DomainBuilder` before public `Domain::join` returns.
    pub(crate) fn set_provider(&self, provider: StreamProvider) -> Result<(), StreamsError> {
        self.ensure_running()?;
        let mut current = self.provider.lock();
        self.ensure_running()?;
        *current = provider;
        Ok(())
    }

    /// Open one authenticated typed subscription to `expected_peer`.
    ///
    /// The route/authentication open, request write, and Accept/Decline read
    /// are independently bounded pre-live phases. Once accepted, a stream is
    /// deliberately live until either endpoint, its source, or the Domain
    /// ends it; there is no idle or total-byte timeout.
    pub(crate) async fn open<T>(
        &self,
        expected_peer: PeerId,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamsError>
    where
        T: Message + Default + Send + 'static,
    {
        self.ensure_running()?;
        let open = timeout(
            STREAM_PRE_LIVE_TIMEOUT,
            self.protocols.open(expected_peer, STREAM_V0_2_0),
        );
        let mut stream = tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => return Err(StreamsError::Stopped),
            result = open => match result {
                Err(_) => return Err(StreamsError::Timeout(STREAM_PRE_LIVE_TIMEOUT)),
                Ok(Err(error)) => return Err(StreamsError::Protocol(error)),
                Ok(Ok(stream)) => stream,
            },
        };

        run_codec_phase(
            &self.lifecycle,
            write_message(
                &mut stream,
                &StreamMessage::request(stream_request_to_wire(request)),
            ),
        )
        .await?;
        let reply = run_codec_phase(&self.lifecycle, read_message(&mut stream)).await?;
        let manifest = match reply.variant {
            Some(stream_message::Variant::Accept(manifest)) => manifest,
            Some(stream_message::Variant::Decline(reason)) => {
                return Err(StreamsError::Declined { reason });
            }
            _ => {
                return Err(unexpected_message(
                    "expected Accept or Decline as first reply",
                ));
            }
        };
        self.ensure_running()?;

        let (sender, receiver) = mpsc::channel(STREAM_CONSUMER_QUEUE_CAPACITY);
        let lease = self
            .io_tasks
            .spawn(consumer_reader_task::<_, T>(stream, sender))
            .map_err(StreamsError::from_io_task)?;
        let entries = LeaseRetainingEntries::new(receiver, lease, self.lifecycle.clone());

        Ok(StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        })
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), StreamsError> {
        debug_assert_eq!(stream.max_frame_bytes(), MAX_FRAME_BYTES);
        let requester = stream.remote_peer().peer_id;
        let provider = {
            self.ensure_running()?;
            let provider = self.provider.lock();
            self.ensure_running()?;
            Arc::clone(&provider)
        };
        serve_stream(requester, &mut stream, provider, self.lifecycle.clone()).await
    }

    fn ensure_running(&self) -> Result<(), StreamsError> {
        if self.lifecycle.is_cancelled() {
            Err(StreamsError::Stopped)
        } else {
            Ok(())
        }
    }
}

async fn serve_stream<S>(
    requester: PeerId,
    stream: &mut S,
    provider: StreamProvider,
    lifecycle: CancellationToken,
) -> Result<(), StreamsError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = run_codec_phase(&lifecycle, read_message(stream)).await?;
    let request = match request.variant {
        Some(stream_message::Variant::Request(request)) => stream_request_from_wire(request),
        _ => return Err(unexpected_message("expected Request as first message")),
    };
    if lifecycle.is_cancelled() {
        return Err(StreamsError::Stopped);
    }

    // The provider is deliberately synchronous and must remain sync-fast. It
    // sees the mutually authenticated requester Peer ID, not a cached
    // membership or known-peer authorization decision.
    let dispatch = provider(requester, request);
    if lifecycle.is_cancelled() {
        return Err(StreamsError::Stopped);
    }

    match dispatch {
        StreamDispatch::Decline { reason } => {
            run_codec_phase(
                &lifecycle,
                write_message(stream, &StreamMessage::decline(reason)),
            )
            .await
        }
        StreamDispatch::AcceptCamera { manifest, source } => {
            pump_typed::<_, CameraFrame>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptPointCloud { manifest, source } => {
            pump_typed::<_, point_cloud::Data>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptJointEncoders { manifest, source } => {
            pump_typed::<_, joint_encoders::Data>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptAudio { manifest, source } => {
            pump_typed::<_, audio::Data>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptScalar { manifest, source } => {
            pump_typed::<_, auki_datatypes::scalar::Data>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptPose { manifest, source } => {
            pump_typed::<_, pose::SpatialTransform>(stream, manifest, source, lifecycle).await
        }
        StreamDispatch::AcceptDetection { manifest, source } => {
            pump_typed::<_, auki_datatypes::detection::DetectionFrame>(
                stream, manifest, source, lifecycle,
            )
            .await
        }
        StreamDispatch::AcceptMap { manifest, source } => {
            pump_typed::<_, auki_datatypes::map::MapUpdate>(stream, manifest, source, lifecycle)
                .await
        }
    }
}

async fn pump_typed<S, T>(
    stream: &mut S,
    manifest: auki_network::stream_protocol::StreamManifest,
    mut source: SourceStream<T>,
    lifecycle: CancellationToken,
) -> Result<(), StreamsError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: Message + Default + Send + 'static,
{
    run_codec_phase(
        &lifecycle,
        write_message(stream, &StreamMessage::accept(manifest)),
    )
    .await?;

    let mut sequence = 0_u64;
    let end_reason = loop {
        let mut remote_byte = [0_u8; 1];
        let item = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => break EndReason::producer_shutting_down(),
            result = stream.read(&mut remote_byte) => {
                return Err(match result {
                    Ok(0) => StreamsError::Codec(StreamProtocolError::Io(
                        std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "typed stream consumer closed while the producer source was idle",
                        ),
                    )),
                    Ok(_) => unexpected_message(
                        "unexpected consumer bytes after the typed stream Request",
                    ),
                    Err(error) => StreamsError::Codec(StreamProtocolError::Io(error)),
                });
            }
            item = source.next() => item,
        };
        match item {
            Some(Ok(item)) => {
                let message = StreamMessage::entry(WireStreamEntry {
                    timestamp_ns: item.timestamp_ns,
                    seq: sequence,
                    payload: item.payload.encode_to_vec(),
                });
                tokio::select! {
                    biased;
                    _ = lifecycle.cancelled() => {
                        break EndReason::producer_shutting_down();
                    }
                    result = write_message(stream, &message) => result?,
                }
                sequence = sequence.wrapping_add(1);
            }
            Some(Err(detail)) => break EndReason::producer_error(detail),
            None => break EndReason::source_ended(),
        }
    };

    // Best effort, matching the retained stream contract. Ordered Domain
    // shutdown still owns and may abort this handler if the peer is not
    // draining, so cleanup never depends on this final write succeeding.
    let end = StreamMessage::end_of_stream(end_reason);
    tokio::select! {
        biased;
        _ = lifecycle.cancelled() => {}
        result = write_message(stream, &end) => result?,
    }
    Ok(())
}

async fn run_codec_phase<T, F>(
    lifecycle: &CancellationToken,
    operation: F,
) -> Result<T, StreamsError>
where
    F: Future<Output = Result<T, StreamProtocolError>>,
{
    tokio::select! {
        biased;
        _ = lifecycle.cancelled() => Err(StreamsError::Stopped),
        result = timeout(STREAM_PRE_LIVE_TIMEOUT, operation) => match result {
            Err(_) => Err(StreamsError::Timeout(STREAM_PRE_LIVE_TIMEOUT)),
            Ok(result) => result.map_err(StreamsError::Codec),
        },
    }
}

async fn consumer_reader_task<S, T>(
    mut stream: S,
    mut sender: mpsc::Sender<Result<StreamEntry<T>, StreamError>>,
) where
    S: AsyncRead + Unpin,
    T: Message + Default + Send + 'static,
{
    loop {
        let message = match read_message(&mut stream).await {
            Ok(message) => message,
            Err(StreamProtocolError::Io(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                let _ = sender.send(Err(StreamError::ConnectionLost)).await;
                return;
            }
            Err(error) => {
                let _ = sender.send(Err(StreamError::Protocol(error))).await;
                return;
            }
        };

        match message.variant {
            Some(stream_message::Variant::Entry(entry)) => {
                let payload = match T::decode(&*entry.payload) {
                    Ok(payload) => payload,
                    Err(error) => {
                        let _ = sender
                            .send(Err(StreamError::Protocol(StreamProtocolError::Decode(
                                error,
                            ))))
                            .await;
                        return;
                    }
                };
                if sender
                    .send(Ok(StreamEntry {
                        timestamp_ns: entry.timestamp_ns,
                        seq: entry.seq,
                        payload,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Some(stream_message::Variant::EndOfStream(reason)) => {
                let _ = sender.send(Err(StreamError::EndOfStream { reason })).await;
                return;
            }
            _ => {
                let _ = sender
                    .send(Err(StreamError::Protocol(StreamProtocolError::Decode(
                        prost::DecodeError::new("unexpected message after Accept"),
                    ))))
                    .await;
                return;
            }
        }
    }
}

/// Entry stream that owns the corresponding Domain I/O task lease.
///
/// Dropping this value cancels a reader blocked on network I/O or a full
/// consumer queue. Domain/task closure produces at most one synthetic final
/// `ConnectionLost`; a terminal item emitted by the reader remains the only
/// terminal item otherwise.
struct LeaseRetainingEntries<T> {
    receiver: mpsc::Receiver<Result<StreamEntry<T>, StreamError>>,
    lease: Option<DomainIoTaskLease>,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    done: bool,
}

impl<T> LeaseRetainingEntries<T> {
    fn new(
        receiver: mpsc::Receiver<Result<StreamEntry<T>, StreamError>>,
        lease: DomainIoTaskLease,
        lifecycle: CancellationToken,
    ) -> Self {
        Self {
            receiver,
            lease: Some(lease),
            cancelled: Box::pin(lifecycle.cancelled_owned()),
            done: false,
        }
    }
}

impl<T> Unpin for LeaseRetainingEntries<T> {}

impl<T> Stream for LeaseRetainingEntries<T> {
    type Item = Result<StreamEntry<T>, StreamError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }

        if this.cancelled.as_mut().poll(context).is_ready() {
            this.done = true;
            this.lease.take();
            return Poll::Ready(Some(Err(StreamError::ConnectionLost)));
        }

        match Pin::new(&mut this.receiver).poll_next(context) {
            Poll::Ready(Some(item)) => {
                if item.is_err() {
                    this.done = true;
                    this.lease.take();
                }
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => {
                this.done = true;
                this.lease.take();
                Poll::Ready(Some(Err(StreamError::ConnectionLost)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

fn unexpected_message(detail: &'static str) -> StreamsError {
    StreamsError::Codec(StreamProtocolError::Decode(prost::DecodeError::new(detail)))
}

#[derive(Debug, thiserror::Error)]
pub enum StreamsError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("typed stream protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("typed stream codec failed: {0}")]
    Codec(#[from] StreamProtocolError),
    #[error("typed stream was declined: {reason:?}")]
    Declined { reason: DeclineReason },
    #[error("typed stream pre-live phase exceeded {0:?}")]
    Timeout(Duration),
    #[error("typed stream I/O task host is unavailable")]
    IoTask,
    #[error("the Domain reached its {maximum}-task typed stream I/O limit")]
    IoTaskCapacityExceeded { maximum: usize },
}

impl StreamsError {
    fn from_io_task(error: DomainIoTaskError) -> Self {
        match error {
            DomainIoTaskError::Stopped => Self::Stopped,
            DomainIoTaskError::HostStopped => Self::IoTask,
            DomainIoTaskError::CapacityExceeded { maximum } => {
                Self::IoTaskCapacityExceeded { maximum }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use futures::io::Cursor;

    use super::*;
    use crate::authenticated_runtime::RuntimeFailureSignal;

    struct ScriptedStream {
        inbound: Cursor<Vec<u8>>,
        outbound: Vec<u8>,
        keep_read_open: bool,
    }

    impl ScriptedStream {
        fn new(inbound: Vec<u8>) -> Self {
            Self {
                inbound: Cursor::new(inbound),
                outbound: Vec::new(),
                keep_read_open: true,
            }
        }

        fn closing_after(inbound: Vec<u8>) -> Self {
            Self {
                inbound: Cursor::new(inbound),
                outbound: Vec::new(),
                keep_read_open: false,
            }
        }
    }

    impl AsyncRead for ScriptedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            match Pin::new(&mut self.inbound).poll_read(context, buffer) {
                Poll::Ready(Ok(0)) if self.keep_read_open => Poll::Pending,
                result => result,
            }
        }
    }

    impl AsyncWrite for ScriptedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.outbound.extend_from_slice(buffer);
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn camera_manifest() -> auki_network::stream_protocol::StreamManifest {
        auki_network::stream_protocol::StreamManifest {
            resource_id: "camera/front".into(),
            sensor_id: "camera/front".into(),
            sensor_hash: "sensor-hash".into(),
            clock_peer_id: "producer".into(),
            clock_id: "session/monotonic".into(),
            clock_hash: "clock-hash".into(),
            frame_id: "camera/front/optical".into(),
            frame_hash: "frame-hash".into(),
            ..Default::default()
        }
    }

    struct PendingCameraSource {
        dropped: Arc<AtomicBool>,
    }

    impl Stream for PendingCameraSource {
        type Item = Result<auki_network::stream_runtime::StreamItem<CameraFrame>, String>;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    impl Drop for PendingCameraSource {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn idle_source_remote_eof_or_bytes_drop_source_and_release_handler() {
        for unexpected_byte in [false, true] {
            let request = StreamRequest {
                source_peer_id: "source-peer".into(),
                resource_id: "camera/front".into(),
                from: auki_network::stream_protocol::ReadFrom::Latest,
            };
            let mut inbound = Vec::new();
            write_message(
                &mut inbound,
                &StreamMessage::request(stream_request_to_wire(request)),
            )
            .await
            .unwrap();
            if unexpected_byte {
                inbound.push(0xff);
            }

            let dropped = Arc::new(AtomicBool::new(false));
            let provider_dropped = Arc::clone(&dropped);
            let provider: StreamProvider =
                Arc::new(move |_requester, _request| StreamDispatch::AcceptCamera {
                    manifest: camera_manifest(),
                    source: Box::pin(PendingCameraSource {
                        dropped: Arc::clone(&provider_dropped),
                    }),
                });
            let mut stream = ScriptedStream::closing_after(inbound);

            let error = tokio::time::timeout(
                Duration::from_secs(1),
                serve_stream(
                    PeerId::random(),
                    &mut stream,
                    provider,
                    CancellationToken::new(),
                ),
            )
            .await
            .expect("remote termination must release the inbound handler")
            .expect_err("remote termination must not look like a clean source end");

            if unexpected_byte {
                assert!(matches!(
                    error,
                    StreamsError::Codec(StreamProtocolError::Decode(_))
                ));
            } else {
                assert!(matches!(
                    error,
                    StreamsError::Codec(StreamProtocolError::Io(ref error))
                        if error.kind() == io::ErrorKind::UnexpectedEof
                ));
            }
            assert!(
                dropped.load(Ordering::SeqCst),
                "the idle provider source must be dropped with its handler"
            );

            let mut outbound = Cursor::new(stream.outbound);
            assert!(matches!(
                read_message(&mut outbound).await.unwrap().variant,
                Some(stream_message::Variant::Accept(_))
            ));
        }
    }

    #[tokio::test]
    async fn authenticated_envelope_preserves_request_manifest_entries_and_end() {
        let request = StreamRequest {
            source_peer_id: "source-peer".into(),
            resource_id: "camera/front".into(),
            from: auki_network::stream_protocol::ReadFrom::FromTimestamp(42),
        };
        let mut inbound = Vec::new();
        write_message(
            &mut inbound,
            &StreamMessage::request(stream_request_to_wire(request.clone())),
        )
        .await
        .unwrap();

        let seen = Arc::new(Mutex::new(None));
        let provider_seen = Arc::clone(&seen);
        let manifest = camera_manifest();
        let provider_manifest = manifest.clone();
        let provider: StreamProvider = Arc::new(move |_requester, actual| {
            *provider_seen.lock() = Some(actual);
            StreamDispatch::AcceptCamera {
                manifest: provider_manifest.clone(),
                source: Box::pin(futures::stream::iter(vec![Ok(
                    auki_network::stream_runtime::StreamItem {
                        timestamp_ns: 99,
                        payload: CameraFrame {
                            dynamic_intrinsics: None,
                            frame: vec![1, 2, 3],
                        },
                    },
                )])),
            }
        });
        let mut stream = ScriptedStream::new(inbound);
        serve_stream(
            PeerId::random(),
            &mut stream,
            provider,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(*seen.lock(), Some(request));

        let mut outbound = Cursor::new(stream.outbound);
        let accept = read_message(&mut outbound).await.unwrap();
        assert!(matches!(
            accept.variant,
            Some(stream_message::Variant::Accept(actual)) if actual == manifest
        ));
        let entry = read_message(&mut outbound).await.unwrap();
        let entry = match entry.variant {
            Some(stream_message::Variant::Entry(entry)) => entry,
            other => panic!("expected Entry, got {other:?}"),
        };
        assert_eq!(entry.timestamp_ns, 99);
        assert_eq!(entry.seq, 0);
        assert_eq!(
            CameraFrame::decode(&*entry.payload).unwrap().frame,
            [1, 2, 3]
        );
        let end = read_message(&mut outbound).await.unwrap();
        assert!(matches!(
            end.variant,
            Some(stream_message::Variant::EndOfStream(reason))
                if matches!(
                    reason.kind,
                    Some(auki_network::stream_protocol::end_reason::Kind::SourceEnded(_))
                )
        ));
    }

    #[tokio::test]
    async fn interrupted_consumer_yields_one_connection_lost_terminator() {
        let payload = CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![4, 5, 6],
        }
        .encode_to_vec();
        let mut inbound = Vec::new();
        write_message(
            &mut inbound,
            &StreamMessage::entry(WireStreamEntry {
                timestamp_ns: 7,
                seq: 3,
                payload,
            }),
        )
        .await
        .unwrap();
        let (sender, mut receiver) = mpsc::channel(STREAM_CONSUMER_QUEUE_CAPACITY);
        consumer_reader_task::<_, CameraFrame>(Cursor::new(inbound), sender).await;

        let entry = receiver.next().await.unwrap().unwrap();
        assert_eq!(entry.timestamp_ns, 7);
        assert_eq!(entry.seq, 3);
        assert_eq!(entry.payload.frame, [4, 5, 6]);
        assert!(matches!(
            receiver.next().await,
            Some(Err(StreamError::ConnectionLost))
        ));
        assert!(receiver.next().await.is_none());
    }

    #[tokio::test]
    async fn invalid_typed_payload_never_emits_an_entry() {
        let mut inbound = Vec::new();
        write_message(
            &mut inbound,
            &StreamMessage::entry(WireStreamEntry {
                timestamp_ns: 7,
                seq: 3,
                payload: vec![0xff],
            }),
        )
        .await
        .unwrap();
        let (sender, mut receiver) = mpsc::channel(STREAM_CONSUMER_QUEUE_CAPACITY);
        consumer_reader_task::<_, CameraFrame>(Cursor::new(inbound), sender).await;

        assert!(matches!(
            receiver.next().await,
            Some(Err(StreamError::Protocol(StreamProtocolError::Decode(_))))
        ));
        assert!(receiver.next().await.is_none());
    }

    struct CountedReadStream {
        inner: Cursor<Vec<u8>>,
        prefixes: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    impl AsyncRead for CountedReadStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if buffer.len() == 4 {
                self.prefixes.fetch_add(1, Ordering::SeqCst);
            }
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    impl Drop for CountedReadStream {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_a_full_consumer_queue_cancels_its_owned_reader() {
        let mut inbound = Vec::new();
        for sequence in 0..64_u64 {
            write_message(
                &mut inbound,
                &StreamMessage::entry(WireStreamEntry {
                    timestamp_ns: sequence as i64,
                    seq: sequence,
                    payload: CameraFrame {
                        dynamic_intrinsics: None,
                        frame: vec![sequence as u8],
                    }
                    .encode_to_vec(),
                }),
            )
            .await
            .unwrap();
        }

        let lifecycle = CancellationToken::new();
        let (fatal, mut failures) = tokio::sync::mpsc::unbounded_channel::<RuntimeFailureSignal>();
        let (tasks, host) = DomainIoTasks::new(lifecycle.clone(), fatal);
        let host = tokio::spawn(host.run());
        let prefixes = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let stream = CountedReadStream {
            inner: Cursor::new(inbound),
            prefixes: Arc::clone(&prefixes),
            dropped: Arc::clone(&dropped),
        };
        let (sender, receiver) = mpsc::channel(STREAM_CONSUMER_QUEUE_CAPACITY);
        let lease = tasks
            .spawn(consumer_reader_task::<_, CameraFrame>(stream, sender))
            .unwrap();
        let entries = LeaseRetainingEntries::new(receiver, lease, lifecycle.clone());

        tokio::time::timeout(Duration::from_secs(1), async {
            while prefixes.load(Ordering::SeqCst) <= STREAM_CONSUMER_QUEUE_CAPACITY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reader must reach bounded queue backpressure");
        assert!(prefixes.load(Ordering::SeqCst) < 64);

        drop(entries);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropping entries must cancel and drop its blocked reader");

        lifecycle.cancel();
        host.await.unwrap();
        assert!(failures.try_recv().is_err());
    }
}
