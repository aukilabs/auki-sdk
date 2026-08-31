//! Cross-platform Auki peer endpoint for typed Stream v2.
//!
//! This module owns the mechanical authenticated runtime around the portable
//! wire contract in [`super::v2`]: registration, admission dispatch, bounded
//! handshakes and writes, demand-driven consumer backpressure, typed payload
//! validation, and deterministic stream cleanup. Resource discovery, registry
//! lookup, and platform storage remain application policy.

#![forbid(unsafe_code)]

use std::{fmt, pin::Pin, time::Duration};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use auki_datatypes::{detection::DetectionFrame, map::MapUpdate, scalar};
use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AukiProtocolStream, AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, FutureExt, Stream, StreamExt, pin_mut,
};
use futures_timer::Delay;
use prost::Message;

use super::v2::{
    CameraFrame, DeclineReason, EndReason, ID, MAX_FRAME_BYTES, StreamEntry as WireStreamEntry,
    StreamManifest, StreamMessage, StreamProtocolError, StreamRequest, audio, joint_encoders,
    point_cloud, pose, read_message, stream_message, stream_request_from_wire,
    stream_request_to_wire, write_message,
};

/// Maximum number of concurrently served Stream v2 subscriptions.
pub const MAX_CONCURRENCY: usize = 16;

/// Fixed deadline for every pre-live network open, request, or reply phase.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Fixed deadline for writing one live entry or terminal frame.
pub const LIVE_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Fixed deadline for closing a stream or mounted registration.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the exact bounded Stream v2 registration.
pub fn protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, MAX_CONCURRENCY, MAX_FRAME_BYTES)
}

/// One producer item before the endpoint stamps its wire sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamItem<T> {
    /// Timestamp expressed in the clock pinned by the stream manifest.
    pub timestamp_ns: i64,
    /// Typed payload.
    pub payload: T,
}

/// Application-owned producer stream on native targets.
#[cfg(not(target_arch = "wasm32"))]
pub type SourceStream<T> =
    Pin<Box<dyn Stream<Item = Result<StreamItem<T>, String>> + Send + 'static>>;

/// Application-owned producer stream in a browser-local executor.
#[cfg(target_arch = "wasm32")]
pub type SourceStream<T> = Pin<Box<dyn Stream<Item = Result<StreamItem<T>, String>> + 'static>>;

/// Payload accepted by the native typed-stream runtime.
#[cfg(not(target_arch = "wasm32"))]
pub trait StreamPayload: Message + Default + Send + 'static {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> StreamPayload for T where T: Message + Default + Send + 'static {}

/// Payload accepted by the browser-local typed-stream runtime.
#[cfg(target_arch = "wasm32")]
pub trait StreamPayload: Message + Default + 'static {}

#[cfg(target_arch = "wasm32")]
impl<T> StreamPayload for T where T: Message + Default + 'static {}

/// Closed set of typed producer results supported by Stream v2.
pub enum StreamDispatch {
    /// Accept with camera frames.
    AcceptCamera {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<CameraFrame>,
    },
    /// Accept with point-cloud data.
    AcceptPointCloud {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<point_cloud::Data>,
    },
    /// Accept with joint-encoder data.
    AcceptJointEncoders {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<joint_encoders::Data>,
    },
    /// Accept with audio data.
    AcceptAudio {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<audio::Data>,
    },
    /// Accept with scalar data.
    AcceptScalar {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<scalar::Data>,
    },
    /// Accept with spatial transforms.
    AcceptPose {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<pose::SpatialTransform>,
    },
    /// Accept with detection frames.
    AcceptDetection {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<DetectionFrame>,
    },
    /// Accept with map updates.
    AcceptMap {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<MapUpdate>,
    },
    /// Decline without exposing a typed source.
    Decline {
        /// Stable wire reason.
        reason: DeclineReason,
    },
}

/// Application admission and source dispatch for authenticated stream opens.
///
/// Dispatch must be synchronous and cheap: it selects an already-available
/// source and performs no blocking I/O. Platform storage and asynchronous
/// preparation belong before mounting or behind the returned source stream.
#[cfg(not(target_arch = "wasm32"))]
pub trait StreamProvider: Send + Sync + 'static {
    /// Admit or decline one request using the verified remote identity.
    fn dispatch(&self, remote_peer: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch;
}

