//! Portable [`auki_sdk::AukiPeer`] endpoint for persistent typed messages v1.
//!
//! Channel declarations, bounded live queues, authenticated peer identity, and
//! persistent stream lifecycle live here. The wire contract remains in
//! [`super::v1`]; authorization and message meaning remain application policy.

#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AukiProtocolStream, AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::{
    AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt, lock::Mutex as AsyncMutex, pin_mut,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::endpoint_support::{deadline_after, prefer_primary};

use super::{
    MessageChannelResource, MessageChannelResourceError,
    v1::{
        ID, MAX_MESSAGE_FRAME_BYTES, Message, MessageProtocolError, decode_message_frame,
        read_ack_frame, read_frame_body, read_frame_length, read_open_response, write_ack_frame,
        write_message_frame, write_open_frame, write_open_response,
    },
};

/// Maximum number of concurrently served message streams across all peers.
pub const MESSAGE_MAX_CONCURRENCY: usize = 128;

/// Maximum number of concurrently served message streams from one authenticated peer.
pub const MESSAGE_MAX_CONCURRENCY_PER_PEER: usize = 8;

/// Maximum number of admitted sends sharing one persistent outbound channel.
pub const OUTBOUND_QUEUE_CAPACITY: usize = 16;

/// Maximum aggregate memory reserved for fully framed inbound messages.
pub const MAX_INBOUND_MESSAGE_FRAME_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Fixed deadline for opening and negotiating one channel.
pub const MESSAGE_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Fixed deadline for an active message write/ack exchange.
pub const MESSAGE_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Fixed deadline for stream or registration cleanup.
pub const MESSAGE_CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the exact bounded typed-message v1 registration.
pub fn message_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, MESSAGE_MAX_CONCURRENCY, MAX_MESSAGE_FRAME_BYTES)
}

/// One live message and the complete identity authenticated by `auki-p2p`.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageEvent {
    /// Receiver-owned channel that accepted the message.
    pub channel: MessageChannelResource,
    /// Mutually authenticated sender, including DDS subject, scopes, and application metadata.
    pub sender: AuthenticatedPeer,
    /// Opaque application message.
    pub message: Message,
}

impl MessageEvent {
    /// Opaque application type.
    pub fn message_type(&self) -> &str {
        &self.message.r#type
    }

    /// Application timestamp in the channel's declared clock.
    pub fn timestamp_ns(&self) -> i64 {
        self.message.timestamp_ns
    }

    /// Opaque application payload.
    pub fn payload(&self) -> &[u8] {
        &self.message.payload
    }
}

/// Mounted typed-message service plus its outbound client.
pub struct MessageEndpoint {
    client: MessageClient,
    router: MessageChannelRouter,
    registration: Option<AukiProtocolRegistration>,
}

impl MessageEndpoint {
    /// Mount typed-message v1 on one running Auki peer.
    pub fn mount(protocols: AukiPeerProtocols) -> Result<Self, MessageEndpointError> {
        let router = MessageChannelRouter::new(protocols.peer_id());
        let inbound_router = router.clone();
        let frame_memory = MessageFrameMemoryBudget::new();
        let per_peer_handlers = PerPeerHandlerBudget::default();
        let registration = protocols.register(message_protocol_spec()?, move |mut stream| {
            let router = inbound_router.clone();
            let frame_memory = frame_memory.clone();
            let per_peer_handlers = per_peer_handlers.clone();
            async move {
                let sender = stream.remote_peer().clone();
                let serving = match per_peer_handlers.try_acquire(sender.peer_id) {
                    Some(_permit) => {
                        handle_inbound_io(sender, &mut stream, router, frame_memory).await
                    }
                    None => Err(MessageEndpointError::PeerStreamLimit {
                        peer_id: Box::new(sender.peer_id),
                        maximum: MESSAGE_MAX_CONCURRENCY_PER_PEER,
                    }),
                };
                let cleanup = close_inbound_stream(&mut stream).await;
                let _ = prefer_primary(serving, cleanup);
            }
        })?;

        Ok(Self {
            client: MessageClient::new(protocols),
            router,
            registration: Some(registration),
        })
    }

    /// Clone the outbound client without cloning inbound registration ownership.
    pub fn client(&self) -> MessageClient {
        self.client.clone()
    }

