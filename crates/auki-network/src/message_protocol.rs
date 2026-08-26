//! `/auki/message/0.1.0` receiver-owned live message channels.
//!
//! Channels are bounded and ephemeral. This module exposes no filesystem,
//! storage-path, replay, retry-queue, or materialization API.

pub use crate::message_codec::{MAX_MESSAGE_FRAME_BYTES, MessageProtocolError};
pub(crate) use crate::message_codec::{
    decode_message_frame, decode_open_frame, read_ack_frame, read_frame_body, read_frame_length,
    read_open_response, write_ack_frame, write_message_frame, write_open_frame,
    write_open_response,
};
use crate::resources_v3_protocol::MessageChannelResource;
use auki_datatypes::message::Message;
use auki_registry::RegistryRef;
use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::{PeerId, StreamProtocol};
use libp2p_stream::Control;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};

/// Receiver-owned typed-message protocol identifier.
pub const MESSAGE_PROTOCOL: &str = "/auki/message/0.1.0";
pub(crate) const MAX_INBOUND_MESSAGE_FRAME_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const CHANNEL_CAPACITY: usize = 16;
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct MessageFrameMemoryBudget {
    permits: Arc<Semaphore>,
}

impl MessageFrameMemoryBudget {
    pub(crate) fn new() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(MAX_INBOUND_MESSAGE_FRAME_MEMORY_BYTES)),
        }
    }

    async fn reserve(
        &self,
        validated_frame_len: u32,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::AcquireError> {
        self.permits
            .clone()
            .acquire_many_owned(validated_frame_len)
            .await
    }
}

/// One live inbound event with its Noise-authenticated sender identity.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundMessage {
    /// Authenticated peer that opened the message substream.
    pub sender: PeerId,
    /// Opaque message envelope; the network does not interpret its type or payload.
    pub message: Message,
}

/// Failure to atomically register a local message channel resource.
#[derive(Debug, Error)]
pub enum RegistrationError {
    #[error("network runtime is closed")]
    RuntimeClosed,
    #[error("message channel receiver capacity must be greater than zero")]
    ZeroCapacity,
    #[error("channel owner {actual} does not match local peer {expected}")]
    OwnerMismatch { expected: String, actual: String },
    #[error("message channel already registered: {owner_peer_id}/{resource_id}")]
    DuplicateChannel {
        owner_peer_id: String,
        resource_id: String,
    },
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] crate::resources_v3_protocol::ResourcesProtocolError),
}