#[cfg(not(target_arch = "wasm32"))]
impl<F> StreamProvider for F
where
    F: Fn(&AuthenticatedPeer, StreamRequest) -> StreamDispatch + Send + Sync + 'static,
{
    fn dispatch(&self, remote_peer: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch {
        self(remote_peer, request)
    }
}

/// Application admission and source dispatch for authenticated stream opens.
///
/// Dispatch must be synchronous and cheap: it selects an already-available
/// source and performs no blocking I/O. Platform storage and asynchronous
/// preparation belong before mounting or behind the returned source stream.
#[cfg(target_arch = "wasm32")]
pub trait StreamProvider: 'static {
    /// Admit or decline one request using the verified remote identity.
    fn dispatch(&self, remote_peer: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch;
}

#[cfg(target_arch = "wasm32")]
impl<F> StreamProvider for F
where
    F: Fn(&AuthenticatedPeer, StreamRequest) -> StreamDispatch + 'static,
{
    fn dispatch(&self, remote_peer: &AuthenticatedPeer, request: StreamRequest) -> StreamDispatch {
        self(remote_peer, request)
    }
}

#[cfg(not(target_arch = "wasm32"))]
type SharedProvider<P> = Arc<P>;
#[cfg(target_arch = "wasm32")]
type SharedProvider<P> = Rc<P>;

/// Provider for peers that consume Stream v2 but serve no local sources.
pub fn decline_all_streams() -> impl StreamProvider {
    |_remote_peer: &AuthenticatedPeer, _request: StreamRequest| StreamDispatch::Decline {
        reason: DeclineReason::sensor_not_found(),
    }
}

/// One typed consumer item with its validated wire sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamEntry<T> {
    /// Producer timestamp.
    pub timestamp_ns: i64,
    /// Monotonic per-stream sequence number.
    pub seq: u64,
    /// Typed payload.
    pub payload: T,
}

/// Demand-driven native entry stream.
#[cfg(not(target_arch = "wasm32"))]
pub type SubscriptionEntries<T> =
    Pin<Box<dyn Stream<Item = Result<StreamEntry<T>, StreamError>> + Send + 'static>>;

/// Demand-driven browser-local entry stream.
#[cfg(target_arch = "wasm32")]
pub type SubscriptionEntries<T> =
    Pin<Box<dyn Stream<Item = Result<StreamEntry<T>, StreamError>> + 'static>>;

/// Accepted typed subscription.
///
/// `entries` reads exactly one frame per consumer poll, so a slow consumer
/// applies transport backpressure without an unbounded task or queue. It emits
/// exactly one terminal error and then ends. Dropping it cancels the stream and
/// releases its authenticated route through the SDK's RAII guard.
pub struct StreamSubscription<T> {
    /// Manifest fixed by the producer during the open handshake.
    pub manifest: StreamManifest,
    /// Validated entries followed by exactly one terminal error.
    pub entries: SubscriptionEntries<T>,
}

/// Cloneable outbound half of the Stream v2 endpoint.
#[derive(Clone)]
pub struct StreamClient {
    protocols: AukiPeerProtocols,
}

impl StreamClient {
    /// Construct an outbound client without mounting an inbound provider.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Open a typed subscription through the owning native peer's routes.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn subscribe<T>(
        &self,
        remote_peer_id: PeerId,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamEndpointError>
    where
        T: StreamPayload,
    {
        subscribe_opened(request, self.protocols.open(remote_peer_id, ID)).await
    }

    /// Open a typed subscription through one exact advertised route.
    pub async fn subscribe_exact<T>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamEndpointError>
    where
        T: StreamPayload,
    {
        subscribe_opened(
            request,
            self.protocols.open_exact(remote_peer_id, route, ID),
        )
        .await
    }
}

/// Mounted Stream v2 service plus its outbound client.
pub struct StreamEndpoint {
    client: StreamClient,
    registration: AukiProtocolRegistration,
}

