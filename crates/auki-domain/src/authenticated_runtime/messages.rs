use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use auki_datatypes::message::Message;
use auki_network::{
    message_codec::{
        MAX_MESSAGE_FRAME_BYTES, MessageProtocolError, decode_message_frame, read_ack_frame,
        read_frame_body, read_frame_length, read_open_response, write_ack_frame,
        write_message_frame, write_open_frame, write_open_response,
    },
    protocol_ids::MESSAGE_V0_1_0,
    resources_v3_protocol::{MessageChannelResource, ResourcesProtocolError},
};
use auki_p2p::PeerId;
use futures::{AsyncRead, AsyncWrite};
use parking_lot::Mutex;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    io_tasks::{DomainIoTaskError, DomainIoTaskLease, DomainIoTasks},
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
    resources_v3::MessageChannelCatalogProvider,
};

const MESSAGE_MAX_CONCURRENCY: usize = 128;
const MESSAGE_MAX_CONCURRENCY_PER_PEER: usize = 8;
const OUTBOUND_QUEUE_CAPACITY: usize = 16;
const MAX_INBOUND_MESSAGE_FRAME_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MESSAGE_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// One live inbound event and the identity authenticated by `auki-p2p`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InboundMessage {
    pub(crate) sender: PeerId,
    pub(crate) message: Message,
}

/// The Domain-owned implementation of authenticated message channels 0.1.0.
#[derive(Clone)]
pub(crate) struct MessagesV1 {
    local_peer_id: PeerId,
    protocols: DomainProtocols,
    lifecycle: CancellationToken,
    router: MessageChannelRouter,
    frame_memory: MessageFrameMemoryBudget,
    per_peer_handlers: PerPeerHandlerBudget,
    io_tasks: DomainIoTasks,
}