    /// Declare one receiver-owned live channel with a bounded application queue.
    pub fn declare(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelReceiver, MessageChannelRegistrationError> {
        self.router.register(resource, receiver_capacity)
    }

    /// Return a deterministic snapshot of all currently declared channels.
    pub fn catalog(&self) -> Vec<MessageChannelResource> {
        self.router.catalog()
    }

    /// Stop declarations, wake receivers, and await all admitted protocol handlers.
    pub async fn close(mut self) -> Result<(), MessageEndpointError> {
        self.router.shutdown();
        let Some(registration) = self.registration.take() else {
            return Ok(());
        };
        deadline(
            MessageOperation::Close,
            MESSAGE_CLOSE_TIMEOUT,
            registration.close(),
        )
        .await?
        .map_err(MessageEndpointError::Sdk)
    }
}

impl Drop for MessageEndpoint {
    fn drop(&mut self) {
        self.router.shutdown();
    }
}

/// Cloneable outbound typed-message client.
#[derive(Clone)]
pub struct MessageClient {
    protocols: AukiPeerProtocols,
}

impl MessageClient {
    /// Construct a client over one running peer's protocol surface.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Open a persistent channel using routes configured on the owning native peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn open(
        &self,
        expected_peer: PeerId,
        resource: &MessageChannelResource,
    ) -> Result<MessageChannelSender, OpenMessageChannelError> {
        validate_open_resource(expected_peer, resource)?;
        open_channel(resource.clone(), self.protocols.open(expected_peer, ID)).await
    }

    /// Open a persistent channel through one exact advertised route.
    pub async fn open_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        resource: &MessageChannelResource,
    ) -> Result<MessageChannelSender, OpenMessageChannelError> {
        validate_open_resource(expected_peer, resource)?;
        open_channel(
            resource.clone(),
            self.protocols.open_exact(expected_peer, route, ID),
        )
        .await
    }
}

async fn open_channel<F>(
    resource: MessageChannelResource,
    opening: F,
) -> Result<MessageChannelSender, OpenMessageChannelError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(MessageOperation::Open, MESSAGE_OPEN_TIMEOUT, opening)
        .await
        .map_err(OpenMessageChannelError::from_timeout)?
        .map_err(OpenMessageChannelError::Sdk)?;
    let remote_peer = stream.remote_peer().clone();
    let relayed = stream.is_relayed();

    let negotiation = deadline(MessageOperation::Negotiate, MESSAGE_OPEN_TIMEOUT, async {
        write_open_frame(
            &mut stream,
            resource.owner_peer_id,
            &resource.resource_id,
            &resource.clock,
        )
        .await
        .map_err(OpenMessageChannelError::Codec)?;
        if read_open_response(&mut stream)
            .await
            .map_err(OpenMessageChannelError::Codec)?
        {
            Ok(())
        } else {
            Err(OpenMessageChannelError::Rejected {
                owner_peer_id: Box::new(resource.owner_peer_id),
                resource_id: resource.resource_id.clone(),
            })
        }
    })
    .await
    .map_err(OpenMessageChannelError::from_timeout)
    .and_then(|result| result);

    if let Err(error) = negotiation {
        let cleanup = close_route_stream(stream)
            .await
            .map_err(OpenMessageChannelError::from_close);
        return prefer_primary(Err(error), cleanup);
    }

    Ok(MessageChannelSender::new(
        stream,
        remote_peer,
        resource,
        relayed,
    ))
}