impl StreamEndpoint {
    /// Mount Stream v2 on one running peer with application-owned admission.
    pub fn mount<P>(protocols: AukiPeerProtocols, provider: P) -> Result<Self, StreamEndpointError>
    where
        P: StreamProvider,
    {
        let provider = SharedProvider::new(provider);
        let registration = protocols.register(protocol_spec()?, move |mut stream| {
            let provider = SharedProvider::clone(&provider);
            async move {
                let _ = serve_and_close(&mut stream, provider.as_ref()).await;
            }
        })?;

        Ok(Self {
            client: StreamClient::new(protocols),
            registration,
        })
    }

    /// Clone the outbound client without cloning registration ownership.
    pub fn client(&self) -> StreamClient {
        self.client.clone()
    }

    /// Open a typed subscription through the owning native peer's routes.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn subscribe<T>(
        &self,
        remote_peer_id: PeerId,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamEndpointError>
    where
        T: StreamPayload,
    {
        self.client.subscribe(remote_peer_id, request).await
    }

    /// Open a typed subscription through one exact advertised route.
    pub async fn subscribe_exact<T>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, StreamEndpointError>
    where
        T: StreamPayload,
    {
        self.client
            .subscribe_exact(remote_peer_id, route, request)
            .await
    }

    /// Stop accepting subscriptions and await already-admitted handlers.
    pub async fn close(self) -> Result<(), StreamEndpointError> {
        deadline(StreamOperation::Close, self.registration.close())
            .await?
            .map_err(StreamEndpointError::Sdk)
    }
}

async fn subscribe_opened<T, F>(
    request: StreamRequest,
    opening: F,
) -> Result<StreamSubscription<T>, StreamEndpointError>
where
    T: StreamPayload,
    F: std::future::Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    validate_request(&request)?;
    let mut stream = deadline(StreamOperation::Open, opening)
        .await?
        .map_err(StreamEndpointError::Sdk)?;
    let manifest = match client_handshake(&mut stream, &request).await {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = deadline(StreamOperation::Close, stream.close()).await;
            return Err(error);
        }
    };

    Ok(StreamSubscription {
        manifest,
        entries: subscription_entries(stream),
    })
}

async fn client_handshake<S>(
    stream: &mut S,
    request: &StreamRequest,
) -> Result<StreamManifest, StreamEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    deadline(
        StreamOperation::RequestWrite,
        write_message(
            stream,
            &StreamMessage::request(stream_request_to_wire(request.clone())),
        ),
    )
    .await?
    .map_err(StreamEndpointError::Codec)?;

    let reply = deadline(StreamOperation::ReplyRead, read_message(stream))
        .await?
        .map_err(StreamEndpointError::Codec)?;
    let manifest = match reply.variant {
        Some(stream_message::Variant::Accept(manifest)) => manifest,
        Some(stream_message::Variant::Decline(reason)) => {
            validate_decline_reason(&reason)?;
            return Err(StreamEndpointError::Declined { reason });
        }
        None => {
            return Err(StreamEndpointError::Codec(
                StreamProtocolError::MissingVariant,
            ));
        }
        Some(_) => {
            return Err(StreamEndpointError::UnexpectedMessage(
                "expected Accept or Decline as first reply",
            ));
        }
    };
    validate_manifest(request, &manifest)?;
    Ok(manifest)
}

async fn serve_and_close<P>(
    stream: &mut AukiProtocolStream,
    provider: &P,
) -> Result<(), StreamEndpointError>
where
    P: StreamProvider,
{
    let remote_peer = stream.remote_peer().clone();
    let serving = serve_request(stream, &remote_peer, provider).await;
    let cleanup = deadline(StreamOperation::Close, AsyncWriteExt::close(stream))
        .await
        .and_then(|result| result.map_err(|error| StreamEndpointError::Close(error.to_string())));
    prefer_primary(serving, cleanup)
}