impl MessagesV1 {
    pub(super) fn new(
        local_peer_id: PeerId,
        protocols: DomainProtocols,
        lifecycle: CancellationToken,
        io_tasks: DomainIoTasks,
    ) -> Self {
        Self {
            local_peer_id,
            protocols,
            router: MessageChannelRouter::new(local_peer_id, lifecycle.clone()),
            lifecycle,
            frame_memory: MessageFrameMemoryBudget::new(),
            per_peer_handlers: PerPeerHandlerBudget::default(),
            io_tasks,
        }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, MessagesV1Error> {
        let spec = DomainProtocolSpec::new(
            MESSAGE_V0_1_0,
            MESSAGE_MAX_CONCURRENCY,
            MAX_MESSAGE_FRAME_BYTES,
        )?;
        let messages = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let messages = messages.clone();
                async move {
                    if let Err(error) = messages.handle(stream).await
                        && !error.is_normal_stream_end()
                    {
                        tracing::warn!(%error, "authenticated message channel failed");
                    }
                }
            })
            .map_err(MessagesV1Error::Protocol)
    }

    /// Declare one receiver-owned channel and allocate its bounded live queue.
    pub(crate) fn declare(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelRegistration, MessageChannelRegistrationError> {
        self.ensure_running()
            .map_err(|_| MessageChannelRegistrationError::Stopped)?;
        self.router.register(resource, receiver_capacity)
    }

    /// Current receiver-owned rows for the authenticated resource catalog.
    pub(crate) fn catalog(&self) -> Vec<MessageChannelResource> {
        if self.lifecycle.is_cancelled() {
            Vec::new()
        } else {
            self.router.catalog()
        }
    }

    /// Open one exact receiver-owned channel after mutual Domain authentication.
    pub(crate) async fn open(
        &self,
        expected_peer: PeerId,
        resource: &MessageChannelResource,
    ) -> Result<MessageChannelSender, OpenMessageChannelError> {
        self.ensure_running()
            .map_err(|_| OpenMessageChannelError::Stopped)?;
        resource
            .validate()
            .map_err(OpenMessageChannelError::InvalidResource)?;
        if resource.owner_peer_id != expected_peer {
            return Err(OpenMessageChannelError::OwnerMismatch {
                expected_peer: Box::new(expected_peer),
                resource_owner: Box::new(resource.owner_peer_id),
            });
        }

        // `DomainProtocols::open` completes expected-Peer-ID and exact-Domain
        // mutual authentication. No application byte is written before it
        // returns an authenticated stream.
        let open = tokio::time::timeout(
            MESSAGE_OPEN_TIMEOUT,
            self.protocols.open(expected_peer, MESSAGE_V0_1_0),
        );
        let mut stream = tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => return Err(OpenMessageChannelError::Stopped),
            result = open => match result {
                Err(_) => return Err(OpenMessageChannelError::Timeout(MESSAGE_OPEN_TIMEOUT)),
                Ok(Err(error)) => return Err(OpenMessageChannelError::Protocol(error)),
                Ok(Ok(stream)) => stream,
            },
        };

        let write_open = tokio::time::timeout(
            MESSAGE_OPEN_TIMEOUT,
            write_open_frame(
                &mut stream,
                resource.owner_peer_id,
                &resource.resource_id,
                &resource.clock,
            ),
        );
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => return Err(OpenMessageChannelError::Stopped),
            result = write_open => match result {
                Err(_) => return Err(OpenMessageChannelError::Timeout(MESSAGE_OPEN_TIMEOUT)),
                Ok(Err(error)) => return Err(OpenMessageChannelError::Codec(error)),
                Ok(Ok(())) => {}
            },
        }
        let response = tokio::time::timeout(MESSAGE_OPEN_TIMEOUT, read_open_response(&mut stream));
        let accepted = tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => return Err(OpenMessageChannelError::Stopped),
            result = response => match result {
                Err(_) => return Err(OpenMessageChannelError::Timeout(MESSAGE_OPEN_TIMEOUT)),
                Ok(Err(error)) => return Err(OpenMessageChannelError::Codec(error)),
                Ok(Ok(accepted)) => accepted,
            },
        };
        if !accepted {
            return Err(OpenMessageChannelError::Rejected {
                owner_peer_id: Box::new(resource.owner_peer_id),
                resource_id: resource.resource_id.clone(),
            });
        }

        self.ensure_running()
            .map_err(|_| OpenMessageChannelError::Stopped)?;
        let (requests, receiver) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_lifecycle = self.lifecycle.clone();
        let lease = self
            .io_tasks
            .spawn(async move {
                run_outbound_io(stream, receiver, task_cancel, task_lifecycle).await;
            })
            .map_err(OpenMessageChannelError::from_io_task)?;
        if self.lifecycle.is_cancelled() {
            cancel.cancel();
            drop(lease);
            return Err(OpenMessageChannelError::Stopped);
        }

        Ok(MessageChannelSender {
            inner: Arc::new(MessageChannelSenderInner {
                requests,
                cancel,
                _task: lease,
            }),
        })
    }

    /// Stop accepting declarations and drain every receiver endpoint.
    ///
    /// Domain I/O task cleanup owns the matching outbound task join barrier.
    pub(crate) fn shutdown(&self) {
        self.router.shutdown();
    }

    async fn handle(&self, stream: DomainProtocolStream) -> Result<(), MessagesV1Error> {
        self.ensure_running()?;
        let sender_peer_id = stream.remote_peer().peer_id;
        // Preserve the legacy eight-stream per-peer limit. This permit is taken
        // after mutual authentication but before the first application read.
        let _peer_permit = self.per_peer_handlers.try_acquire(sender_peer_id).ok_or(
            MessagesV1Error::PeerStreamLimit {
                peer_id: Box::new(sender_peer_id),
                maximum: MESSAGE_MAX_CONCURRENCY_PER_PEER,
            },
        )?;
        handle_inbound_io(
            sender_peer_id,
            stream,
            self.router.clone(),
            self.frame_memory.clone(),
            self.lifecycle.clone(),
        )
        .await
    }

    fn ensure_running(&self) -> Result<(), MessagesV1Error> {
        if self.lifecycle.is_cancelled() {
            Err(MessagesV1Error::Stopped)
        } else {
            Ok(())
        }
    }
}

impl MessageChannelCatalogProvider for MessagesV1 {
    fn message_channel_catalog(&self) -> Vec<MessageChannelResource> {
        self.catalog()
    }
}

#[derive(Clone)]
struct MessageFrameMemoryBudget {
    permits: Arc<Semaphore>,
}

