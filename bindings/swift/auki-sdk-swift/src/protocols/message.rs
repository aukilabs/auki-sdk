//! Swift adapter for persistent authenticated Message v1 channels.

use std::sync::Arc;

use auki_protocols::message::{
    MessageChannelReceiver, MessageChannelRegistrationError, MessageChannelResource,
    MessageChannelSender, MessageClient, MessageEndpoint, MessageEvent,
    v1::{ID, MAX_MESSAGE_FRAME_BYTES},
};
use auki_registry::RegistryRef;
use auki_sdk_rs::{AuthenticatedPeer, Multiaddr, PeerId};
use parking_lot::Mutex;
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    AukiPeer, AukiPeerTarget, AukiSdkError, CleanupResult, DetachedCleanup, operation_error,
    parse_target, wait_cleanup,
};

/// Maximum Swift-selected queue depth for one live Message declaration.
///
/// Message payload memory is independently bounded by the Rust protocol. This
/// bound prevents an accidental FFI value from creating an unreasonable
/// number of queue slots before any message arrives.
const MAX_MESSAGE_RECEIVER_CAPACITY: u32 = 65_536;
const MESSAGE_OPEN_FIXED_BYTES: usize = 1 + 5 * std::mem::size_of::<u32>();

/// Immutable content-addressed clock reference carried by a Message channel.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiMessageClockReference {
    pub peer_id: String,
    pub id: String,
    pub hash: String,
}

impl From<&RegistryRef> for AukiMessageClockReference {
    fn from(value: &RegistryRef) -> Self {
        Self {
            peer_id: value.peer_id.clone(),
            id: value.id.clone(),
            hash: value.hash.clone(),
        }
    }
}

/// Receiver-owned identity of one persistent typed-message channel.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiMessageChannel {
    pub owner_peer_id: String,
    pub resource_id: String,
    pub clock: AukiMessageClockReference,
}

impl From<&MessageChannelResource> for AukiMessageChannel {
    fn from(value: &MessageChannelResource) -> Self {
        Self {
            owner_peer_id: value.owner_peer_id.to_string(),
            resource_id: value.resource_id.clone(),
            clock: AukiMessageClockReference::from(&value.clock),
        }
    }
}

fn channel_from_record(
    channel: AukiMessageChannel,
) -> Result<MessageChannelResource, AukiSdkError> {
    let owner_peer_id = channel
        .owner_peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse Message channel owner Peer ID", error))?;
    let clock_peer_id = channel
        .clock
        .peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse Message channel clock Peer ID", error))?;
    let resource = MessageChannelResource {
        owner_peer_id,
        resource_id: channel.resource_id,
        clock: RegistryRef {
            peer_id: clock_peer_id.to_string(),
            id: channel.clock.id,
            hash: channel.clock.hash,
        },
    };
    resource
        .validate()
        .map_err(|error| operation_error("validate Message channel", error))?;
    validate_channel_wire_bound(&resource)?;
    Ok(resource)
}

fn validate_channel_wire_bound(resource: &MessageChannelResource) -> Result<(), AukiSdkError> {
    let encoded_bytes = [
        resource.owner_peer_id.to_string().len(),
        resource.resource_id.len(),
        resource.clock.peer_id.len(),
        resource.clock.id.len(),
        resource.clock.hash.len(),
    ]
    .into_iter()
    .try_fold(MESSAGE_OPEN_FIXED_BYTES, usize::checked_add)
    .ok_or_else(|| operation_error("validate Message channel", "open frame size overflow"))?;
    if encoded_bytes > MAX_MESSAGE_FRAME_BYTES as usize {
        return Err(operation_error(
            "validate Message channel",
            format!("open frame is {encoded_bytes} bytes; maximum is {MAX_MESSAGE_FRAME_BYTES}"),
        ));
    }
    Ok(())
}

/// Non-authoritative application metadata proven by the remote DDS token.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiMessageApplication {
    pub name: String,
    pub version: String,
}

/// Complete authenticated identity metadata for one Message participant.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiMessageAuthenticatedPeer {
    pub peer_id: String,
    pub subject: String,
    pub peer_type: Option<String>,
    pub domain_ids: Vec<String>,
    pub scopes: Vec<String>,
    pub application: Option<AukiMessageApplication>,
    pub verified_until: String,
}