async fn serve_request<S, P>(
    stream: &mut S,
    remote_peer: &AuthenticatedPeer,
    provider: &P,
) -> Result<(), StreamEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: StreamProvider,
{
    let message = deadline(StreamOperation::RequestRead, read_message(stream))
        .await?
        .map_err(StreamEndpointError::Codec)?;
    let request = match message.variant {
        Some(stream_message::Variant::Request(request)) => stream_request_from_wire(request),
        None => {
            return Err(StreamEndpointError::Codec(
                StreamProtocolError::MissingVariant,
            ));
        }
        Some(_) => {
            return Err(StreamEndpointError::UnexpectedMessage(
                "expected Request as first message",
            ));
        }
    };
    validate_request(&request)?;
    let requested_resource = request.resource_id.clone();
    let dispatch = provider.dispatch(remote_peer, request);

    match dispatch {
        StreamDispatch::Decline { reason } => {
            validate_decline_reason(&reason)?;
            write_reply(stream, StreamMessage::decline(reason)).await
        }
        StreamDispatch::AcceptCamera { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptPointCloud { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptJointEncoders { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptAudio { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptScalar { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptPose { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptDetection { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
        StreamDispatch::AcceptMap { manifest, source } => {
            pump_typed(stream, &requested_resource, manifest, source).await
        }
    }
}

async fn write_reply<S>(stream: &mut S, reply: StreamMessage) -> Result<(), StreamEndpointError>
where
    S: AsyncWrite + Unpin,
{
    deadline(StreamOperation::ReplyWrite, write_message(stream, &reply))
        .await?
        .map_err(StreamEndpointError::Codec)
}

async fn pump_typed<S, T>(
    stream: &mut S,
    requested_resource: &str,
    manifest: StreamManifest,
    mut source: SourceStream<T>,
) -> Result<(), StreamEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: StreamPayload,
{
    validate_manifest_resource(requested_resource, &manifest)?;
    write_reply(stream, StreamMessage::accept(manifest)).await?;

    let mut sequence = 0_u64;
    let end_reason = loop {
        let mut remote_byte = [0_u8; 1];
        let item = {
            let remote_read = stream.read(&mut remote_byte).fuse();
            let source_next = source.next().fuse();
            pin_mut!(remote_read, source_next);
            futures::select_biased! {
                result = remote_read => {
                    return match result {
                        Ok(0) => Err(StreamEndpointError::ConsumerClosed),
                        Ok(_) => Err(StreamEndpointError::UnexpectedMessage(
                            "unexpected consumer bytes after the Stream v2 Request",
                        )),
                        Err(error) => Err(StreamEndpointError::Codec(StreamProtocolError::Io(error))),
                    };
                }
                item = source_next => item,
            }
        };

        match item {
            Some(Ok(item)) => {
                let message = StreamMessage::entry(WireStreamEntry {
                    timestamp_ns: item.timestamp_ns,
                    seq: sequence,
                    payload: item.payload.encode_to_vec(),
                });
                deadline(StreamOperation::EntryWrite, write_message(stream, &message))
                    .await?
                    .map_err(StreamEndpointError::Codec)?;
                sequence = sequence
                    .checked_add(1)
                    .ok_or(StreamEndpointError::SequenceExhausted)?;
            }
            Some(Err(detail)) => break EndReason::producer_error(detail),
            None => break EndReason::source_ended(),
        }
    };

    deadline(
        StreamOperation::EndWrite,
        write_message(stream, &StreamMessage::end_of_stream(end_reason)),
    )
    .await?
    .map_err(StreamEndpointError::Codec)
}

#[cfg(not(target_arch = "wasm32"))]
trait SubscriptionIo: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> SubscriptionIo for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

#[cfg(target_arch = "wasm32")]
trait SubscriptionIo: AsyncRead + AsyncWrite + Unpin + 'static {}

#[cfg(target_arch = "wasm32")]
impl<T> SubscriptionIo for T where T: AsyncRead + AsyncWrite + Unpin + 'static {}

fn subscription_entries<T, S>(stream: S) -> SubscriptionEntries<T>
where
    T: StreamPayload,
    S: SubscriptionIo,
{
    Box::pin(futures::stream::unfold(
        (Some(stream), 0_u64),
        |(stream, expected_sequence)| async move {
            let mut stream = stream?;
            let decoded = match read_message(&mut stream).await {
                Ok(message) => decode_live_message::<T>(message, expected_sequence),
                Err(StreamProtocolError::Io(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    Err(StreamError::ConnectionLost)
                }
                Err(error) => Err(StreamError::Protocol(error)),
            };

            match decoded {
                Ok(DecodedLiveMessage::Entry {
                    entry,
                    next_sequence,
                }) => Some((Ok(entry), (Some(stream), next_sequence))),
                Ok(DecodedLiveMessage::End(reason)) => {
                    close_consumed_stream(&mut stream).await;
                    Some((
                        Err(StreamError::EndOfStream { reason }),
                        (None, expected_sequence),
                    ))
                }
                Err(error) => {
                    close_consumed_stream(&mut stream).await;
                    Some((Err(error), (None, expected_sequence)))
                }
            }
        },
    ))
}

async fn close_consumed_stream<S>(stream: &mut S)
where
    S: AsyncWrite + Unpin,
{
    let _ = deadline(StreamOperation::Close, AsyncWriteExt::close(stream)).await;
}

enum DecodedLiveMessage<T> {
    Entry {
        entry: StreamEntry<T>,
        next_sequence: u64,
    },
    End(EndReason),
}

fn decode_live_message<T>(
    message: StreamMessage,
    expected_sequence: u64,
) -> Result<DecodedLiveMessage<T>, StreamError>
where
    T: Message + Default,
{
    match message.variant {
        Some(stream_message::Variant::Entry(entry)) => {
            if entry.seq != expected_sequence {
                return Err(StreamError::SequenceMismatch {
                    expected: expected_sequence,
                    actual: entry.seq,
                });
            }
            let next_sequence = expected_sequence
                .checked_add(1)
                .ok_or(StreamError::SequenceExhausted)?;
            let payload = T::decode(entry.payload.as_slice())
                .map_err(|error| StreamError::Protocol(StreamProtocolError::Decode(error)))?;
            Ok(DecodedLiveMessage::Entry {
                entry: StreamEntry {
                    timestamp_ns: entry.timestamp_ns,
                    seq: entry.seq,
                    payload,
                },
                next_sequence,
            })
        }
        Some(stream_message::Variant::EndOfStream(reason)) if reason.kind.is_some() => {
            Ok(DecodedLiveMessage::End(reason))
        }
        Some(stream_message::Variant::EndOfStream(_)) => Err(StreamError::Protocol(
            malformed_payload("EndReason has no kind"),
        )),
        None => Err(StreamError::Protocol(StreamProtocolError::MissingVariant)),
        Some(_) => Err(StreamError::UnexpectedMessage),
    }
}

fn validate_request(request: &StreamRequest) -> Result<(), StreamEndpointError> {
    if request.resource_id.is_empty() {
        return Err(StreamEndpointError::InvalidRequest(
            "resource_id must not be empty".into(),
        ));
    }
    Ok(())
}

fn validate_decline_reason(reason: &DeclineReason) -> Result<(), StreamEndpointError> {
    if reason.kind.is_none() {
        return Err(StreamEndpointError::Codec(malformed_payload(
            "DeclineReason has no kind",
        )));
    }
    Ok(())
}

fn malformed_payload(detail: &'static str) -> StreamProtocolError {
    StreamProtocolError::Decode(prost::DecodeError::new(detail))
}

fn validate_manifest(
    request: &StreamRequest,
    manifest: &StreamManifest,
) -> Result<(), StreamEndpointError> {
    validate_manifest_resource(&request.resource_id, manifest)
}

fn validate_manifest_resource(
    requested_resource: &str,
    manifest: &StreamManifest,
) -> Result<(), StreamEndpointError> {
    if manifest.resource_id != requested_resource {
        return Err(StreamEndpointError::ManifestMismatch {
            expected: requested_resource.to_owned(),
            actual: manifest.resource_id.clone(),
        });
    }
    Ok(())
}

async fn deadline<T>(
    operation: StreamOperation,
    future: impl std::future::Future<Output = T>,
) -> Result<T, StreamEndpointError> {
    deadline_after(operation, operation.timeout(), future).await
}

async fn deadline_after<T>(
    operation: StreamOperation,
    duration: Duration,
    future: impl std::future::Future<Output = T>,
) -> Result<T, StreamEndpointError> {
    let work = future.fuse();
    let timer = Delay::new(duration).fuse();
    pin_mut!(work, timer);
    futures::select_biased! {
        result = work => Ok(result),
        () = timer => Err(StreamEndpointError::Timeout(operation)),
    }
}

fn prefer_primary<T, E>(primary: Result<T, E>, cleanup: Result<(), E>) -> Result<T, E> {
    match primary {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}

/// One fixed-deadline Stream v2 operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamOperation {
    /// Open and mutually authenticate one application stream.
    Open,
    /// Write the consumer's initial request.
    RequestWrite,
    /// Read the producer's initial reply.
    ReplyRead,
    /// Read the producer-side initial request.
    RequestRead,
    /// Write a producer-side Accept or Decline reply.
    ReplyWrite,
    /// Write one typed live entry.
    EntryWrite,
    /// Write one terminal end frame.
    EndWrite,
    /// Close one stream or protocol registration.
    Close,
}

impl StreamOperation {
    fn timeout(self) -> Duration {
        match self {
            Self::Open
            | Self::RequestWrite
            | Self::ReplyRead
            | Self::RequestRead
            | Self::ReplyWrite => HANDSHAKE_TIMEOUT,
            Self::EntryWrite | Self::EndWrite => LIVE_WRITE_TIMEOUT,
            Self::Close => CLOSE_TIMEOUT,
        }
    }
}

impl fmt::Display for StreamOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::RequestWrite => "request write",
            Self::ReplyRead => "reply read",
            Self::RequestRead => "request read",
            Self::ReplyWrite => "reply write",
            Self::EntryWrite => "entry write",
            Self::EndWrite => "end write",
            Self::Close => "close",
        })
    }
}

/// Failure opening, serving, or closing a Stream v2 endpoint.
#[derive(Debug, thiserror::Error)]
pub enum StreamEndpointError {
    /// The SDK protocol surface rejected registration, opening, or shutdown.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// The portable Stream v2 codec rejected a frame.
    #[error("typed stream codec failed: {0}")]
    Codec(#[from] StreamProtocolError),
    /// The requested resource identity is invalid.
    #[error("typed stream request is invalid: {0}")]
    InvalidRequest(String),
    /// The producer explicitly declined the subscription.
    #[error("typed stream was declined: {reason:?}")]
    Declined {
        /// Stable protocol decline reason.
        reason: DeclineReason,
    },
    /// The peer sent a valid envelope in an invalid conversation position.
    #[error("invalid typed stream conversation: {0}")]
    UnexpectedMessage(&'static str),
    /// An accepted manifest did not identify the requested resource.
    #[error("typed stream manifest resource mismatch: expected {expected:?}, got {actual:?}")]
    ManifestMismatch {
        /// Requested resource identity.
        expected: String,
        /// Accepted resource identity.
        actual: String,
    },
    /// A fixed-deadline endpoint operation did not complete.
    #[error("typed stream {0} timed out")]
    Timeout(StreamOperation),
    /// Stream cleanup failed.
    #[error("close authenticated typed stream: {0}")]
    Close(String),
    /// The live consumer closed while its producer source was active.
    #[error("typed stream consumer closed")]
    ConsumerClosed,
    /// The producer exhausted the unsigned 64-bit sequence space.
    #[error("typed stream sequence is exhausted")]
    SequenceExhausted,
}

/// One terminal typed-stream consumer outcome.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Producer sent an explicit end reason.
    #[error("end of stream: {reason:?}")]
    EndOfStream {
        /// Typed producer reason.
        reason: EndReason,
    },
    /// The transport closed without an explicit end frame.
    #[error("connection lost")]
    ConnectionLost,
    /// The peer sent a malformed envelope or typed payload.
    #[error("protocol error: {0}")]
    Protocol(#[source] StreamProtocolError),
    /// The peer sent a valid envelope in an invalid live-stream position.
    #[error("unexpected message after Stream v2 Accept")]
    UnexpectedMessage,
    /// The producer did not send the exact next sequence number.
    #[error("stream sequence mismatch: expected {expected}, got {actual}")]
    SequenceMismatch {
        /// Exact sequence required next.
        expected: u64,
        /// Sequence received from the producer.
        actual: u64,
    },
    /// No entry can follow the maximum unsigned sequence number.
    #[error("stream sequence is exhausted")]
    SequenceExhausted,
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        task::{Context, Poll},
    };

    use futures::io::Cursor;

    use super::*;

    struct ScriptedStream {
        inbound: Cursor<Vec<u8>>,
        outbound: Vec<u8>,
        pending_after_input: bool,
        dropped: Option<Arc<AtomicBool>>,
        prefixes: Option<Arc<AtomicUsize>>,
    }

    impl ScriptedStream {
        fn new(inbound: Vec<u8>) -> Self {
            Self {
                inbound: Cursor::new(inbound),
                outbound: Vec::new(),
                pending_after_input: false,
                dropped: None,
                prefixes: None,
            }
        }

        fn producer() -> Self {
            Self {
                inbound: Cursor::new(Vec::new()),
                outbound: Vec::new(),
                pending_after_input: true,
                dropped: None,
                prefixes: None,
            }
        }
    }

    impl AsyncRead for ScriptedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if buffer.len() == 4
                && let Some(prefixes) = &self.prefixes
            {
                prefixes.fetch_add(1, Ordering::SeqCst);
            }
            match Pin::new(&mut self.inbound).poll_read(context, buffer) {
                Poll::Ready(Ok(0)) if self.pending_after_input => Poll::Pending,
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

    impl Drop for ScriptedStream {
        fn drop(&mut self) {
            if let Some(dropped) = &self.dropped {
                dropped.store(true, Ordering::SeqCst);
            }
        }
    }

    fn camera_manifest(resource_id: &str) -> StreamManifest {
        StreamManifest {
            resource_id: resource_id.into(),
            sensor_id: resource_id.into(),
            sensor_hash: "sensor-hash".into(),
            clock_peer_id: "producer".into(),
            clock_id: "session/monotonic".into(),
            clock_hash: "clock-hash".into(),
            frame_id: "camera/front/optical".into(),
            frame_hash: "frame-hash".into(),
            ..Default::default()
        }
    }

    fn camera_entry(sequence: u64, bytes: &[u8]) -> StreamMessage {
        StreamMessage::entry(WireStreamEntry {
            timestamp_ns: sequence as i64,
            seq: sequence,
            payload: CameraFrame {
                dynamic_intrinsics: None,
                frame: bytes.to_vec(),
            }
            .encode_to_vec(),
        })
    }

    #[test]
    fn spec_mounts_the_exact_stream_contract() {
        let spec = protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ID);
        assert_eq!(spec.max_concurrency(), MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_FRAME_BYTES);
    }

    #[tokio::test]
    async fn client_handshake_preserves_request_and_validates_manifest() {
        let request = StreamRequest {
            source_peer_id: "source-peer".into(),
            resource_id: "camera/front".into(),
            ..Default::default()
        };
        let manifest = camera_manifest("camera/front");
        let mut inbound = Vec::new();
        write_message(&mut inbound, &StreamMessage::accept(manifest.clone()))
            .await
            .unwrap();
        let mut stream = ScriptedStream::new(inbound);

        assert_eq!(
            client_handshake(&mut stream, &request).await.unwrap(),
            manifest
        );
        let mut outbound = Cursor::new(stream.outbound.clone());
        let sent = read_message(&mut outbound).await.unwrap();
        assert!(matches!(
            sent.variant,
            Some(stream_message::Variant::Request(wire))
                if stream_request_from_wire(wire.clone()) == request
        ));
    }

    #[tokio::test]
    async fn producer_preserves_manifest_entries_sequence_and_end() {
        let manifest = camera_manifest("camera/front");
        let source: SourceStream<CameraFrame> = Box::pin(futures::stream::iter([
            Ok(StreamItem {
                timestamp_ns: 99,
                payload: CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![1, 2, 3],
                },
            }),
            Ok(StreamItem {
                timestamp_ns: 100,
                payload: CameraFrame {
                    dynamic_intrinsics: None,
                    frame: vec![4, 5],
                },
            }),
        ]));
        let mut stream = ScriptedStream::producer();

        pump_typed(&mut stream, "camera/front", manifest.clone(), source)
            .await
            .unwrap();

        let mut outbound = Cursor::new(stream.outbound.clone());
        assert!(matches!(
            read_message(&mut outbound).await.unwrap().variant,
            Some(stream_message::Variant::Accept(actual)) if actual == manifest
        ));
        for (sequence, expected) in [(0, vec![1, 2, 3]), (1, vec![4, 5])] {
            let message = read_message(&mut outbound).await.unwrap();
            let entry = match message.variant {
                Some(stream_message::Variant::Entry(entry)) => entry,
                other => panic!("expected Entry, got {other:?}"),
            };
            assert_eq!(entry.seq, sequence);
            assert_eq!(
                CameraFrame::decode(entry.payload.as_slice()).unwrap().frame,
                expected
            );
        }
        assert!(matches!(
            read_message(&mut outbound).await.unwrap().variant,
            Some(stream_message::Variant::EndOfStream(reason))
                if matches!(reason.kind, Some(super::super::v2::end_reason::Kind::SourceEnded(_)))
        ));
    }

    #[test]
    fn live_decoder_rejects_sequence_gaps_before_exposing_payload() {
        let result = decode_live_message::<CameraFrame>(camera_entry(7, &[1]), 0);
        assert!(matches!(
            result,
            Err(StreamError::SequenceMismatch {
                expected: 0,
                actual: 7
            })
        ));
    }

    #[test]
    fn live_decoder_rejects_invalid_typed_payload() {
        let result = decode_live_message::<CameraFrame>(
            StreamMessage::entry(WireStreamEntry {
                timestamp_ns: 7,
                seq: 0,
                payload: vec![0xff],
            }),
            0,
        );
        assert!(matches!(
            result,
            Err(StreamError::Protocol(StreamProtocolError::Decode(_)))
        ));
    }

    #[test]
    fn live_decoder_rejects_an_end_without_a_reason_kind() {
        let result = decode_live_message::<CameraFrame>(
            StreamMessage::end_of_stream(EndReason { kind: None }),
            0,
        );
        assert!(matches!(
            result,
            Err(StreamError::Protocol(StreamProtocolError::Decode(_)))
        ));
    }

    #[tokio::test]
    async fn consumer_entries_are_demand_driven_and_drop_the_owned_stream() {
        let mut inbound = Vec::new();
        write_message(&mut inbound, &camera_entry(0, &[1]))
            .await
            .unwrap();
        write_message(&mut inbound, &camera_entry(1, &[2]))
            .await
            .unwrap();
        let prefixes = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let mut stream = ScriptedStream::new(inbound);
        stream.prefixes = Some(Arc::clone(&prefixes));
        stream.dropped = Some(Arc::clone(&dropped));
        let mut entries = subscription_entries::<CameraFrame, _>(stream);

        assert_eq!(prefixes.load(Ordering::SeqCst), 0);
        assert_eq!(entries.next().await.unwrap().unwrap().seq, 0);
        assert_eq!(prefixes.load(Ordering::SeqCst), 1);
        drop(entries);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn manifest_must_commit_to_the_requested_resource() {
        let error = validate_manifest_resource("camera/front", &camera_manifest("camera/other"))
            .unwrap_err();
        assert!(matches!(
            error,
            StreamEndpointError::ManifestMismatch { .. }
        ));
    }

    #[test]
    fn expired_deadline_reports_the_interrupted_operation() {
        let result = futures::executor::block_on(deadline_after(
            StreamOperation::ReplyRead,
            Duration::ZERO,
            futures::future::pending::<()>(),
        ));
        assert!(matches!(
            result,
            Err(StreamEndpointError::Timeout(StreamOperation::ReplyRead))
        ));
    }

    #[test]
    fn exchange_failure_wins_over_cleanup_failure() {
        assert_eq!(
            prefer_primary::<(), _>(Err("exchange"), Err("cleanup")),
            Err("exchange")
        );
        assert_eq!(prefer_primary(Ok(7), Err("cleanup")), Err("cleanup"));
        assert_eq!(prefer_primary::<_, &str>(Ok(7), Ok(())), Ok(7));
    }
}