impl MessageFrameMemoryBudget {
    fn new() -> Self {
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

#[derive(Clone, Default)]
struct PerPeerHandlerBudget {
    active: Arc<Mutex<HashMap<PeerId, usize>>>,
}

impl PerPeerHandlerBudget {
    fn try_acquire(&self, peer_id: PeerId) -> Option<PerPeerHandlerPermit> {
        let mut active = self.active.lock();
        let count = active.entry(peer_id).or_default();
        if *count == MESSAGE_MAX_CONCURRENCY_PER_PEER {
            return None;
        }
        *count += 1;
        Some(PerPeerHandlerPermit {
            active: Arc::clone(&self.active),
            peer_id,
        })
    }
}

struct PerPeerHandlerPermit {
    active: Arc<Mutex<HashMap<PeerId, usize>>>,
    peer_id: PeerId,
}

impl Drop for PerPeerHandlerPermit {
    fn drop(&mut self) {
        let mut active = self.active.lock();
        let Some(count) = active.get_mut(&self.peer_id) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            active.remove(&self.peer_id);
        }
    }
}

#[derive(Clone)]
struct MessageChannelRouter {
    inner: Arc<MessageChannelRouterInner>,
}

struct MessageChannelRouterInner {
    local_peer_id: PeerId,
    lifecycle: CancellationToken,
    closed: AtomicBool,
    channels: Mutex<HashMap<(PeerId, String), EndpointState>>,
}

struct EndpointState {
    sender: mpsc::Sender<InboundMessage>,
    cancel: CancellationToken,
    resource: MessageChannelResource,
}

impl MessageChannelRouter {
    fn new(local_peer_id: PeerId, lifecycle: CancellationToken) -> Self {
        Self {
            inner: Arc::new(MessageChannelRouterInner {
                local_peer_id,
                lifecycle,
                closed: AtomicBool::new(false),
                channels: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn register(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelRegistration, MessageChannelRegistrationError> {
        if receiver_capacity == 0 {
            return Err(MessageChannelRegistrationError::ZeroCapacity);
        }
        resource
            .validate()
            .map_err(MessageChannelRegistrationError::InvalidResource)?;
        if resource.owner_peer_id != self.inner.local_peer_id {
            return Err(MessageChannelRegistrationError::OwnerMismatch {
                expected: Box::new(self.inner.local_peer_id),
                actual: Box::new(resource.owner_peer_id),
            });
        }

        let key = (resource.owner_peer_id, resource.resource_id.clone());
        let mut channels = self.inner.channels.lock();
        if self.inner.closed.load(Ordering::SeqCst) || self.inner.lifecycle.is_cancelled() {
            return Err(MessageChannelRegistrationError::Stopped);
        }
        if channels.contains_key(&key) {
            return Err(MessageChannelRegistrationError::DuplicateChannel {
                owner_peer_id: Box::new(key.0),
                resource_id: key.1,
            });
        }

        let (sender, receiver) = mpsc::channel(receiver_capacity);
        let cancel = CancellationToken::new();
        channels.insert(
            key.clone(),
            EndpointState {
                sender,
                cancel,
                resource: resource.clone(),
            },
        );
        Ok(MessageChannelRegistration {
            resource,
            receiver,
            lifecycle: self.inner.lifecycle.clone(),
            router: self.clone(),
            key,
        })
    }

    fn catalog(&self) -> Vec<MessageChannelResource> {
        if self.inner.closed.load(Ordering::SeqCst) || self.inner.lifecycle.is_cancelled() {
            return Vec::new();
        }
        let mut resources = self
            .inner
            .channels
            .lock()
            .values()
            .map(|endpoint| endpoint.resource.clone())
            .collect::<Vec<_>>();
        resources.sort_unstable_by(|left, right| {
            left.owner_peer_id
                .to_string()
                .cmp(&right.owner_peer_id.to_string())
                .then_with(|| left.resource_id.cmp(&right.resource_id))
        });
        resources
    }

    fn lookup_exact(
        &self,
        owner_peer_id: PeerId,
        resource_id: &str,
        expected_clock: &auki_registry::RegistryRef,
    ) -> Option<MessageChannelEndpoint> {
        if self.inner.closed.load(Ordering::SeqCst) || self.inner.lifecycle.is_cancelled() {
            return None;
        }
        self.inner
            .channels
            .lock()
            .get(&(owner_peer_id, resource_id.to_owned()))
            .filter(|endpoint| endpoint.resource.clock == *expected_clock)
            .map(|endpoint| MessageChannelEndpoint {
                sender: endpoint.sender.clone(),
                cancel: endpoint.cancel.clone(),
            })
    }

    fn unregister(&self, key: &(PeerId, String)) {
        if let Some(endpoint) = self.inner.channels.lock().remove(key) {
            endpoint.cancel.cancel();
        }
    }

    fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let endpoints = self
            .inner
            .channels
            .lock()
            .drain()
            .map(|(_, endpoint)| endpoint)
            .collect::<Vec<_>>();
        for endpoint in endpoints {
            endpoint.cancel.cancel();
        }
    }
}

struct MessageChannelEndpoint {
    sender: mpsc::Sender<InboundMessage>,
    cancel: CancellationToken,
}

/// A receiver declaration whose lifetime is also the channel lifetime.
pub(crate) struct MessageChannelRegistration {
    resource: MessageChannelResource,
    receiver: mpsc::Receiver<InboundMessage>,
    lifecycle: CancellationToken,
    router: MessageChannelRouter,
    key: (PeerId, String),
}

impl MessageChannelRegistration {
    pub(crate) fn resource(&self) -> &MessageChannelResource {
        &self.resource
    }

    pub(crate) async fn recv(&mut self) -> Option<InboundMessage> {
        tokio::select! {
            biased;
            _ = self.lifecycle.cancelled() => None,
            message = self.receiver.recv() => message,
        }
    }
}

impl Drop for MessageChannelRegistration {
    fn drop(&mut self) {
        self.router.unregister(&self.key);
    }
}

/// Cloneable sender for one persistent authenticated message substream.
#[derive(Clone)]
pub(crate) struct MessageChannelSender {
    inner: Arc<MessageChannelSenderInner>,
}

struct MessageChannelSenderInner {
    requests: mpsc::Sender<SendRequest>,
    cancel: CancellationToken,
    _task: DomainIoTaskLease,
}

impl Drop for MessageChannelSenderInner {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct SendRequest {
    message: Message,
    result: oneshot::Sender<Result<(), SendMessageError>>,
}

impl MessageChannelSender {
    /// Resolve after transport acceptance into the receiver's bounded queue.
    ///
    /// An error is indeterminate when the receiver enqueued the message but its
    /// ACK was lost. The SDK never retries or replays a send.
    pub(crate) async fn send(
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

async fn run_outbound_io<S>(
    mut stream: S,
    mut requests: mpsc::Receiver<SendRequest>,
    cancel: CancellationToken,
    lifecycle: CancellationToken,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut sequence = 1_u64;
    loop {
        let request = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return,
            _ = cancel.cancelled() => return,
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
            _ = lifecycle.cancelled() => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = cancel.cancelled() => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = result.closed() => return,
            write = write_message_frame(&mut stream, sequence, &message) => write,
        };
        if let Err(error) = write {
            let _ = result.send(Err(SendMessageError::Codec(error)));
            return;
        }

        let ack = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = cancel.cancelled() => {
                let _ = result.send(Err(SendMessageError::Closed));
                return;
            }
            _ = result.closed() => return,
            ack = read_ack_frame(&mut stream) => ack,
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
                let _ = result.send(Err(SendMessageError::Codec(error)));
                return;
            }
        }
        sequence = sequence.wrapping_add(1).max(1);
    }
}

async fn handle_inbound_io<S>(
    sender_peer_id: PeerId,
    mut stream: S,
    router: MessageChannelRouter,
    frame_memory: MessageFrameMemoryBudget,
    lifecycle: CancellationToken,
) -> Result<(), MessagesV1Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let open_frame_len = tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
        result = read_frame_length(&mut stream) => result?,
    };
    let open_frame_memory = tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
        result = frame_memory.reserve(open_frame_len) => {
            result.map_err(|_| MessagesV1Error::Stopped)?
        },
    };
    let open_payload = tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
        result = read_frame_body(&mut stream, open_frame_len) => result?,
    };
    let (owner_peer_id, resource_id, expected_clock) =
        auki_network::message_codec::decode_open_frame(&open_payload)?;
    drop(open_frame_memory);