fn validate_open_resource(
    expected_peer: PeerId,
    resource: &MessageChannelResource,
) -> Result<(), OpenMessageChannelError> {
    resource
        .validate()
        .map_err(OpenMessageChannelError::InvalidResource)?;
    if resource.owner_peer_id == expected_peer {
        Ok(())
    } else {
        Err(OpenMessageChannelError::OwnerMismatch {
            expected_peer: Box::new(expected_peer),
            resource_owner: Box::new(resource.owner_peer_id),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
type SharedSenderInner = Arc<MessageChannelSenderInner>;
#[cfg(target_arch = "wasm32")]
type SharedSenderInner = Rc<MessageChannelSenderInner>;

/// Cloneable sender for one persistent authenticated message channel.
#[derive(Clone)]
pub struct MessageChannelSender {
    inner: SharedSenderInner,
}

impl MessageChannelSender {
    fn new(
        stream: AuthenticatedRouteStream,
        remote_peer: AuthenticatedPeer,
        resource: MessageChannelResource,
        relayed: bool,
    ) -> Self {
        Self {
            inner: SharedSenderInner::new(MessageChannelSenderInner {
                remote_peer,
                resource,
                relayed,
                closed: AtomicBool::new(false),
                admissions: Arc::new(Semaphore::new(OUTBOUND_QUEUE_CAPACITY)),
                state: AsyncMutex::new(MessageChannelSenderState {
                    stream: Some(stream),
                    next_sequence: 1,
                }),
            }),
        }
    }

    /// Mutually authenticated receiver bound to this channel.
    pub fn remote_peer(&self) -> &AuthenticatedPeer {
        &self.inner.remote_peer
    }

    /// Exact receiver-owned channel bound by the open handshake.
    pub fn resource(&self) -> &MessageChannelResource {
        &self.inner.resource
    }

    /// Whether this channel owns an exact relay circuit.
    pub fn is_relayed(&self) -> bool {
        self.inner.relayed
    }

    /// Send one opaque message and wait for acceptance into the receiver's bounded queue.
    ///
    /// A codec, timeout, cancellation, or mismatched ACK closes this channel.
    /// An error is indeterminate when the receiver enqueued the message but its
    /// ACK was lost; this endpoint never retries or replays a send.
    pub async fn send(
        &self,
        r#type: impl Into<String>,
        timestamp_ns: i64,
        payload: impl Into<Vec<u8>>,
    ) -> Result<(), SendMessageError> {
        self.send_message(Message {
            r#type: r#type.into(),
            timestamp_ns,
            payload: payload.into(),
        })
        .await
    }

    /// Send one already-constructed message and wait for its exact sequence ACK.
    pub async fn send_message(&self, message: Message) -> Result<(), SendMessageError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(SendMessageError::Closed);
        }

        // The permit bounds both active and queued send futures. Waiting for a
        // permit retains no stream lock and performs no wire I/O.
        let _admission = self
            .inner
            .admissions
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SendMessageError::Closed)?;
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(SendMessageError::Closed);
        }

        let mut state = self.inner.state.lock().await;
        if self.inner.closed.load(Ordering::SeqCst) || state.stream.is_none() {
            return Err(SendMessageError::Closed);
        }
        let sequence = state.next_sequence;
        let mut transaction = SendTransaction::new(&mut state, &self.inner.closed);
        let exchange = deadline(
            MessageOperation::Send,
            MESSAGE_SEND_TIMEOUT,
            exchange_message(transaction.stream(), sequence, &message),
        )
        .await
        .map_err(SendMessageError::from_timeout)
        .and_then(|result| result);

        if exchange.is_ok() {
            transaction.commit();
        }
        exchange
    }

    /// Close this channel for every clone and await exact route cleanup.
    pub async fn close(self) -> Result<(), SendMessageError> {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.admissions.close();
        let mut state = deadline(
            MessageOperation::Close,
            MESSAGE_CLOSE_TIMEOUT,
            self.inner.state.lock(),
        )
        .await
        .map_err(SendMessageError::from_timeout)?;
        let stream = state.stream.take();
        drop(state);
        match stream {
            Some(stream) => close_route_stream(stream)
                .await
                .map_err(SendMessageError::from_close),
            None => Ok(()),
        }
    }
}

struct MessageChannelSenderInner {
    remote_peer: AuthenticatedPeer,
    resource: MessageChannelResource,
    relayed: bool,
    closed: AtomicBool,
    admissions: Arc<Semaphore>,
    state: AsyncMutex<MessageChannelSenderState>,
}

impl Drop for MessageChannelSenderInner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.admissions.close();
    }
}

struct MessageChannelSenderState {
    stream: Option<AuthenticatedRouteStream>,
    next_sequence: u64,
}

/// An armed transaction poisons and drops the stream unless the exact ACK commits it.
struct SendTransaction<'a> {
    state: &'a mut MessageChannelSenderState,
    closed: &'a AtomicBool,
    committed: bool,
}

impl<'a> SendTransaction<'a> {
    fn new(state: &'a mut MessageChannelSenderState, closed: &'a AtomicBool) -> Self {
        Self {
            state,
            closed,
            committed: false,
        }
    }

    fn stream(&mut self) -> &mut AuthenticatedRouteStream {
        self.state
            .stream
            .as_mut()
            .expect("a send transaction starts only with a live stream")
    }

    fn commit(mut self) {
        self.state.next_sequence = self.state.next_sequence.wrapping_add(1).max(1);
        self.committed = true;
    }
}

impl Drop for SendTransaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.closed.store(true, Ordering::SeqCst);
            drop(self.state.stream.take());
        }
    }
}