/// Failure while opening an exact receiver-owned channel.
#[derive(Debug, Error)]
pub enum OpenMessageChannelError {
    #[error("open_stream: {0}")]
    OpenStream(#[source] libp2p_stream::OpenStreamError),
    #[error("protocol: {0}")]
    Protocol(#[source] MessageProtocolError),
    #[error("receiver rejected channel {owner_peer_id}/{resource_id}")]
    Rejected {
        owner_peer_id: PeerId,
        resource_id: String,
    },
    #[error("message channel open timed out after {0:?}")]
    Timeout(Duration),
}

/// Failure while sending and awaiting a transport acceptance ACK.
#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("message channel is closed")]
    Closed,
    #[error("protocol: {0}")]
    Protocol(#[source] MessageProtocolError),
    #[error("receiver acked sequence {actual}, expected {expected}")]
    AckSequenceMismatch { expected: u64, actual: u64 },
}

#[derive(Clone)]
/// Cloneable sender for one persistent live message substream.
pub struct MessageChannelSender {
    inner: Arc<MessageChannelSenderInner>,
}

struct MessageChannelSenderInner {
    requests: mpsc::Sender<SendRequest>,
    cancel: watch::Sender<bool>,
}

impl Drop for MessageChannelSenderInner {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

struct SendRequest {
    message: Message,
    result: oneshot::Sender<Result<(), SendMessageError>>,
}

impl MessageChannelSender {
    /// Send one opaque envelope and resolve after receiver-runtime queueing.
    ///
    /// Success is transport acceptance only, never application semantic
    /// acceptance. If the receiver enqueues the event but its ACK is lost,
    /// this returns an error even though delivery may already have occurred.
    /// Callers must treat send errors as indeterminate and must not
    /// automatically retry. The SDK never queues, retries, or replays sends.
    pub async fn send(
        &self,
        r#type: impl Into<String>,
        timestamp_ns: i64,
        payload: impl Into<Vec<u8>>,
    ) -> Result<(), SendMessageError> {
        let (result, response) = oneshot::channel();
        self.inner
            .requests
            .send(SendRequest {
                message: Message {
                    r#type: r#type.into(),
                    timestamp_ns,
                    payload: payload.into(),
                },
                result,
            })
            .await
            .map_err(|_| SendMessageError::Closed)?;
        response.await.map_err(|_| SendMessageError::Closed)?
    }
}

#[derive(Clone)]
pub(crate) struct MessageChannelRouter {
    inner: Arc<RouterInner>,
}

struct RouterInner {
    owner_peer_id: PeerId,
    closed: AtomicBool,
    channels: Mutex<HashMap<(PeerId, String), EndpointState>>,
}

struct EndpointState {
    sender: mpsc::Sender<InboundMessage>,
    cancel: watch::Sender<bool>,
    resource: MessageChannelResource,
}

impl MessageChannelRouter {
    pub(crate) fn new(owner_peer_id: PeerId) -> Self {
        Self {
            inner: Arc::new(RouterInner {
                owner_peer_id,
                closed: AtomicBool::new(false),
                channels: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub(crate) fn register(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelRegistration, RegistrationError> {
        if receiver_capacity == 0 {
            return Err(RegistrationError::ZeroCapacity);
        }
        resource
            .validate()
            .map_err(RegistrationError::InvalidResource)?;
        if resource.owner_peer_id != self.inner.owner_peer_id {
            return Err(RegistrationError::OwnerMismatch {
                expected: self.inner.owner_peer_id.to_string(),
                actual: resource.owner_peer_id.to_string(),
            });
        }

        let key = (resource.owner_peer_id, resource.resource_id.clone());
        let mut channels = self
            .inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned");
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(RegistrationError::RuntimeClosed);
        }
        if channels.contains_key(&key) {
            return Err(RegistrationError::DuplicateChannel {
                owner_peer_id: key.0.to_string(),
                resource_id: key.1,
            });
        }

        let (sender, receiver) = mpsc::channel(receiver_capacity);
        let (cancel, _) = watch::channel(false);
        channels.insert(
            key.clone(),
            EndpointState {
                sender,
                cancel,
                resource: resource.clone(),
            },
        );
        drop(channels);

        Ok(MessageChannelRegistration {
            resource,
            receiver,
            router: self.clone(),
            key,
        })
    }

    pub(crate) fn catalog(&self) -> Vec<MessageChannelResource> {
        self.inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned")
            .values()
            .map(|endpoint| endpoint.resource.clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn lookup(
        &self,
        owner_peer_id: PeerId,
        resource_id: &str,
    ) -> Option<MessageChannelEndpoint> {
        self.inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned")
            .get(&(owner_peer_id, resource_id.to_owned()))
            .map(|endpoint| MessageChannelEndpoint {
                sender: endpoint.sender.clone(),
                cancelled: endpoint.cancel.subscribe(),
            })
    }

    fn lookup_exact(
        &self,
        owner_peer_id: PeerId,
        resource_id: &str,
        expected_clock: &RegistryRef,
    ) -> Option<MessageChannelEndpoint> {
        self.inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned")
            .get(&(owner_peer_id, resource_id.to_owned()))
            .filter(|endpoint| endpoint.resource.clock == *expected_clock)
            .map(|endpoint| MessageChannelEndpoint {
                sender: endpoint.sender.clone(),
                cancelled: endpoint.cancel.subscribe(),
            })
    }

    fn unregister(&self, key: &(PeerId, String)) {
        if let Some(endpoint) = self
            .inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned")
            .remove(key)
        {
            let _ = endpoint.cancel.send(true);
        }
    }

    pub(crate) fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let endpoints = self
            .inner
            .channels
            .lock()
            .expect("message channel router mutex poisoned")
            .drain()
            .map(|(_, endpoint)| endpoint)
            .collect::<Vec<_>>();
        for endpoint in endpoints {
            let _ = endpoint.cancel.send(true);
        }
    }
}

/// Runtime-owned registration and bounded live receiver for a message channel.
///
/// Created only by [`crate::NetworkRuntime::register_message_channel`]; routing
/// internals and standalone registration are intentionally crate-private.
///
/// Dropping it deregisters the catalog row and cancels all handlers currently
/// targeting the channel.
pub struct MessageChannelRegistration {
    resource: MessageChannelResource,
    receiver: mpsc::Receiver<InboundMessage>,
    router: MessageChannelRouter,
    key: (PeerId, String),
}

impl MessageChannelRegistration {
    /// The exact catalog identity atomically bound to this receiver.
    pub fn resource(&self) -> &MessageChannelResource {
        &self.resource
    }

    /// Receive the next live event, or `None` after deregistration/closure.
    pub async fn recv(&mut self) -> Option<InboundMessage> {
        self.receiver.recv().await
    }
}

impl Drop for MessageChannelRegistration {
    fn drop(&mut self) {
        self.router.unregister(&self.key);
    }
}

pub(crate) struct MessageChannelEndpoint {
    sender: mpsc::Sender<InboundMessage>,
    cancelled: watch::Receiver<bool>,
}

impl MessageChannelEndpoint {
    #[cfg(test)]
    pub(crate) async fn deliver(
        &self,
        event: InboundMessage,
    ) -> Result<(), mpsc::error::SendError<InboundMessage>> {
        self.sender.send(event).await
    }

    pub(crate) async fn cancelled(&mut self) {
        if *self.cancelled.borrow() {
            return;
        }
        let _ = self.cancelled.changed().await;
    }
}

pub(crate) async fn open_message_channel(
    mut control: Control,
    owner_peer_id: PeerId,
    resource_id: String,
    expected_clock: RegistryRef,
    runtime_shutdown: watch::Receiver<bool>,
    lifeline: watch::Receiver<()>,
) -> Result<MessageChannelSender, OpenMessageChannelError> {
    let proto = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_owned())
        .expect("MESSAGE_PROTOCOL is a valid libp2p protocol id");
    let mut substream =
        tokio::time::timeout(OPEN_TIMEOUT, control.open_stream(owner_peer_id, proto))
            .await
            .map_err(|_| OpenMessageChannelError::Timeout(OPEN_TIMEOUT))?
            .map_err(OpenMessageChannelError::OpenStream)?;

    write_open_frame(&mut substream, owner_peer_id, &resource_id, &expected_clock)
        .await
        .map_err(OpenMessageChannelError::Protocol)?;
    let accepted = tokio::time::timeout(OPEN_TIMEOUT, read_open_response(&mut substream))
        .await
        .map_err(|_| OpenMessageChannelError::Timeout(OPEN_TIMEOUT))?
        .map_err(OpenMessageChannelError::Protocol)?;
    if !accepted {
        return Err(OpenMessageChannelError::Rejected {
            owner_peer_id,
            resource_id,
        });
    }

    let (requests, receiver) = mpsc::channel(CHANNEL_CAPACITY);
    let (cancel, cancel_rx) = watch::channel(false);
    tokio::spawn(run_outbound_channel(
        substream,
        receiver,
        cancel_rx,
        runtime_shutdown,
        lifeline,
    ));
    Ok(MessageChannelSender {
        inner: Arc::new(MessageChannelSenderInner { requests, cancel }),
    })
}

async fn run_outbound_channel(
    substream: libp2p::Stream,
    requests: mpsc::Receiver<SendRequest>,
    cancel: watch::Receiver<bool>,
    runtime_shutdown: watch::Receiver<bool>,
    lifeline: watch::Receiver<()>,
) {
    run_outbound_io(substream, requests, cancel, runtime_shutdown, lifeline).await;
}

async fn wait_for_cancellation(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    let _ = cancel.changed().await;
}

async fn wait_for_lifeline_end(lifeline: &mut watch::Receiver<()>) {
    let _ = lifeline.changed().await;
}

async fn run_outbound_io<S>(
    mut substream: S,
    mut requests: mpsc::Receiver<SendRequest>,
    mut cancel: watch::Receiver<bool>,
    mut runtime_shutdown: watch::Receiver<bool>,
    mut lifeline: watch::Receiver<()>,
) where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut sequence = 1u64;
    loop {
        let request = tokio::select! {
            biased;
            _ = wait_for_cancellation(&mut cancel) => return,
            _ = wait_for_cancellation(&mut runtime_shutdown) => return,
            _ = wait_for_lifeline_end(&mut lifeline) => return,
            request = requests.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };
        let SendRequest {
            message,
            mut result,
        } = request;
        let write = tokio::select! {
            biased;
            _ = wait_for_cancellation(&mut cancel) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = wait_for_cancellation(&mut runtime_shutdown) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = wait_for_lifeline_end(&mut lifeline) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = result.closed() => return,
            write = write_message_frame(&mut substream, sequence, &message) => write,
        };
        if let Err(error) = write {
            let _ = result.send(Err(SendMessageError::Protocol(error)));
            return;
        }

        let ack = tokio::select! {
            biased;
            _ = wait_for_cancellation(&mut cancel) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = wait_for_cancellation(&mut runtime_shutdown) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = wait_for_lifeline_end(&mut lifeline) => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = result.closed() => return,
            ack = read_ack_frame(&mut substream) => ack,
        };
        match ack {
            Ok(actual) if actual == sequence => {
                let _ = result.send(Ok(()));
            }
            Ok(actual) => {
                let _ = result.send(Err(SendMessageError::AckSequenceMismatch {
                    expected: sequence,
                    actual,
                }));
                return;
            }
            Err(error) => {
                let _ = result.send(Err(SendMessageError::Protocol(error)));
                return;
            }
        }
        sequence = sequence.wrapping_add(1).max(1);
    }
}

pub(crate) async fn handle_inbound_substream(
    sender_peer_id: PeerId,
    substream: libp2p::Stream,
    router: MessageChannelRouter,
    frame_memory: MessageFrameMemoryBudget,
    runtime_shutdown: watch::Receiver<bool>,
    revocation: watch::Receiver<bool>,
) {
    handle_inbound_io(
        sender_peer_id,
        substream,
        router,
        frame_memory,
        runtime_shutdown,
        revocation,
    )
    .await;
}

async fn handle_inbound_io<S>(
    sender_peer_id: PeerId,
    mut substream: S,
    router: MessageChannelRouter,
    frame_memory: MessageFrameMemoryBudget,
    mut runtime_shutdown: watch::Receiver<bool>,
    mut revocation: watch::Receiver<bool>,
) where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let open_frame_len = tokio::select! {
        biased;
        _ = revocation.changed() => return,
        _ = runtime_shutdown.changed() => return,
        frame_len = read_frame_length(&mut substream) => match frame_len {
            Ok(frame_len) => frame_len,
            Err(_) => return,
        },
    };
    let open_frame_memory = tokio::select! {
        biased;
        _ = revocation.changed() => return,
        _ = runtime_shutdown.changed() => return,
        permit = frame_memory.reserve(open_frame_len) => match permit {
            Ok(permit) => permit,
            Err(_) => return,
        },
    };
    let open_payload = tokio::select! {
        biased;
        _ = revocation.changed() => return,
        _ = runtime_shutdown.changed() => return,
        payload = read_frame_body(&mut substream, open_frame_len) => match payload {
            Ok(payload) => payload,
            Err(_) => return,
        },
    };
    let (owner_peer_id, resource_id, expected_clock) = match decode_open_frame(&open_payload) {
        Ok(open) => open,
        Err(_) => return,
    };
    drop(open_frame_memory);
    let Some(mut endpoint) = router.lookup_exact(owner_peer_id, &resource_id, &expected_clock)
    else {
        let _ = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            response = write_open_response(&mut substream, false) => response,
        };
        return;
    };
    let accepted = tokio::select! {
        biased;
        _ = revocation.changed() => return,
        _ = runtime_shutdown.changed() => return,
        _ = endpoint.cancelled() => return,
        response = write_open_response(&mut substream, true) => response,
    };
    if accepted.is_err() {
        return;
    }

    loop {
        // Idle streams consume neither application queue capacity nor frame
        // memory: the fixed-size length prefix is read first.
        let frame_len = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            frame_len = read_frame_length(&mut substream) => match frame_len {
                Ok(frame_len) => frame_len,
                Err(_) => return,
            },
        };
        let frame_memory_permit = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            permit = frame_memory.reserve(frame_len) => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        let payload = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            payload = read_frame_body(&mut substream, frame_len) => match payload {
                Ok(payload) => payload,
                Err(_) => return,
            },
        };
        let (sequence, message) = match decode_message_frame(&payload) {
            Ok(frame) => frame,
            Err(_) => return,
        };
        // Application capacity is reserved only after a complete validated
        // frame exists, immediately before enqueue.
        let app_sender = endpoint.sender.clone();
        let permit = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            permit = app_sender.reserve_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            _ = tokio::task::yield_now() => {
                permit.send(InboundMessage {
                sender: sender_peer_id,
                message,
                });
            }
        }
        drop(frame_memory_permit);
        // Transport ack means the runtime accepted the event into the live
        // receiver queue. It deliberately says nothing about app semantics.
        let ack = tokio::select! {
            biased;
            _ = revocation.changed() => return,
            _ = runtime_shutdown.changed() => return,
            _ = endpoint.cancelled() => return,
            ack = write_ack_frame(&mut substream, sequence) => ack,
        };
        if ack.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANNEL_CAPACITY, InboundMessage, MESSAGE_PROTOCOL, MessageChannelRouter,
        MessageChannelSender, MessageChannelSenderInner, MessageFrameMemoryBudget,
        MessageProtocolError, SendMessageError, decode_open_frame, handle_inbound_io,
        read_ack_frame, read_open_response, run_outbound_io, write_ack_frame, write_message_frame,
        write_open_frame,
    };
    use crate::message_codec::read_message_frame;
    use crate::resources_v3_protocol::MessageChannelResource;
    use auki_datatypes::message::Message;
    use auki_registry::RegistryRef;
    use futures::{
        AsyncRead, AsyncWrite,
        io::Cursor,
        task::{Context, Poll},
    };
    use std::{
        collections::VecDeque,
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        task::Waker,
    };

    struct InstrumentedIo {
        input: Cursor<Vec<u8>>,
        bytes_read: Arc<AtomicUsize>,
        output: Arc<Mutex<Vec<u8>>>,
        pending_at_eof: bool,
    }

    impl AsyncRead for InstrumentedIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.pending_at_eof && self.input.position() == self.input.get_ref().len() as u64 {
                return Poll::Pending;
            }
            let result = Pin::new(&mut self.input).poll_read(cx, buf);
            if let Poll::Ready(Ok(read)) = result {
                self.bytes_read.fetch_add(read, Ordering::SeqCst);
            }
            result
        }
    }

    impl AsyncWrite for InstrumentedIo {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.output.lock().unwrap().extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct BlockingWriteIo {
        write_started: Option<tokio::sync::oneshot::Sender<()>>,
    }

    impl AsyncRead for BlockingWriteIo {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for BlockingWriteIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if let Some(started) = self.write_started.take() {
                let _ = started.send(());
            }
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[derive(Default)]
    struct Pipe {
        bytes: VecDeque<u8>,
        reader: Option<Waker>,
        closed: bool,
    }

    struct LoopbackIo {
        incoming: Arc<Mutex<Pipe>>,
        outgoing: Arc<Mutex<Pipe>>,
        fail_writes_after: Option<usize>,
        bytes_written: usize,
    }

    impl LoopbackIo {
        fn pair_with_receiver_ack_loss() -> (Self, Self) {
            let sender_to_receiver = Arc::new(Mutex::new(Pipe::default()));
            let receiver_to_sender = Arc::new(Mutex::new(Pipe::default()));
            (
                Self {
                    incoming: receiver_to_sender.clone(),
                    outgoing: sender_to_receiver.clone(),
                    fail_writes_after: None,
                    bytes_written: 0,
                },
                Self {
                    incoming: sender_to_receiver,
                    outgoing: receiver_to_sender,
                    // The accepted open response is six bytes. The next write
                    // is the ACK frame, which this deterministic transport
                    // drops by closing the sender-facing pipe.
                    fail_writes_after: Some(6),
                    bytes_written: 0,
                },
            )
        }
    }

    impl AsyncRead for LoopbackIo {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            let mut incoming = self.incoming.lock().unwrap();
            if !incoming.bytes.is_empty() {
                let read = buf.len().min(incoming.bytes.len());
                for slot in &mut buf[..read] {
                    *slot = incoming.bytes.pop_front().unwrap();
                }
                return Poll::Ready(Ok(read));
            }
            if incoming.closed {
                return Poll::Ready(Ok(0));
            }
            incoming.reader = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    impl AsyncWrite for LoopbackIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self
                .fail_writes_after
                .is_some_and(|limit| self.bytes_written >= limit)
            {
                let mut outgoing = self.outgoing.lock().unwrap();
                outgoing.closed = true;
                if let Some(reader) = outgoing.reader.take() {
                    reader.wake();
                }
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "deterministic ACK loss",
                )));
            }
            let mut outgoing = self.outgoing.lock().unwrap();
            outgoing.bytes.extend(buf);
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
            drop(outgoing);
            self.bytes_written += buf.len();
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            let mut outgoing = self.outgoing.lock().unwrap();
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
            Poll::Ready(Ok(()))
        }
    }

    impl Drop for LoopbackIo {
        fn drop(&mut self) {
            let mut outgoing = self.outgoing.lock().unwrap();
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
        }
    }

    fn outbound_sender(
        requests: tokio::sync::mpsc::Sender<super::SendRequest>,
        cancel: tokio::sync::watch::Sender<bool>,
    ) -> MessageChannelSender {
        MessageChannelSender {
            inner: Arc::new(MessageChannelSenderInner { requests, cancel }),
        }
    }

    fn owner() -> libp2p::PeerId {
        crate::PeerIdentity::from_seed(&[71; 32]).peer_id()
    }

    fn sender() -> libp2p::PeerId {
        crate::PeerIdentity::from_seed(&[72; 32]).peer_id()
    }

    fn resource(owner_peer_id: libp2p::PeerId, id: &str) -> MessageChannelResource {
        MessageChannelResource {
            owner_peer_id,
            resource_id: id.into(),
            clock: RegistryRef {
                peer_id: owner_peer_id.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        }
    }

    fn frame_memory() -> MessageFrameMemoryBudget {
        MessageFrameMemoryBudget::new()
    }

    #[test]
    fn protocol_id_is_locked() {
        assert_eq!(MESSAGE_PROTOCOL, "/auki/message/0.1.0");
    }

    #[tokio::test]
    async fn open_frame_carries_the_full_expected_clock_reference() {
        let expected = resource(owner(), "events");
        let mut bytes = Vec::new();
        write_open_frame(
            &mut bytes,
            expected.owner_peer_id,
            &expected.resource_id,
            &expected.clock,
        )
        .await
        .unwrap();

        let payload_len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(payload_len, bytes.len() - 4);
        let (decoded_owner, decoded_resource_id, decoded_clock) =
            decode_open_frame(&bytes[4..]).unwrap();
        assert_eq!(decoded_owner, expected.owner_peer_id);
        assert_eq!(decoded_resource_id, expected.resource_id);
        assert_eq!(decoded_clock, expected.clock);
    }

    #[tokio::test]
    async fn stale_clock_is_rejected_before_message_payload_bytes_are_read() {
        let router = MessageChannelRouter::new(owner());
        let mut current = resource(owner(), "events");
        current.clock.hash = "current-clock-hash".into();
        let mut receiver = router.register(current.clone(), CHANNEL_CAPACITY).unwrap();
        let mut stale = current;
        stale.clock.hash = "stale-clock-hash".into();

        let mut input = Vec::new();
        write_open_frame(
            &mut input,
            stale.owner_peer_id,
            &stale.resource_id,
            &stale.clock,
        )
        .await
        .unwrap();
        let open_frame_bytes = input.len();
        write_message_frame(
            &mut input,
            1,
            &Message {
                r#type: "must-not-read".into(),
                timestamp_ns: 1,
                payload: vec![0xde, 0xad, 0xbe, 0xef],
            },
        )
        .await
        .unwrap();

        let bytes_read = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(Vec::new()));
        let io = InstrumentedIo {
            input: Cursor::new(input),
            bytes_read: bytes_read.clone(),
            output: output.clone(),
            pending_at_eof: false,
        };
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (_revocation_tx, revocation_rx) = tokio::sync::watch::channel(false);
        handle_inbound_io(
            sender(),
            io,
            router,
            frame_memory(),
            shutdown_rx,
            revocation_rx,
        )
        .await;

        assert_eq!(bytes_read.load(Ordering::SeqCst), open_frame_bytes);
        let response_bytes = output.lock().unwrap().clone();
        assert!(
            !read_open_response(&mut Cursor::new(response_bytes))
                .await
                .unwrap()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), receiver.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn registration_rejects_owner_mismatch_and_duplicate_channel_ids() {
        let router = MessageChannelRouter::new(owner());
        let mismatch = router.register(resource(sender(), "events"), CHANNEL_CAPACITY);
        assert!(matches!(
            mismatch,
            Err(super::RegistrationError::OwnerMismatch { .. })
        ));

        let _receiver = router
            .register(resource(owner(), "events"), CHANNEL_CAPACITY)
            .unwrap();
        let duplicate = router.register(resource(owner(), "events"), CHANNEL_CAPACITY);
        assert!(matches!(
            duplicate,
            Err(super::RegistrationError::DuplicateChannel { .. })
        ));
    }

    #[tokio::test]
    async fn registration_binds_catalog_row_and_authenticated_sender_delivery() {
        let router = MessageChannelRouter::new(owner());
        let mut receiver = router
            .register(resource(owner(), "events"), CHANNEL_CAPACITY)
            .unwrap();
        assert_eq!(router.catalog(), vec![receiver.resource().clone()]);

        let endpoint = router.lookup(owner(), "events").unwrap();
        let message = Message {
            r#type: "opaque.vendor/type-a".into(),
            timestamp_ns: 17,
            payload: vec![0x00, 0xff],
        };
        endpoint
            .deliver(InboundMessage {
                sender: sender(),
                message: message.clone(),
            })
            .await
            .unwrap();

        let event = receiver.recv().await.unwrap();
        assert_eq!(event.sender, sender());
        assert_eq!(event.message, message);
    }

    #[tokio::test]
    async fn dropping_registration_removes_catalog_cancels_endpoint_and_allows_reregistration() {
        let router = MessageChannelRouter::new(owner());
        let receiver = router
            .register(resource(owner(), "events"), CHANNEL_CAPACITY)
            .unwrap();
        let mut endpoint = router.lookup(owner(), "events").unwrap();
        drop(receiver);

        endpoint.cancelled().await;
        assert!(router.catalog().is_empty());
        assert!(router.lookup(owner(), "events").is_none());
        assert!(
            router
                .register(resource(owner(), "events"), CHANNEL_CAPACITY)
                .is_ok()
        );
    }

    #[tokio::test]
    async fn router_shutdown_closes_receiver_and_rejects_late_registration() {
        let router = MessageChannelRouter::new(owner());
        let mut receiver = router
            .register(resource(owner(), "shutdown"), CHANNEL_CAPACITY)
            .unwrap();

        router.shutdown();

        assert!(receiver.recv().await.is_none());
        assert!(matches!(
            router.register(resource(owner(), "late"), CHANNEL_CAPACITY),
            Err(super::RegistrationError::RuntimeClosed)
        ));
    }

    #[tokio::test]
    async fn message_and_ack_frames_round_trip_with_transport_sequence_outside_message() {
        let message = Message {
            r#type: "type-b".into(),
            timestamp_ns: -9,
            payload: vec![1, 2, 3, 4],
        };
        let mut bytes = Vec::new();
        write_message_frame(&mut bytes, 44, &message).await.unwrap();
        let (sequence, decoded) = read_message_frame(&mut Cursor::new(bytes)).await.unwrap();
        assert_eq!(sequence, 44);
        assert_eq!(decoded, message);

        let mut ack = Vec::new();
        write_ack_frame(&mut ack, sequence).await.unwrap();
        assert_eq!(read_ack_frame(&mut Cursor::new(ack)).await.unwrap(), 44);
    }

    #[tokio::test]
    async fn ack_loss_after_enqueue_reports_send_error_with_event_already_delivered() {
        let router = MessageChannelRouter::new(owner());
        let mut receiver = router
            .register(resource(owner(), "ack-loss"), CHANNEL_CAPACITY)
            .unwrap();
        let (mut sender_io, receiver_io) = LoopbackIo::pair_with_receiver_ack_loss();
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (_revocation_tx, revocation_rx) = tokio::sync::watch::channel(false);
        let inbound = tokio::spawn(handle_inbound_io(
            sender(),
            receiver_io,
            router,
            frame_memory(),
            shutdown_rx,
            revocation_rx,
        ));

        write_open_frame(
            &mut sender_io,
            owner(),
            "ack-loss",
            &resource(owner(), "ack-loss").clock,
        )
        .await
        .unwrap();
        assert!(read_open_response(&mut sender_io).await.unwrap());

        let (requests_tx, requests_rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let message_sender = outbound_sender(requests_tx, cancel_tx);
        let (_runtime_shutdown_tx, runtime_shutdown_rx) = tokio::sync::watch::channel(false);
        let (_lifeline_tx, lifeline_rx) = tokio::sync::watch::channel(());
        let outbound = tokio::spawn(run_outbound_io(
            sender_io,
            requests_rx,
            cancel_rx,
            runtime_shutdown_rx,
            lifeline_rx,
        ));

        let send = message_sender
            .send("application.command.v1", 42, b"opaque".to_vec())
            .await;
        assert!(
            matches!(send, Err(SendMessageError::Protocol(_))),
            "missing ACK must make transport delivery indeterminate"
        );
        let event = receiver
            .recv()
            .await
            .expect("receiver may already hold the enqueued event");
        assert_eq!(event.sender, sender());
        assert_eq!(event.message.r#type, "application.command.v1");
        assert_eq!(event.message.timestamp_ns, 42);
        assert_eq!(event.message.payload, b"opaque");

        inbound.await.unwrap();
        outbound.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_frame_is_rejected() {
        let mut bytes = Cursor::new(vec![0, 0, 0, 1, 0xff]);
        assert!(matches!(
            read_message_frame(&mut bytes).await,
            Err(MessageProtocolError::UnexpectedFrameKind(0xff))
        ));
    }

    #[tokio::test]
    async fn one_decoded_frame_waits_for_app_capacity_without_reading_another_frame() {
        let router = MessageChannelRouter::new(owner());
        let receiver = router
            .register(resource(owner(), "bounded"), CHANNEL_CAPACITY)
            .unwrap();
        let mut input = Vec::new();
        write_open_frame(
            &mut input,
            owner(),
            "bounded",
            &resource(owner(), "bounded").clock,
        )
        .await
        .unwrap();
        for sequence in 1..=CHANNEL_CAPACITY as u64 {
            write_message_frame(
                &mut input,
                sequence,
                &Message {
                    r#type: format!("type-{sequence}"),
                    timestamp_ns: sequence as i64,
                    payload: vec![sequence as u8; 32],
                },
            )
            .await
            .unwrap();
        }
        let waiting_sequence = CHANNEL_CAPACITY as u64 + 1;
        write_message_frame(
            &mut input,
            waiting_sequence,
            &Message {
                r#type: "waiting-for-app".into(),
                timestamp_ns: waiting_sequence as i64,
                payload: vec![0xaa; 4096],
            },
        )
        .await
        .unwrap();
        let bytes_before_next_frame = input.len();
        write_message_frame(
            &mut input,
            waiting_sequence + 1,
            &Message {
                r#type: "must-not-be-read".into(),
                timestamp_ns: (waiting_sequence + 1) as i64,
                payload: vec![0xbb; 4096],
            },
        )
        .await
        .unwrap();

        let bytes_read = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(Vec::new()));
        let io = InstrumentedIo {
            input: Cursor::new(input),
            bytes_read: bytes_read.clone(),
            output: output.clone(),
            pending_at_eof: true,
        };
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (_revocation_tx, revocation_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(handle_inbound_io(
            sender(),
            io,
            router,
            frame_memory(),
            shutdown_rx,
            revocation_rx,
        ));

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while output.lock().unwrap().len() < 6 + CHANNEL_CAPACITY * 13 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("open response and first sixteen ACKs");
        assert_eq!(
            bytes_read.load(Ordering::SeqCst),
            bytes_before_next_frame,
            "only one budgeted frame may wait for application queue capacity"
        );

        drop(receiver);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("channel deregistration closes handler")
            .unwrap();
    }

    #[tokio::test]
    async fn runtime_shutdown_cancels_handler_waiting_for_a_payload() {
        let router = MessageChannelRouter::new(owner());
        let _receiver = router
            .register(resource(owner(), "shutdown"), CHANNEL_CAPACITY)
            .unwrap();
        let mut input = Vec::new();
        write_open_frame(
            &mut input,
            owner(),
            "shutdown",
            &resource(owner(), "shutdown").clock,
        )
        .await
        .unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let io = InstrumentedIo {
            input: Cursor::new(input),
            bytes_read: Arc::new(AtomicUsize::new(0)),
            output: output.clone(),
            pending_at_eof: true,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let (_revocation_tx, revocation_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(handle_inbound_io(
            sender(),
            io,
            router,
            frame_memory(),
            shutdown_rx,
            revocation_rx,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while output.lock().unwrap().len() < 6 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("open response");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("runtime shutdown closes handler")
            .unwrap();
    }

    #[tokio::test]
    async fn runtime_shutdown_closes_an_idle_outbound_task_with_sender_retained() {
        let (requests_tx, requests_rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let sender = outbound_sender(requests_tx, cancel_tx);
        let (runtime_shutdown_tx, runtime_shutdown_rx) = tokio::sync::watch::channel(false);
        let (_lifeline_tx, lifeline_rx) = tokio::sync::watch::channel(());
        let io = InstrumentedIo {
            input: Cursor::new(Vec::new()),
            bytes_read: Arc::new(AtomicUsize::new(0)),
            output: Arc::new(Mutex::new(Vec::new())),
            pending_at_eof: true,
        };
        let task = tokio::spawn(run_outbound_io(
            io,
            requests_rx,
            cancel_rx,
            runtime_shutdown_rx,
            lifeline_rx,
        ));

        runtime_shutdown_tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("runtime shutdown closes idle outbound task")
            .unwrap();
        assert!(matches!(
            sender.send("after-shutdown", 1, vec![1]).await,
            Err(SendMessageError::Closed)
        ));
    }

    #[tokio::test]
    async fn lifeline_drop_cancels_an_outbound_task_blocked_writing_a_frame() {
        let (requests_tx, requests_rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let sender = outbound_sender(requests_tx, cancel_tx);
        let (_runtime_shutdown_tx, runtime_shutdown_rx) = tokio::sync::watch::channel(false);
        let (lifeline_tx, lifeline_rx) = tokio::sync::watch::channel(());
        let (write_started_tx, write_started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run_outbound_io(
            BlockingWriteIo {
                write_started: Some(write_started_tx),
            },
            requests_rx,
            cancel_rx,
            runtime_shutdown_rx,
            lifeline_rx,
        ));
        let send = tokio::spawn(async move { sender.send("blocked", 2, vec![2; 32]).await });
        write_started_rx.await.unwrap();

        drop(lifeline_tx);
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("lifeline closes blocked outbound write")
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), send)
                .await
                .expect("blocked send resolves")
                .unwrap(),
            Err(SendMessageError::Closed)
        ));
    }

    #[tokio::test]
    async fn idle_streams_do_not_starve_an_active_sender_of_app_queue_capacity() {
        let router = MessageChannelRouter::new(owner());
        let mut receiver = router
            .register(resource(owner(), "shared"), CHANNEL_CAPACITY)
            .unwrap();
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let mut idle_tasks = Vec::new();
        let mut idle_outputs = Vec::new();
        let mut revocations = Vec::new();
        let shared_frame_memory = frame_memory();

        for _ in 0..CHANNEL_CAPACITY {
            let mut input = Vec::new();
            write_open_frame(
                &mut input,
                owner(),
                "shared",
                &resource(owner(), "shared").clock,
            )
            .await
            .unwrap();
            let output = Arc::new(Mutex::new(Vec::new()));
            let io = InstrumentedIo {
                input: Cursor::new(input),
                bytes_read: Arc::new(AtomicUsize::new(0)),
                output: output.clone(),
                pending_at_eof: true,
            };
            let (revocation_tx, revocation_rx) = tokio::sync::watch::channel(false);
            revocations.push(revocation_tx);
            idle_outputs.push(output);
            idle_tasks.push(tokio::spawn(handle_inbound_io(
                sender(),
                io,
                router.clone(),
                shared_frame_memory.clone(),
                shutdown_rx.clone(),
                revocation_rx,
            )));
        }

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if idle_outputs
                    .iter()
                    .all(|output| output.lock().unwrap().len() >= 6)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all idle streams are accepted");
        for _ in 0..CHANNEL_CAPACITY {
            tokio::task::yield_now().await;
        }

        let mut active_input = Vec::new();
        write_open_frame(
            &mut active_input,
            owner(),
            "shared",
            &resource(owner(), "shared").clock,
        )
        .await
        .unwrap();
        let message = Message {
            r#type: "active".into(),
            timestamp_ns: 9,
            payload: vec![9, 8, 7],
        };
        write_message_frame(&mut active_input, 1, &message)
            .await
            .unwrap();
        let (_active_revocation_tx, active_revocation_rx) = tokio::sync::watch::channel(false);
        let active_task = tokio::spawn(handle_inbound_io(
            sender(),
            InstrumentedIo {
                input: Cursor::new(active_input),
                bytes_read: Arc::new(AtomicUsize::new(0)),
                output: Arc::new(Mutex::new(Vec::new())),
                pending_at_eof: true,
            },
            router,
            shared_frame_memory,
            shutdown_rx,
            active_revocation_rx,
        ));

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("idle streams must not reserve application queue slots")
            .unwrap();
        assert_eq!(event.message, message);

        drop(receiver);
        for task in idle_tasks {
            tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(1), active_task)
            .await
            .unwrap()
            .unwrap();
        drop(revocations);
    }
}