    let Some(endpoint) = router.lookup_exact(owner_peer_id, &resource_id, &expected_clock) else {
        tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            result = write_open_response(&mut stream, false) => result?,
        }
        return Ok(());
    };
    tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
        _ = endpoint.cancel.cancelled() => return Ok(()),
        result = write_open_response(&mut stream, true) => result?,
    }

    loop {
        // An idle channel retains no application-queue or body-memory permit.
        let frame_len = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            result = read_frame_length(&mut stream) => result?,
        };
        let frame_memory_permit = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            result = frame_memory.reserve(frame_len) => {
                result.map_err(|_| MessagesV1Error::Stopped)?
            },
        };
        let payload = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            result = read_frame_body(&mut stream, frame_len) => result?,
        };
        let (sequence, message) = decode_message_frame(&payload)?;

        // Reserve application capacity only after one complete validated frame.
        // Do not read another frame while this one waits for a slow consumer.
        let app_sender = endpoint.sender.clone();
        let permit = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            result = app_sender.reserve_owned() => match result {
                Ok(permit) => permit,
                Err(_) => return Ok(()),
            },
        };
        tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            _ = tokio::task::yield_now() => {
                permit.send(InboundMessage {
                    sender: sender_peer_id,
                    message,
                });
            }
        }
        drop(frame_memory_permit);

        // ACK means runtime queue acceptance, not application acceptance.
        tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(MessagesV1Error::Stopped),
            _ = endpoint.cancel.cancelled() => return Ok(()),
            result = write_ack_frame(&mut stream, sequence) => result?,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageChannelRegistrationError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("message channel receiver capacity must be greater than zero")]
    ZeroCapacity,
    #[error("message channel owner {actual} does not match local peer {expected}")]
    OwnerMismatch {
        expected: Box<PeerId>,
        actual: Box<PeerId>,
    },
    #[error("message channel already declared: {owner_peer_id}/{resource_id}")]
    DuplicateChannel {
        owner_peer_id: Box<PeerId>,
        resource_id: String,
    },
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] ResourcesProtocolError),
}