async fn exchange_message<S>(
    stream: &mut S,
    sequence: u64,
    message: &Message,
) -> Result<(), SendMessageError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_message_frame(stream, sequence, message)
        .await
        .map_err(SendMessageError::Codec)?;
    let actual = read_ack_frame(stream)
        .await
        .map_err(SendMessageError::Codec)?;
    if actual == sequence {
        Ok(())
    } else {
        Err(SendMessageError::AckSequenceMismatch {
            expected: sequence,
            actual,
        })
    }
}

/// Bounded receiver whose lifetime is also the declared channel lifetime.
pub struct MessageChannelReceiver {
    resource: MessageChannelResource,
    receiver: async_channel::Receiver<MessageEvent>,
    cancel: Cancellation,
    router: MessageChannelRouter,
    key: (PeerId, String),
}

impl MessageChannelReceiver {
    /// Exact resource row bound to this receiver.
    pub fn resource(&self) -> &MessageChannelResource {
        &self.resource
    }

    /// Receive the next live event, or `None` after undeclaration or endpoint shutdown.
    pub async fn recv(&mut self) -> Option<MessageEvent> {
        let cancelled = self.cancel.cancelled().fuse();
        let received = self.receiver.recv().fuse();
        pin_mut!(cancelled, received);
        futures::select_biased! {
            () = cancelled => None,
            result = received => result.ok(),
        }
    }
}

impl Drop for MessageChannelReceiver {
    fn drop(&mut self) {
        self.router.unregister(&self.key);
    }
}

#[derive(Clone)]
struct MessageChannelRouter {
    inner: Arc<MessageChannelRouterInner>,
}

struct MessageChannelRouterInner {
    local_peer_id: PeerId,
    closed: AtomicBool,
    channels: Mutex<HashMap<(PeerId, String), EndpointState>>,
}

struct EndpointState {
    sender: async_channel::Sender<MessageEvent>,
    cancel: Cancellation,
    resource: MessageChannelResource,
}

#[derive(Clone)]
struct MessageChannelTarget {
    sender: async_channel::Sender<MessageEvent>,
    cancel: Cancellation,
    resource: MessageChannelResource,
}

impl MessageChannelRouter {
    fn new(local_peer_id: PeerId) -> Self {
        Self {
            inner: Arc::new(MessageChannelRouterInner {
                local_peer_id,
                closed: AtomicBool::new(false),
                channels: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn register(
        &self,
        resource: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelReceiver, MessageChannelRegistrationError> {
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
        let mut channels = lock_unpoisoned(&self.inner.channels);
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(MessageChannelRegistrationError::Stopped);
        }
        if channels.contains_key(&key) {
            return Err(MessageChannelRegistrationError::DuplicateChannel {
                owner_peer_id: Box::new(key.0),
                resource_id: key.1,
            });
        }

        let (sender, receiver) = async_channel::bounded(receiver_capacity);
        let cancel = Cancellation::new();
        channels.insert(
            key.clone(),
            EndpointState {
                sender,
                cancel: cancel.clone(),
                resource: resource.clone(),
            },
        );
        Ok(MessageChannelReceiver {
            resource,
            receiver,
            cancel,
            router: self.clone(),
            key,
        })
    }

    fn catalog(&self) -> Vec<MessageChannelResource> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Vec::new();
        }
        let mut resources = lock_unpoisoned(&self.inner.channels)
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
    ) -> Option<MessageChannelTarget> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return None;
        }
        lock_unpoisoned(&self.inner.channels)
            .get(&(owner_peer_id, resource_id.to_owned()))
            .filter(|endpoint| endpoint.resource.clock == *expected_clock)
            .map(|endpoint| MessageChannelTarget {
                sender: endpoint.sender.clone(),
                cancel: endpoint.cancel.clone(),
                resource: endpoint.resource.clone(),
            })
    }

    fn unregister(&self, key: &(PeerId, String)) {
        if let Some(endpoint) = lock_unpoisoned(&self.inner.channels).remove(key) {
            endpoint.cancel.cancel();
            endpoint.sender.close();
        }
    }

    fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let endpoints = lock_unpoisoned(&self.inner.channels)
            .drain()
            .map(|(_, endpoint)| endpoint)
            .collect::<Vec<_>>();
        for endpoint in endpoints {
            endpoint.cancel.cancel();
            endpoint.sender.close();
        }
    }
}

#[derive(Clone)]
struct Cancellation {
    sender: async_channel::Sender<()>,
    receiver: async_channel::Receiver<()>,
}

impl Cancellation {
    fn new() -> Self {
        let (sender, receiver) = async_channel::bounded(1);
        Self { sender, receiver }
    }

    fn cancel(&self) {
        self.sender.close();
    }