impl From<&AuthenticatedPeer> for AukiMessageAuthenticatedPeer {
    fn from(value: &AuthenticatedPeer) -> Self {
        Self {
            peer_id: value.peer_id.to_string(),
            subject: value.subject.to_string(),
            peer_type: value.peer_type.clone(),
            domain_ids: value.domain_ids.iter().map(ToString::to_string).collect(),
            scopes: value.scopes.clone(),
            application: value
                .application
                .as_ref()
                .map(|application| AukiMessageApplication {
                    name: application.name.clone(),
                    version: application.version.clone(),
                }),
            verified_until: value.verified_until.to_rfc3339(),
        }
    }
}

/// One accepted Message event with opaque binary application payload.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiMessageEvent {
    pub channel: AukiMessageChannel,
    pub sender: AukiMessageAuthenticatedPeer,
    pub message_type: String,
    pub timestamp_ns: i64,
    pub payload: Vec<u8>,
}

impl From<MessageEvent> for AukiMessageEvent {
    fn from(value: MessageEvent) -> Self {
        Self {
            channel: AukiMessageChannel::from(&value.channel),
            sender: AukiMessageAuthenticatedPeer::from(&value.sender),
            message_type: value.message.r#type,
            timestamp_ns: value.message.timestamp_ns,
            payload: value.message.payload,
        }
    }
}

/// Validate Domain equality before parsing one exact Message target.
fn exact_target(
    local_domain_id: &str,
    target: AukiPeerTarget,
) -> Result<(PeerId, Multiaddr), AukiSdkError> {
    let local_domain = Uuid::parse_str(local_domain_id)
        .map_err(|error| operation_error("parse local Auki Domain ID", error))?;
    let target_domain = Uuid::parse_str(&target.domain_id)
        .map_err(|error| operation_error("parse target Auki Domain ID", error))?;
    if local_domain != target_domain {
        return Err(operation_error(
            "validate exact Message target",
            format!(
                "target Domain {} does not match local Domain {local_domain_id}",
                target.domain_id
            ),
        ));
    }
    parse_target(target)
}

/// Outbound Message v1 client over one running native Auki peer.
#[derive(uniffi::Object)]
pub struct AukiMessageClient {
    inner: MessageClient,
    domain_id: String,
}

impl AukiMessageClient {
    fn from_inner(inner: MessageClient, domain_id: String) -> Arc<Self> {
        Arc::new(Self { inner, domain_id })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiMessageClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(MessageClient::new(peer.rust_protocols()), peer.domain_id())
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    /// Open one persistent channel through the exact advertised relay route.
    pub async fn open_exact(
        &self,
        target: AukiPeerTarget,
        channel: AukiMessageChannel,
    ) -> Result<Arc<AukiMessageSender>, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        let channel = channel_from_record(channel)?;
        let sender = self
            .inner
            .open_exact(peer_id, route, &channel)
            .await
            .map_err(|error| operation_error("open exact Message channel", error))?;
        Ok(AukiMessageSender::new(sender))
    }
}

struct MessageEndpointOwner {
    endpoint: Mutex<Option<MessageEndpoint>>,
    cleanup: DetachedCleanup,
}