#[derive(Debug, thiserror::Error)]
pub enum OpenMessageChannelError {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("message channel owner {resource_owner} does not match expected peer {expected_peer}")]
    OwnerMismatch {
        expected_peer: Box<PeerId>,
        resource_owner: Box<PeerId>,
    },
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] ResourcesProtocolError),
    #[error("authenticated message protocol failed: {0}")]
    Protocol(#[source] DomainProtocolError),
    #[error("message codec failed: {0}")]
    Codec(#[source] MessageProtocolError),
    #[error("receiver rejected message channel {owner_peer_id}/{resource_id}")]
    Rejected {
        owner_peer_id: Box<PeerId>,
        resource_id: String,
    },
    #[error("message channel open exceeded {0:?}")]
    Timeout(Duration),
    #[error("the Domain protocol I/O task host is unavailable")]
    IoTask,
    #[error("the Domain reached its {maximum}-task protocol I/O limit")]
    IoTaskCapacityExceeded { maximum: usize },
}

impl OpenMessageChannelError {
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

#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error("message channel is closed")]
    Closed,
    #[error("message codec failed: {0}")]
    Codec(#[source] MessageProtocolError),
    #[error("receiver acked sequence {actual}, expected {expected}")]
    AckSequenceMismatch { expected: u64, actual: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum MessagesV1Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("authenticated message protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("message codec failed: {0}")]
    Codec(#[from] MessageProtocolError),
    #[error("authenticated peer {peer_id} reached its {maximum}-stream message limit")]
    PeerStreamLimit {
        peer_id: Box<PeerId>,
        maximum: usize,
    },
}

impl MessagesV1Error {
    fn is_normal_stream_end(&self) -> bool {
        matches!(
            self,
            Self::Codec(MessageProtocolError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use auki_registry::RegistryRef;
    use futures::{AsyncRead, AsyncWrite, io::Cursor};

    use super::*;

    fn owner() -> PeerId {
        "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw"
            .parse()
            .unwrap()
    }

    fn sender() -> PeerId {
        "12D3KooWQTx8hmK9nUAZdXZVL8mFpCZkVJCwRThtgmHXkRfGbvU4"
            .parse()
            .unwrap()
    }

    fn resource(owner_peer_id: PeerId, id: &str) -> MessageChannelResource {
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

    struct InstrumentedIo {
        input: Cursor<Vec<u8>>,
        output: Arc<Mutex<Vec<u8>>>,
        reads: Arc<AtomicUsize>,
        pending_at_eof: bool,
    }

    impl AsyncRead for InstrumentedIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            output: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.pending_at_eof && self.input.position() == self.input.get_ref().len() as u64 {
                return Poll::Pending;
            }
            let result = Pin::new(&mut self.input).poll_read(context, output);
            if let Poll::Ready(Ok(read)) = result {
                self.reads.fetch_add(read, Ordering::SeqCst);
            }
            result
        }
    }

    impl AsyncWrite for InstrumentedIo {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.output.lock().extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
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
                    incoming: Arc::clone(&receiver_to_sender),
                    outgoing: Arc::clone(&sender_to_receiver),
                    fail_writes_after: None,
                    bytes_written: 0,
                },
                Self {
                    incoming: sender_to_receiver,
                    outgoing: receiver_to_sender,
                    // The accepted open response is six bytes; fail the ACK.
                    fail_writes_after: Some(6),
                    bytes_written: 0,
                },
            )
        }
    }

    impl AsyncRead for LoopbackIo {
        fn poll_read(
            self: Pin<&mut Self>,
            context: &mut Context<'_>,
            output: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            let mut incoming = self.incoming.lock();
            if !incoming.bytes.is_empty() {
                let read = output.len().min(incoming.bytes.len());
                for slot in &mut output[..read] {
                    *slot = incoming.bytes.pop_front().unwrap();
                }
                return Poll::Ready(Ok(read));
            }
            if incoming.closed {
                return Poll::Ready(Ok(0));
            }
            incoming.reader = Some(context.waker().clone());
            Poll::Pending
        }
    }

    impl AsyncWrite for LoopbackIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self
                .fail_writes_after
                .is_some_and(|limit| self.bytes_written >= limit)
            {
                let mut outgoing = self.outgoing.lock();
                outgoing.closed = true;
                if let Some(reader) = outgoing.reader.take() {
                    reader.wake();
                }
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "scripted ACK loss",
                )));
            }
            let outgoing = Arc::clone(&self.outgoing);
            self.bytes_written += bytes.len();
            let mut outgoing = outgoing.lock();
            outgoing.bytes.extend(bytes.iter().copied());
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut outgoing = self.outgoing.lock();
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
            Poll::Ready(Ok(()))
        }
    }

    impl Drop for LoopbackIo {
        fn drop(&mut self) {
            let mut outgoing = self.outgoing.lock();
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
        }
    }

    #[test]
    fn per_peer_stream_limit_recovers_when_a_handler_finishes() {
        let budget = PerPeerHandlerBudget::default();
        let permits = (0..MESSAGE_MAX_CONCURRENCY_PER_PEER)
            .map(|_| budget.try_acquire(sender()).unwrap())
            .collect::<Vec<_>>();
        assert!(budget.try_acquire(sender()).is_none());
        assert!(budget.try_acquire(owner()).is_some());

        drop(permits);
        assert!(budget.try_acquire(sender()).is_some());
    }

    #[tokio::test]
    async fn declaration_lifetime_owns_catalog_queue_and_reregistration() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle);
        assert!(matches!(
            router.register(resource(sender(), "events"), 1),
            Err(MessageChannelRegistrationError::OwnerMismatch { .. })
        ));
        assert!(matches!(
            router.register(resource(owner(), "events"), 0),
            Err(MessageChannelRegistrationError::ZeroCapacity)
        ));

        let registration = router.register(resource(owner(), "events"), 1).unwrap();
        assert_eq!(router.catalog(), vec![registration.resource().clone()]);
        assert!(matches!(
            router.register(resource(owner(), "events"), 1),
            Err(MessageChannelRegistrationError::DuplicateChannel { .. })
        ));
        drop(registration);
        assert!(router.catalog().is_empty());
        assert!(router.register(resource(owner(), "events"), 1).is_ok());

        router.shutdown();
        assert!(router.catalog().is_empty());
        assert!(matches!(
            router.register(resource(owner(), "late"), 1),
            Err(MessageChannelRegistrationError::Stopped)
        ));
    }

    #[tokio::test]
    async fn fatal_lifecycle_cancellation_closes_a_retained_registration() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle.clone());
        let mut registration = router.register(resource(owner(), "events"), 1).unwrap();

        // Keep the router alive so its endpoint sender remains retained, exactly
        // as it does between fail-and-shutdown and explicit leave or Drop.
        lifecycle.cancel();

        assert!(
            tokio::time::timeout(Duration::from_millis(100), registration.recv())
                .await
                .expect("fatal cancellation must wake the receiver")
                .is_none()
        );
        assert!(router.catalog().is_empty());
    }

    #[test]
    fn catalog_order_is_deterministic() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle);
        let zulu = router.register(resource(owner(), "zulu"), 1).unwrap();
        let alpha = router.register(resource(owner(), "alpha"), 1).unwrap();

        assert_eq!(
            router
                .catalog()
                .into_iter()
                .map(|resource| resource.resource_id)
                .collect::<Vec<_>>(),
            ["alpha", "zulu"]
        );

        drop((alpha, zulu));
    }

    #[tokio::test]
    async fn stale_clock_is_rejected_before_payload_bytes_are_read() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle.clone());
        let mut current = resource(owner(), "events");
        current.clock.hash = "current-clock-hash".into();
        let mut registration = router.register(current.clone(), 1).unwrap();
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

        let reads = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(Vec::new()));
        handle_inbound_io(
            sender(),
            InstrumentedIo {
                input: Cursor::new(input),
                output: Arc::clone(&output),
                reads: Arc::clone(&reads),
                pending_at_eof: false,
            },
            router,
            MessageFrameMemoryBudget::new(),
            lifecycle,
        )
        .await
        .unwrap();

        assert_eq!(reads.load(Ordering::SeqCst), open_frame_bytes);
        let response_bytes = output.lock().clone();
        assert!(
            !read_open_response(&mut Cursor::new(response_bytes))
                .await
                .unwrap()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), registration.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn one_decoded_frame_waits_for_slow_consumer_without_reading_the_next() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle.clone());
        let mut registration = router.register(resource(owner(), "bounded"), 1).unwrap();
        let mut input = Vec::new();
        write_open_frame(
            &mut input,
            owner(),
            "bounded",
            &resource(owner(), "bounded").clock,
        )
        .await
        .unwrap();
        for sequence in 1..=2 {
            write_message_frame(
                &mut input,
                sequence,
                &Message {
                    r#type: format!("message-{sequence}"),
                    timestamp_ns: sequence as i64,
                    payload: vec![sequence as u8; 32],
                },
            )
            .await
            .unwrap();
        }
        let bytes_before_third_frame = input.len();
        write_message_frame(
            &mut input,
            3,
            &Message {
                r#type: "must-not-be-read".into(),
                timestamp_ns: 3,
                payload: vec![3; 32],
            },
        )
        .await
        .unwrap();

        let reads = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(handle_inbound_io(
            sender(),
            InstrumentedIo {
                input: Cursor::new(input),
                output: Arc::clone(&output),
                reads: Arc::clone(&reads),
                pending_at_eof: true,
            },
            router,
            MessageFrameMemoryBudget::new(),
            lifecycle,
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            while output.lock().len() < 6 + 13 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("open and first message must be acknowledged");
        tokio::task::yield_now().await;
        assert_eq!(reads.load(Ordering::SeqCst), bytes_before_third_frame);

        assert_eq!(
            registration.recv().await.unwrap().message.r#type,
            "message-1"
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while output.lock().len() < 6 + 2 * 13 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("releasing capacity must acknowledge the second message");
        drop(registration);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("registration drop cancels the handler")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn ack_loss_after_enqueue_is_an_indeterminate_send_error() {
        let lifecycle = CancellationToken::new();
        let router = MessageChannelRouter::new(owner(), lifecycle.clone());
        let mut registration = router.register(resource(owner(), "ack-loss"), 1).unwrap();
        let (mut sender_io, receiver_io) = LoopbackIo::pair_with_receiver_ack_loss();
        let inbound = tokio::spawn(handle_inbound_io(
            sender(),
            receiver_io,
            router,
            MessageFrameMemoryBudget::new(),
            lifecycle.clone(),
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

        let (requests, receiver) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        let cancel = CancellationToken::new();
        let outbound = tokio::spawn(run_outbound_io(sender_io, receiver, cancel, lifecycle));
        let (result, response) = oneshot::channel();
        requests
            .send(SendRequest {
                message: Message {
                    r#type: "application.command.v1".into(),
                    timestamp_ns: 42,
                    payload: b"opaque".to_vec(),
                },
                result,
            })
            .await
            .unwrap();

        assert!(matches!(
            response.await.unwrap(),
            Err(SendMessageError::Codec(_))
        ));
        let delivered = registration.recv().await.unwrap();
        assert_eq!(delivered.sender, sender());
        assert_eq!(delivered.message.r#type, "application.command.v1");
        assert_eq!(delivered.message.payload, b"opaque");

        assert!(inbound.await.unwrap().unwrap_err().is_normal_stream_end());
        outbound.await.unwrap();
    }
}