    async fn cancelled(&self) {
        let _ = self.receiver.recv().await;
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
    ) -> Result<OwnedSemaphorePermit, MessageEndpointError> {
        self.permits
            .clone()
            .acquire_many_owned(validated_frame_len)
            .await
            .map_err(|_| MessageEndpointError::Stopped)
    }
}

#[derive(Clone, Default)]
struct PerPeerHandlerBudget {
    active: Arc<Mutex<HashMap<PeerId, usize>>>,
}

impl PerPeerHandlerBudget {
    fn try_acquire(&self, peer_id: PeerId) -> Option<PerPeerHandlerPermit> {
        let mut active = lock_unpoisoned(&self.active);
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
        let mut active = lock_unpoisoned(&self.active);
        let Some(count) = active.get_mut(&self.peer_id) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            active.remove(&self.peer_id);
        }
    }
}

async fn handle_inbound_io<S>(
    sender: AuthenticatedPeer,
    stream: &mut S,
    router: MessageChannelRouter,
    frame_memory: MessageFrameMemoryBudget,
) -> Result<(), MessageEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let open_frame_len = deadline(
        MessageOperation::Negotiate,
        MESSAGE_OPEN_TIMEOUT,
        read_frame_length(stream),
    )
    .await??;
    let open_frame_memory = frame_memory.reserve(open_frame_len).await?;
    let open_payload = deadline(
        MessageOperation::Negotiate,
        MESSAGE_OPEN_TIMEOUT,
        read_frame_body(stream, open_frame_len),
    )
    .await??;
    let (owner_peer_id, resource_id, expected_clock) = decode_message_open(&open_payload)?;
    drop(open_frame_memory);

    let Some(endpoint) = router.lookup_exact(owner_peer_id, &resource_id, &expected_clock) else {
        deadline(
            MessageOperation::Negotiate,
            MESSAGE_OPEN_TIMEOUT,
            write_open_response(stream, false),
        )
        .await??;
        return Ok(());
    };

    match cancellable(
        &endpoint.cancel,
        deadline(
            MessageOperation::Negotiate,
            MESSAGE_OPEN_TIMEOUT,
            write_open_response(stream, true),
        ),
    )
    .await
    {
        Ok(result) => result??,
        Err(()) => return Ok(()),
    }

    loop {
        // An idle channel retains no body-memory or application-queue permit.
        let frame_len = match cancellable(&endpoint.cancel, read_frame_length(stream)).await {
            Ok(result) => result?,
            Err(()) => return Ok(()),
        };
        let frame_memory_permit =
            match cancellable(&endpoint.cancel, frame_memory.reserve(frame_len)).await {
                Ok(result) => result?,
                Err(()) => return Ok(()),
            };
        let payload = match cancellable(
            &endpoint.cancel,
            deadline(
                MessageOperation::ReceiveBody,
                MESSAGE_SEND_TIMEOUT,
                read_frame_body(stream, frame_len),
            ),
        )
        .await
        {
            Ok(result) => result??,
            Err(()) => return Ok(()),
        };
        let (sequence, message) = decode_message_frame(&payload)?;

        // Do not read another frame while this validated frame waits for a slow consumer.
        let event = MessageEvent {
            channel: endpoint.resource.clone(),
            sender: sender.clone(),
            message,
        };
        match cancellable(&endpoint.cancel, endpoint.sender.send(event)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(()) => return Ok(()),
        }
        drop(frame_memory_permit);

        // ACK means acceptance into the bounded runtime queue, not application consumption.
        match cancellable(
            &endpoint.cancel,
            deadline(
                MessageOperation::Acknowledge,
                MESSAGE_SEND_TIMEOUT,
                write_ack_frame(stream, sequence),
            ),
        )
        .await
        {
            Ok(result) => result??,
            Err(()) => return Ok(()),
        }
    }
}

fn decode_message_open(
    payload: &[u8],
) -> Result<(PeerId, String, auki_registry::RegistryRef), MessageProtocolError> {
    super::v1::decode_open_frame(payload)
}

async fn cancellable<T>(cancel: &Cancellation, future: impl Future<Output = T>) -> Result<T, ()> {
    let cancelled = cancel.cancelled().fuse();
    let work = future.fuse();
    pin_mut!(cancelled, work);
    futures::select_biased! {
        () = cancelled => Err(()),
        result = work => Ok(result),
    }
}

async fn close_inbound_stream(stream: &mut AukiProtocolStream) -> Result<(), MessageEndpointError> {
    deadline(
        MessageOperation::Close,
        MESSAGE_CLOSE_TIMEOUT,
        AsyncWriteExt::close(stream),
    )
    .await?
    .map_err(|error| MessageEndpointError::Close(error.to_string()))
}