impl MessageEndpointOwner {
    fn new(endpoint: MessageEndpoint) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            cleanup: DetachedCleanup::new(),
        }
    }

    fn declare(
        &self,
        channel: MessageChannelResource,
        receiver_capacity: usize,
    ) -> Result<MessageChannelReceiver, AukiSdkError> {
        self.endpoint
            .lock()
            .as_ref()
            .ok_or_else(|| operation_error("declare Message channel", "endpoint is stopped"))?
            .declare(channel, receiver_capacity)
            .map_err(declaration_error)
    }

    fn catalog(&self) -> Result<Vec<MessageChannelResource>, AukiSdkError> {
        Ok(self
            .endpoint
            .lock()
            .as_ref()
            .ok_or_else(|| operation_error("read Message catalog", "endpoint is stopped"))?
            .catalog())
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let endpoint = self.endpoint.lock().take();
            async move {
                match endpoint {
                    Some(endpoint) => endpoint.close().await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for MessageEndpointOwner {
    fn drop(&mut self) {
        if self.endpoint.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

fn declaration_error(error: MessageChannelRegistrationError) -> AukiSdkError {
    match error {
        MessageChannelRegistrationError::Stopped => {
            operation_error("declare Message channel", "endpoint is stopped")
        }
        error => operation_error("declare Message channel", error),
    }
}

fn validate_receiver_capacity(value: u32) -> Result<usize, AukiSdkError> {
    if value == 0 || value > MAX_MESSAGE_RECEIVER_CAPACITY {
        return Err(operation_error(
            "validate Message receiver capacity",
            format!(
                "capacity must be between 1 and {MAX_MESSAGE_RECEIVER_CAPACITY}; received {value}"
            ),
        ));
    }
    Ok(value as usize)
}

/// Mounted inbound Message v1 service and its declared receivers.
#[derive(uniffi::Object)]
pub struct AukiMessageEndpoint {
    owner: MessageEndpointOwner,
    client: Arc<AukiMessageClient>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiMessageEndpoint {
    /// Mount Message v1 on one running peer.
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let endpoint = MessageEndpoint::mount(peer.rust_protocols())
            .map_err(|error| operation_error("mount Message endpoint", error))?;
        let client = AukiMessageClient::from_inner(endpoint.client(), peer.domain_id());
        Ok(Arc::new(Self {
            owner: MessageEndpointOwner::new(endpoint),
            client,
        }))
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    pub fn client(&self) -> Arc<AukiMessageClient> {
        Arc::clone(&self.client)
    }

    /// Declare one receiver-owned channel and its bounded native queue.
    ///
    /// This operation is async so its cleanup barrier always captures the
    /// UniFFI Tokio runtime, even though declaration itself performs no I/O.
    pub async fn declare(
        &self,
        channel: AukiMessageChannel,
        receiver_capacity: u32,
    ) -> Result<Arc<AukiMessageReceiver>, AukiSdkError> {
        let channel = channel_from_record(channel)?;
        let receiver = self
            .owner
            .declare(channel, validate_receiver_capacity(receiver_capacity)?)?;
        Ok(AukiMessageReceiver::new(receiver))
    }

    /// Snapshot every currently declared channel.
    pub fn catalog(&self) -> Result<Vec<AukiMessageChannel>, AukiSdkError> {
        Ok(self
            .owner
            .catalog()?
            .iter()
            .map(AukiMessageChannel::from)
            .collect())
    }

    /// Stop declarations and await all admitted handlers behind one barrier.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Message endpoint", error))
    }
}

struct MessageSenderOwner {
    sender: Mutex<Option<MessageChannelSender>>,
    cleanup: DetachedCleanup,
}

impl MessageSenderOwner {
    fn new(sender: MessageChannelSender) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
            cleanup: DetachedCleanup::new(),
        }
    }

    fn sender(&self) -> Result<MessageChannelSender, AukiSdkError> {
        self.sender
            .lock()
            .as_ref()
            .cloned()
            .ok_or_else(|| operation_error("use Message sender", "sender is closed"))
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let sender = self.sender.lock().take();
            async move {
                match sender {
                    Some(sender) => sender.close().await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for MessageSenderOwner {
    fn drop(&mut self) {
        if self.sender.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

/// Persistent outbound Message v1 channel.
#[derive(uniffi::Object)]
pub struct AukiMessageSender {
    owner: MessageSenderOwner,
    remote_peer: AukiMessageAuthenticatedPeer,
    channel: AukiMessageChannel,
    relayed: bool,
}

impl AukiMessageSender {
    fn new(sender: MessageChannelSender) -> Arc<Self> {
        Arc::new(Self {
            remote_peer: AukiMessageAuthenticatedPeer::from(sender.remote_peer()),
            channel: AukiMessageChannel::from(sender.resource()),
            relayed: sender.is_relayed(),
            owner: MessageSenderOwner::new(sender),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiMessageSender {
    /// Mutually authenticated receiver metadata without credentials or proofs.
    pub fn remote_peer(&self) -> AukiMessageAuthenticatedPeer {
        self.remote_peer.clone()
    }

    pub fn channel(&self) -> AukiMessageChannel {
        self.channel.clone()
    }

    pub fn relayed(&self) -> bool {
        self.relayed
    }

    /// Send one opaque typed message and await its exact acknowledgement.
    pub async fn send(
        &self,
        message_type: String,
        timestamp_ns: i64,
        payload: Vec<u8>,
    ) -> Result<(), AukiSdkError> {
        self.owner
            .sender()?
            .send(message_type, timestamp_ns, payload)
            .await
            .map_err(|error| operation_error("send Message", error))
    }

    /// Close every clone of the native sender behind one replayable barrier.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Message sender", error))
    }
}

struct MessageReceiverSlot<R> {
    receiver: Option<R>,
    closed: bool,
    next_pending: bool,
}

impl<R> MessageReceiverSlot<R> {
    fn new(receiver: R) -> Self {
        Self {
            receiver: Some(receiver),
            closed: false,
            next_pending: false,
        }
    }

    fn begin_next(&mut self) -> Result<Option<R>, AukiSdkError> {
        if self.closed {
            return Ok(None);
        }
        if self.next_pending {
            return Err(operation_error(
                "receive Message",
                "receiver already has a pending next()",
            ));
        }
        let Some(receiver) = self.receiver.take() else {
            self.closed = true;
            return Err(operation_error(
                "receive Message",
                "receiver state is unavailable",
            ));
        };
        self.next_pending = true;
        Ok(Some(receiver))
    }

    /// Return a receiver to the slot, or return it to the caller for dropping.
    fn finish_next(&mut self, receiver: R, ended: bool) -> (Option<R>, bool) {
        self.next_pending = false;
        if self.closed || ended {
            self.closed = true;
            (Some(receiver), true)
        } else {
            self.receiver = Some(receiver);
            (None, false)
        }
    }

    /// Fence future receives and return an idle receiver for immediate drop.
    fn begin_close(&mut self) -> (Option<R>, bool) {
        self.closed = true;
        if self.next_pending {
            (None, false)
        } else {
            (self.receiver.take(), true)
        }
    }
}

struct MessageReceiverState {
    slot: Mutex<MessageReceiverSlot<MessageChannelReceiver>>,
    channel: AukiMessageChannel,
    cancel: watch::Sender<bool>,
    completed: watch::Sender<bool>,
    cleanup: DetachedCleanup,
}

impl MessageReceiverState {
    fn new(receiver: MessageChannelReceiver) -> Self {
        let channel = AukiMessageChannel::from(receiver.resource());
        let (cancel, _) = watch::channel(false);
        let (completed, _) = watch::channel(false);
        Self {
            slot: Mutex::new(MessageReceiverSlot::new(receiver)),
            channel,
            cancel,
            completed,
            cleanup: DetachedCleanup::new(),
        }
    }

    fn begin_next(self: &Arc<Self>) -> Result<Option<PendingMessageReceiver>, AukiSdkError> {
        let receiver = self.slot.lock().begin_next()?;
        Ok(receiver.map(|receiver| PendingMessageReceiver {
            state: Arc::clone(self),
            receiver: Some(receiver),
        }))
    }

    fn finish_next(&self, receiver: MessageChannelReceiver, ended: bool) {
        let (receiver, complete) = self.slot.lock().finish_next(receiver, ended);
        drop(receiver);
        if complete {
            self.cancel.send_replace(true);
            self.completed.send_replace(true);
        }
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            self.cancel.send_replace(true);
            let (receiver, complete) = self.slot.lock().begin_close();
            drop(receiver);
            if complete {
                self.completed.send_replace(true);
            }
            let completion = self.completed.subscribe();
            async move { wait_receiver_completion(completion).await }
        })
    }
}

struct PendingMessageReceiver {
    state: Arc<MessageReceiverState>,
    receiver: Option<MessageChannelReceiver>,
}

impl PendingMessageReceiver {
    fn receiver(&mut self) -> &mut MessageChannelReceiver {
        self.receiver
            .as_mut()
            .expect("a pending Message receive owns the native receiver")
    }

    fn finish(mut self, ended: bool) {
        let receiver = self
            .receiver
            .take()
            .expect("a pending Message receive finishes only once");
        self.state.finish_next(receiver, ended);
    }
}

impl Drop for PendingMessageReceiver {
    fn drop(&mut self) {
        if let Some(receiver) = self.receiver.take() {
            self.state.finish_next(receiver, false);
        }
    }
}

async fn receive_next(mut pending: PendingMessageReceiver) -> Option<MessageEvent> {
    let mut cancellation = pending.state.cancel.subscribe();
    let event = if *cancellation.borrow() {
        None
    } else {
        tokio::select! {
            biased;
            _ = cancellation.changed() => None,
            event = pending.receiver().recv() => event,
        }
    };
    let ended = event.is_none();
    pending.finish(ended);
    event
}

async fn wait_receiver_completion(mut completion: watch::Receiver<bool>) -> Result<(), String> {
    loop {
        if *completion.borrow_and_update() {
            return Ok(());
        }
        if completion.changed().await.is_err() {
            return Err("Message receiver cleanup ended without a result".into());
        }
    }
}

/// One bounded receiver declaration.
#[derive(uniffi::Object)]
pub struct AukiMessageReceiver {
    state: Arc<MessageReceiverState>,
}

impl AukiMessageReceiver {
    fn new(receiver: MessageChannelReceiver) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(MessageReceiverState::new(receiver)),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiMessageReceiver {
    pub fn channel(&self) -> AukiMessageChannel {
        self.state.channel.clone()
    }

    /// Receive one event, or `nil` after close or undeclaration.
    /// Only one call may be pending at a time.
    pub async fn next(&self) -> Result<Option<AukiMessageEvent>, AukiSdkError> {
        let Some(pending) = self.state.begin_next()? else {
            return Ok(None);
        };
        Ok(receive_next(pending).await.map(AukiMessageEvent::from))
    }

    /// Undeclare the channel and await native receiver cleanup.
    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.state.begin_close())
            .await
            .map_err(|error| operation_error("close Message receiver", error))
    }
}

impl Drop for AukiMessageReceiver {
    fn drop(&mut self) {
        let _ = self.state.begin_close();
    }
}

#[cfg(test)]
mod tests {
    use auki_protocols::message::v1::Message;
    use auki_sdk_rs::Identity;

    use super::*;

    fn channel() -> MessageChannelResource {
        let owner_peer_id = Identity::generate().peer_id();
        MessageChannelResource {
            owner_peer_id,
            resource_id: "events".into(),
            clock: RegistryRef {
                peer_id: owner_peer_id.to_string(),
                id: "session/monotonic".into(),
                hash: "clock-hash".into(),
            },
        }
    }

    fn requester(peer_id: PeerId) -> AuthenticatedPeer {
        AuthenticatedPeer {
            peer_id,
            subject: Uuid::nil(),
            peer_type: Some("native_app".into()),
            domain_ids: vec![Uuid::nil()],
            scopes: vec!["protocol:test".into()],
            application: None,
            verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn channel_records_are_validated_canonical_and_bounded() {
        let expected = channel();
        let record = AukiMessageChannel::from(&expected);
        assert_eq!(channel_from_record(record.clone()).unwrap(), expected);

        let mut invalid = record.clone();
        invalid.owner_peer_id = "not-a-peer".into();
        assert!(channel_from_record(invalid).is_err());

        let mut invalid = record.clone();
        invalid.clock.peer_id = "not-a-peer".into();
        assert!(channel_from_record(invalid).is_err());

        let mut oversized = record;
        oversized.resource_id = "x".repeat(MAX_MESSAGE_FRAME_BYTES as usize);
        assert!(channel_from_record(oversized).is_err());
    }

    #[test]
    fn message_events_preserve_authenticated_sender_and_binary_payload() {
        let channel = channel();
        let sender_peer_id = Identity::generate().peer_id();
        let record = AukiMessageEvent::from(MessageEvent {
            channel: channel.clone(),
            sender: requester(sender_peer_id),
            message: Message {
                r#type: "example.event".into(),
                timestamp_ns: i64::MAX,
                payload: vec![0, 1, 127, 255],
            },
        });
        assert_eq!(record.channel, AukiMessageChannel::from(&channel));
        assert_eq!(record.sender.peer_id, sender_peer_id.to_string());
        assert_eq!(record.sender.peer_type.as_deref(), Some("native_app"));
        assert_eq!(record.sender.scopes, ["protocol:test"]);
        assert_eq!(record.message_type, "example.event");
        assert_eq!(record.timestamp_ns, i64::MAX);
        assert_eq!(record.payload, [0, 1, 127, 255]);
    }

    #[test]
    fn exact_targets_require_the_same_domain_before_route_parsing() {
        let target = AukiPeerTarget {
            domain_id: "00000000-0000-0000-0000-000000000002".into(),
            peer_id: "not-a-peer".into(),
            route: "not-a-route".into(),
        };
        let error = exact_target("00000000-0000-0000-0000-000000000001", target)
            .expect_err("different Domains must fail");
        assert!(error.to_string().contains("does not match local Domain"));
    }

    #[test]
    fn receiver_capacity_is_strictly_bounded() {
        assert!(validate_receiver_capacity(0).is_err());
        assert_eq!(validate_receiver_capacity(1).unwrap(), 1);
        assert_eq!(
            validate_receiver_capacity(MAX_MESSAGE_RECEIVER_CAPACITY).unwrap(),
            MAX_MESSAGE_RECEIVER_CAPACITY as usize
        );
        assert!(validate_receiver_capacity(MAX_MESSAGE_RECEIVER_CAPACITY + 1).is_err());
    }

    #[test]
    fn receiver_slot_allows_only_one_pending_receive_and_closes_once() {
        let mut slot = MessageReceiverSlot::new(7_u8);
        let receiver = slot.begin_next().unwrap().unwrap();
        assert!(slot.begin_next().is_err());

        assert_eq!(slot.finish_next(receiver, false), (None, false));
        let receiver = slot.begin_next().unwrap().unwrap();
        assert_eq!(slot.begin_close(), (None, false));
        assert!(slot.begin_next().unwrap().is_none());
        assert_eq!(slot.finish_next(receiver, false), (Some(7), true));

        assert_eq!(slot.begin_close(), (None, true));
        assert!(slot.begin_next().unwrap().is_none());
    }
}