async fn close_route_stream(stream: AuthenticatedRouteStream) -> Result<(), MessageEndpointError> {
    deadline(
        MessageOperation::Close,
        MESSAGE_CLOSE_TIMEOUT,
        stream.close(),
    )
    .await?
    .map_err(|error| MessageEndpointError::Close(error.to_string()))
}

async fn deadline<T>(
    operation: MessageOperation,
    timeout: Duration,
    future: impl Future<Output = T>,
) -> Result<T, MessageEndpointError> {
    deadline_after(timeout, future, || MessageEndpointError::Timeout(operation)).await
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One bounded typed-message endpoint operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageOperation {
    /// Open and authenticate a protocol stream.
    Open,
    /// Exchange the addressed channel open request and response.
    Negotiate,
    /// Write a message and await its exact acknowledgement.
    Send,
    /// Read a declared message frame body after its bounded length prefix.
    ReceiveBody,
    /// Write acceptance of one message into the bounded receiver queue.
    Acknowledge,
    /// Close a stream or mounted registration.
    Close,
}

impl fmt::Display for MessageOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Negotiate => "negotiate",
            Self::Send => "send",
            Self::ReceiveBody => "receive body",
            Self::Acknowledge => "acknowledge",
            Self::Close => "close",
        })
    }
}

/// Failure from the mounted typed-message endpoint.
#[derive(Debug, thiserror::Error)]
pub enum MessageEndpointError {
    /// The SDK rejected protocol registration or cleanup.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// The typed-message wire conversation failed.
    #[error("message codec failed: {0}")]
    Codec(#[from] MessageProtocolError),
    /// The endpoint has stopped.
    #[error("the message endpoint is stopped")]
    Stopped,
    /// One authenticated peer exceeded its independent inbound stream budget.
    #[error("authenticated peer {peer_id} reached its {maximum}-stream message limit")]
    PeerStreamLimit {
        /// Authenticated peer whose streams reached the bound.
        peer_id: Box<PeerId>,
        /// Fixed per-peer maximum.
        maximum: usize,
    },
    /// One endpoint phase exceeded its fixed deadline.
    #[error("message {0} timed out")]
    Timeout(MessageOperation),
    /// Stream cleanup failed.
    #[error("close message stream: {0}")]
    Close(String),
}

/// Failure to declare one bounded receiver-owned channel.
#[derive(Debug, thiserror::Error)]
pub enum MessageChannelRegistrationError {
    /// The endpoint no longer accepts declarations.
    #[error("the message endpoint is stopped")]
    Stopped,
    /// A bounded channel cannot have zero capacity.
    #[error("message channel receiver capacity must be greater than zero")]
    ZeroCapacity,
    /// The declared owner must be the local Auki peer.
    #[error("message channel owner {actual} does not match local peer {expected}")]
    OwnerMismatch {
        /// Local peer required by this endpoint.
        expected: Box<PeerId>,
        /// Owner encoded by the supplied resource.
        actual: Box<PeerId>,
    },
    /// A live declaration already owns this exact identity.
    #[error("message channel already declared: {owner_peer_id}/{resource_id}")]
    DuplicateChannel {
        /// Receiver owner.
        owner_peer_id: Box<PeerId>,
        /// Receiver-scoped resource identifier.
        resource_id: String,
    },
    /// The supplied catalog row is malformed.
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] MessageChannelResourceError),
}

/// Failure to open one exact persistent receiver-owned channel.
#[derive(Debug, thiserror::Error)]
pub enum OpenMessageChannelError {
    /// The catalog row is malformed.
    #[error("invalid message channel resource: {0}")]
    InvalidResource(#[source] MessageChannelResourceError),
    /// The selected catalog owner differs from the expected authenticated peer.
    #[error("message channel owner {resource_owner} does not match expected peer {expected_peer}")]
    OwnerMismatch {
        /// Peer selected by the caller.
        expected_peer: Box<PeerId>,
        /// Owner encoded by the catalog row.
        resource_owner: Box<PeerId>,
    },
    /// The SDK could not open or authenticate the stream.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[source] AukiProtocolError),
    /// The typed-message wire conversation failed.
    #[error("message codec failed: {0}")]
    Codec(#[source] MessageProtocolError),
    /// The receiver rejected this exact row, including a stale clock reference.
    #[error("receiver rejected message channel {owner_peer_id}/{resource_id}")]
    Rejected {
        /// Receiver owner.
        owner_peer_id: Box<PeerId>,
        /// Receiver-scoped resource identifier.
        resource_id: String,
    },
    /// One open phase exceeded its fixed deadline.
    #[error("message channel {0} timed out")]
    Timeout(MessageOperation),
    /// Cleanup after a failed open also failed.
    #[error("close message channel: {0}")]
    Close(String),
}

impl OpenMessageChannelError {
    fn from_timeout(error: MessageEndpointError) -> Self {
        match error {
            MessageEndpointError::Timeout(operation) => Self::Timeout(operation),
            error => Self::Close(error.to_string()),
        }
    }

    fn from_close(error: MessageEndpointError) -> Self {
        match error {
            MessageEndpointError::Timeout(operation) => Self::Timeout(operation),
            MessageEndpointError::Close(reason) => Self::Close(reason),
            error => Self::Close(error.to_string()),
        }
    }
}

/// Failure to send or close one persistent channel.
#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    /// The channel is closed or was poisoned by a canceled/failed exchange.
    #[error("message channel is closed")]
    Closed,
    /// The typed-message wire conversation failed.
    #[error("message codec failed: {0}")]
    Codec(#[source] MessageProtocolError),
    /// The receiver acknowledged a different message sequence.
    #[error("receiver acked sequence {actual}, expected {expected}")]
    AckSequenceMismatch {
        /// Sequence written by this sender.
        expected: u64,
        /// Sequence returned by the receiver.
        actual: u64,
    },
    /// One active send or close phase exceeded its fixed deadline.
    #[error("message channel {0} timed out")]
    Timeout(MessageOperation),
    /// Exact route cleanup failed.
    #[error("close message channel: {0}")]
    Close(String),
}

impl SendMessageError {
    fn from_timeout(error: MessageEndpointError) -> Self {
        match error {
            MessageEndpointError::Timeout(operation) => Self::Timeout(operation),
            error => Self::Close(error.to_string()),
        }
    }

    fn from_close(error: MessageEndpointError) -> Self {
        match error {
            MessageEndpointError::Timeout(operation) => Self::Timeout(operation),
            MessageEndpointError::Close(reason) => Self::Close(reason),
            error => Self::Close(error.to_string()),
        }
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
    use futures::io::Cursor;

    use super::*;

    fn owner() -> PeerId {
        "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw"
            .parse()
            .unwrap()
    }

    fn sender_id() -> PeerId {
        "12D3KooWQTx8hmK9nUAZdXZVL8mFpCZkVJCwRThtgmHXkRfGbvU4"
            .parse()
            .unwrap()
    }

    fn sender() -> AuthenticatedPeer {
        AuthenticatedPeer {
            peer_id: sender_id(),
            subject: "b03a67cb-45d4-4f60-a8b8-d9687e91d018".parse().unwrap(),
            peer_type: Some("robot".into()),
            domain_ids: vec!["4e990513-b110-467b-84ca-09a42d786f6d".parse().unwrap()],
            scopes: vec!["messages:send".into()],
            application: None,
            verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
        }
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
            lock_unpoisoned(&self.output).extend_from_slice(bytes);
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
                    // The accepted open response is six bytes; fail the following ACK.
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
            let mut incoming = lock_unpoisoned(&self.incoming);
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
                let mut outgoing = lock_unpoisoned(&self.outgoing);
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
            let mut outgoing = lock_unpoisoned(&outgoing);
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
            let mut outgoing = lock_unpoisoned(&self.outgoing);
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
            Poll::Ready(Ok(()))
        }
    }

    impl Drop for LoopbackIo {
        fn drop(&mut self) {
            let mut outgoing = lock_unpoisoned(&self.outgoing);
            outgoing.closed = true;
            if let Some(reader) = outgoing.reader.take() {
                reader.wake();
            }
        }
    }

    #[test]
    fn spec_mounts_the_exact_bounded_wire_contract() {
        let spec = message_protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ID);
        assert_eq!(spec.max_concurrency(), MESSAGE_MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_MESSAGE_FRAME_BYTES);
    }

    #[test]
    fn per_peer_stream_limit_recovers_when_a_handler_finishes() {
        let budget = PerPeerHandlerBudget::default();
        let permits = (0..MESSAGE_MAX_CONCURRENCY_PER_PEER)
            .map(|_| budget.try_acquire(sender_id()).unwrap())
            .collect::<Vec<_>>();
        assert!(budget.try_acquire(sender_id()).is_none());
        assert!(budget.try_acquire(owner()).is_some());

        drop(permits);
        assert!(budget.try_acquire(sender_id()).is_some());
    }

    #[tokio::test]
    async fn declaration_lifetime_owns_catalog_queue_and_reregistration() {
        let router = MessageChannelRouter::new(owner());
        assert!(matches!(
            router.register(resource(sender_id(), "events"), 1),
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
    async fn endpoint_shutdown_wakes_a_retained_receiver() {
        let router = MessageChannelRouter::new(owner());
        let mut registration = router.register(resource(owner(), "events"), 1).unwrap();

        router.shutdown();

        assert!(
            tokio::time::timeout(Duration::from_millis(100), registration.recv())
                .await
                .expect("shutdown must wake the receiver")
                .is_none()
        );
    }

    #[test]
    fn catalog_order_is_deterministic() {
        let router = MessageChannelRouter::new(owner());
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
        let router = MessageChannelRouter::new(owner());
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
            &mut InstrumentedIo {
                input: Cursor::new(input),
                output: Arc::clone(&output),
                reads: Arc::clone(&reads),
                pending_at_eof: false,
            },
            router,
            MessageFrameMemoryBudget::new(),
        )
        .await
        .unwrap();

        assert_eq!(reads.load(Ordering::SeqCst), open_frame_bytes);
        let response_bytes = lock_unpoisoned(&output).clone();
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
        let router = MessageChannelRouter::new(owner());
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
        let task = tokio::spawn({
            let reads = Arc::clone(&reads);
            let output = Arc::clone(&output);
            async move {
                handle_inbound_io(
                    sender(),
                    &mut InstrumentedIo {
                        input: Cursor::new(input),
                        output,
                        reads,
                        pending_at_eof: true,
                    },
                    router,
                    MessageFrameMemoryBudget::new(),
                )
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            while lock_unpoisoned(&output).len() < 6 + 13 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("open and first message must be acknowledged");
        tokio::task::yield_now().await;
        assert_eq!(reads.load(Ordering::SeqCst), bytes_before_third_frame);

        let first = registration.recv().await.unwrap();
        assert_eq!(first.sender, sender());
        assert_eq!(first.message.r#type, "message-1");
        tokio::time::timeout(Duration::from_secs(1), async {
            while lock_unpoisoned(&output).len() < 6 + 2 * 13 {
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
    async fn mismatched_ack_is_a_terminal_send_error() {
        let mut bytes = Vec::new();
        write_ack_frame(&mut bytes, 9).await.unwrap();
        let mut io = InstrumentedIo {
            input: Cursor::new(bytes),
            output: Arc::new(Mutex::new(Vec::new())),
            reads: Arc::new(AtomicUsize::new(0)),
            pending_at_eof: false,
        };
        assert!(matches!(
            exchange_message(
                &mut io,
                8,
                &Message {
                    r#type: "type".into(),
                    timestamp_ns: 1,
                    payload: Vec::new(),
                }
            )
            .await,
            Err(SendMessageError::AckSequenceMismatch {
                expected: 8,
                actual: 9
            })
        ));
    }

    #[tokio::test]
    async fn ack_loss_after_enqueue_is_an_indeterminate_send_error() {
        let router = MessageChannelRouter::new(owner());
        let mut registration = router.register(resource(owner(), "ack-loss"), 1).unwrap();
        let (mut sender_io, mut receiver_io) = LoopbackIo::pair_with_receiver_ack_loss();
        let inbound = tokio::spawn(async move {
            handle_inbound_io(
                sender(),
                &mut receiver_io,
                router,
                MessageFrameMemoryBudget::new(),
            )
            .await
        });

        write_open_frame(
            &mut sender_io,
            owner(),
            "ack-loss",
            &resource(owner(), "ack-loss").clock,
        )
        .await
        .unwrap();
        assert!(read_open_response(&mut sender_io).await.unwrap());

        assert!(matches!(
            exchange_message(
                &mut sender_io,
                1,
                &Message {
                    r#type: "application.command.v1".into(),
                    timestamp_ns: 42,
                    payload: b"opaque".to_vec(),
                }
            )
            .await,
            Err(SendMessageError::Codec(_))
        ));
        let delivered = registration.recv().await.unwrap();
        assert_eq!(delivered.sender, sender());
        assert_eq!(delivered.message.r#type, "application.command.v1");
        assert_eq!(delivered.message.payload, b"opaque");

        assert!(matches!(
            inbound.await.unwrap(),
            Err(MessageEndpointError::Codec(MessageProtocolError::Io(_)))
        ));
    }
}
