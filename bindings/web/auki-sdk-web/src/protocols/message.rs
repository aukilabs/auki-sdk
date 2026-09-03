use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use auki_protocols::message::{
    MessageChannelReceiver, MessageChannelResource, MessageChannelSender, MessageClient,
    MessageEndpoint, MessageEvent, v1::ID,
};
use auki_registry::RegistryRef;
use futures::{FutureExt, pin_mut};
use js_sys::{Promise, Reflect, Uint8Array};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::{
    AukiPeer,
    protocol_support::{
        AuthenticatedPeerRecord, CloseBarrier, js_context, js_error, parse_exact_target,
        peer_protocols, to_js_value,
    },
};

/// Outbound Message v1 client backed by the portable Rust protocol.
#[wasm_bindgen]
pub struct AukiMessageClient {
    inner: MessageClient,
}

#[wasm_bindgen]
impl AukiMessageClient {
    /// Bind an outbound Message client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiMessageClient, JsValue> {
        Ok(Self {
            inner: MessageClient::new(peer_protocols(peer, "Message")?),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Open one persistent receiver-owned channel through an exact advertised route.
    #[wasm_bindgen(js_name = openExact)]
    pub async fn open_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        #[wasm_bindgen(unchecked_param_type = "AukiMessageChannelResource")] channel: JsValue,
    ) -> Result<AukiMessageSender, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let channel = message_channel_from_js(channel)?;
        let sender = self
            .inner
            .open_exact(peer_id, route, &channel)
            .await
            .map_err(|error| js_context("open Message channel", error))?;
        Ok(AukiMessageSender::new(sender))
    }
}

/// Mounted inbound Message v1 service and its declared channels.
#[wasm_bindgen]
pub struct AukiMessageEndpoint {
    inner: RefCell<Option<MessageEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiMessageEndpoint {
    /// Mount Message v1 on one running browser peer.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(peer: &AukiPeer) -> Result<AukiMessageEndpoint, JsValue> {
        let endpoint = MessageEndpoint::mount(peer_protocols(peer, "Message endpoint")?)
            .map_err(|error| js_context("mount Message endpoint", error))?;
        Ok(Self {
            inner: RefCell::new(Some(endpoint)),
            closing: CloseBarrier::default(),
        })
    }

    /// Declare one receiver-owned channel and its bounded application queue.
    pub fn declare(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiMessageChannelResource")] channel: JsValue,
        receiver_capacity: usize,
    ) -> Result<AukiMessageReceiver, JsValue> {
        let channel = message_channel_from_js(channel)?;
        let receiver = self
            .inner
            .borrow()
            .as_ref()
            .ok_or_else(|| js_error("Message endpoint is closed"))?
            .declare(channel, receiver_capacity)
            .map_err(|error| js_context("declare Message channel", error))?;
        Ok(AukiMessageReceiver::new(receiver))
    }

    /// Snapshot all currently declared channels as canonical Catalog-compatible rows.
    #[wasm_bindgen(unchecked_return_type = "AukiMessageChannelResource[]")]
    pub fn catalog(&self) -> Result<JsValue, JsValue> {
        let channels = self
            .inner
            .borrow()
            .as_ref()
            .ok_or_else(|| js_error("Message endpoint is closed"))?
            .catalog();
        message_channel_catalog_to_js(&channels)
    }

    /// Idempotently stop declarations and await every admitted handler.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Message endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

/// Persistent outbound Message channel.
#[wasm_bindgen]
pub struct AukiMessageSender {
    inner: RefCell<Option<MessageChannelSender>>,
    closing: CloseBarrier,
    remote_peer: AuthenticatedPeerRecord,
    channel: MessageChannelRecord,
    relayed: bool,
}

impl AukiMessageSender {
    fn new(sender: MessageChannelSender) -> Self {
        Self {
            remote_peer: AuthenticatedPeerRecord::from(sender.remote_peer()),
            channel: MessageChannelRecord::from(sender.resource()),
            relayed: sender.is_relayed(),
            inner: RefCell::new(Some(sender)),
            closing: CloseBarrier::default(),
        }
    }
}

#[wasm_bindgen]
impl AukiMessageSender {
    /// Mutually authenticated receiver metadata, without credentials or proofs.
    #[wasm_bindgen(getter, js_name = remotePeer, unchecked_return_type = "AukiAuthenticatedPeer")]
    pub fn remote_peer(&self) -> Result<JsValue, JsValue> {
        to_js_value("convert authenticated Message receiver", &self.remote_peer)
    }

    /// Exact receiver-owned channel bound by the open handshake.
    #[wasm_bindgen(getter, unchecked_return_type = "AukiMessageChannelResource")]
    pub fn channel(&self) -> Result<JsValue, JsValue> {
        to_js_value("convert Message channel", &self.channel)
    }

    /// Whether this persistent channel uses a relay circuit.
    #[wasm_bindgen(getter)]
    pub fn relayed(&self) -> bool {
        self.relayed
    }

    /// Send one opaque typed message and wait for its exact acknowledgement.
    pub async fn send(
        &self,
        #[wasm_bindgen(js_name = type)] message_type: String,
        timestamp_ns: i64,
        payload: Vec<u8>,
    ) -> Result<(), JsValue> {
        let sender = self
            .inner
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| js_error("Message sender is closed"))?;
        sender
            .send(message_type, timestamp_ns, payload)
            .await
            .map_err(|error| js_context("send Message", error))
    }

    /// Idempotently close the channel for every sender clone.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let sender = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(sender) = sender {
                    sender
                        .close()
                        .await
                        .map_err(|error| js_context("close Message sender", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

/// One bounded receiver declaration.
#[wasm_bindgen]
pub struct AukiMessageReceiver {
    inner: Rc<MessageReceiverState>,
    closing: CloseBarrier,
}

impl AukiMessageReceiver {
    fn new(receiver: MessageChannelReceiver) -> Self {
        Self {
            inner: Rc::new(MessageReceiverState::new(receiver)),
            closing: CloseBarrier::default(),
        }
    }
}

#[wasm_bindgen]
impl AukiMessageReceiver {
    /// Exact receiver-owned channel bound to this declaration.
    #[wasm_bindgen(getter, unchecked_return_type = "AukiMessageChannelResource")]
    pub fn channel(&self) -> Result<JsValue, JsValue> {
        to_js_value("convert Message channel", &self.inner.channel)
    }

    /// Receive one message or `null` after close/undeclaration.
    ///
    /// Only one pending call is permitted. Closing the receiver wakes a pending
    /// call and resolves it with `null`.
    #[wasm_bindgen(unchecked_return_type = "Promise<AukiMessageEvent | null>")]
    pub fn next(&self) -> Result<Promise, JsValue> {
        match self.inner.begin_next()? {
            Some(receiver) => {
                let state = Rc::clone(&self.inner);
                Ok(future_to_promise(async move {
                    receive_next(state, receiver).await
                }))
            }
            None => Ok(Promise::resolve(&JsValue::NULL)),
        }
    }

    /// Idempotently undeclare this channel and cancel a pending `next()`.
    ///
    /// The returned Promise resolves only after the Rust receiver has been
    /// dropped, including when a pending `next()` temporarily owns it.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let state = Rc::clone(&self.inner);
            state.close();
            future_to_promise(async move {
                state.wait_closed().await;
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

impl Drop for AukiMessageReceiver {
    fn drop(&mut self) {
        self.inner.close();
    }
}

struct MessageReceiverState {
    receiver: RefCell<Option<MessageChannelReceiver>>,
    channel: MessageChannelRecord,
    closed: Cell<bool>,
    next_pending: Cell<bool>,
    cancel_sender: async_channel::Sender<()>,
    cancel_receiver: async_channel::Receiver<()>,
    cleanup_sender: async_channel::Sender<()>,
    cleanup_receiver: async_channel::Receiver<()>,
}

impl MessageReceiverState {
    fn new(receiver: MessageChannelReceiver) -> Self {
        let channel = MessageChannelRecord::from(receiver.resource());
        let (cancel_sender, cancel_receiver) = async_channel::bounded(1);
        let (cleanup_sender, cleanup_receiver) = async_channel::bounded(1);
        Self {
            receiver: RefCell::new(Some(receiver)),
            channel,
            closed: Cell::new(false),
            next_pending: Cell::new(false),
            cancel_sender,
            cancel_receiver,
            cleanup_sender,
            cleanup_receiver,
        }
    }

    fn begin_next(&self) -> Result<Option<MessageChannelReceiver>, JsValue> {
        if self.closed.get() {
            return Ok(None);
        }
        if self.next_pending.replace(true) {
            return Err(js_error("Message receiver already has a pending next()"));
        }
        match self.receiver.borrow_mut().take() {
            Some(receiver) => Ok(Some(receiver)),
            None => {
                self.next_pending.set(false);
                Err(js_error("Message receiver is unavailable"))
            }
        }
    }

    fn finish_next(&self, receiver: MessageChannelReceiver, ended: bool) {
        self.next_pending.set(false);
        if self.closed.get() || ended {
            self.closed.set(true);
            self.cancel_sender.close();
            drop(receiver);
            self.complete_cleanup();
        } else {
            self.receiver.borrow_mut().replace(receiver);
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.cancel_sender.close();
        if let Some(receiver) = self.receiver.borrow_mut().take() {
            drop(receiver);
            self.complete_cleanup();
        }
    }

    fn complete_cleanup(&self) {
        let _ = self.cleanup_sender.try_send(());
        self.cleanup_sender.close();
    }

    async fn wait_closed(&self) {
        let _ = self.cleanup_receiver.recv().await;
    }
}

async fn receive_next(
    state: Rc<MessageReceiverState>,
    mut receiver: MessageChannelReceiver,
) -> Result<JsValue, JsValue> {
    let event = {
        let cancelled = state.cancel_receiver.recv().fuse();
        let received = receiver.recv().fuse();
        pin_mut!(cancelled, received);
        futures::select_biased! {
            _ = cancelled => None,
            event = received => event,
        }
    };
    let ended = event.is_none();
    state.finish_next(receiver, ended);
    match event {
        Some(event) => message_event_to_js(event),
        None => Ok(JsValue::NULL),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MessageChannelVariant {
    MessageChannel,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MessageChannelRecord {
    variant: MessageChannelVariant,
    owner_peer_id: String,
    resource_id: String,
    clock: RegistryRef,
}

impl From<&MessageChannelResource> for MessageChannelRecord {
    fn from(resource: &MessageChannelResource) -> Self {
        Self {
            variant: MessageChannelVariant::MessageChannel,
            owner_peer_id: resource.owner_peer_id.to_string(),
            resource_id: resource.resource_id.clone(),
            clock: resource.clock.clone(),
        }
    }
}

fn message_channel_from_js(value: JsValue) -> Result<MessageChannelResource, JsValue> {
    let record: MessageChannelRecord = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read Message channel", error))?;
    let resource = MessageChannelResource {
        owner_peer_id: record
            .owner_peer_id
            .parse()
            .map_err(|error| js_context("parse Message channel owner Peer ID", error))?,
        resource_id: record.resource_id,
        clock: record.clock,
    };
    resource
        .validate()
        .map_err(|error| js_context("validate Message channel", error))?;
    Ok(resource)
}

fn message_channel_catalog_to_js(channels: &[MessageChannelResource]) -> Result<JsValue, JsValue> {
    let records = channels
        .iter()
        .map(MessageChannelRecord::from)
        .collect::<Vec<_>>();
    to_js_value("convert Message channel catalog", &records)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageEventRecord {
    channel: MessageChannelRecord,
    sender: AuthenticatedPeerRecord,
    #[serde(rename = "type")]
    message_type: String,
    timestamp_ns: i64,
}

fn message_event_to_js(event: MessageEvent) -> Result<JsValue, JsValue> {
    message_record_to_js(
        MessageEventRecord {
            channel: MessageChannelRecord::from(&event.channel),
            sender: AuthenticatedPeerRecord::from(&event.sender),
            message_type: event.message.r#type,
            timestamp_ns: event.message.timestamp_ns,
        },
        &event.message.payload,
    )
}

fn message_record_to_js(record: MessageEventRecord, payload: &[u8]) -> Result<JsValue, JsValue> {
    let value = to_js_value("convert Message event", &record)?;
    let attached = Reflect::set(
        &value,
        &JsValue::from_str("payload"),
        &Uint8Array::from(payload),
    )
    .map_err(|error| js_context("attach Message payload", format!("{error:?}")))?;
    if !attached {
        return Err(js_error(
            "attach Message payload: property assignment failed",
        ));
    }
    Ok(value)
}

#[wasm_bindgen(typescript_custom_section)]
const MESSAGE_TYPESCRIPT: &str = r#"
/** Immutable content-addressed Registry reference. */
export interface AukiRegistryRef {
    readonly peer_id: string;
    readonly id: string;
    readonly hash: string;
}

/** Canonical Catalog v3 message_channel row accepted directly by Message v1. */
export interface AukiMessageChannelResource {
    readonly variant: "message_channel";
    readonly owner_peer_id: string;
    readonly resource_id: string;
    readonly clock: AukiRegistryRef;
}

/** One accepted live message with its authenticated sender and channel. */
export interface AukiMessageEvent {
    readonly channel: AukiMessageChannelResource;
    readonly sender: AukiAuthenticatedPeer;
    readonly type: string;
    readonly timestampNs: bigint;
    readonly payload: Uint8Array;
}
"#;

#[cfg(test)]
mod tests {
    use js_sys::{Array, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    fn channel() -> MessageChannelResource {
        let owner_peer_id = peer_id();
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

    #[wasm_bindgen_test]
    fn channel_records_match_catalog_v3_and_round_trip_directly() {
        let value = message_channel_catalog_to_js(&[channel()]).unwrap();
        assert!(Array::is_array(&value));
        let row = Array::from(&value).get(0);
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("variant"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("message_channel")
        );
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("owner_peer_id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some(peer_id().to_string().as_str())
        );

        assert_eq!(message_channel_from_js(row).unwrap(), channel());
    }

    #[wasm_bindgen_test]
    fn message_events_use_bigint_timestamps_and_uint8array_payloads() {
        let value = message_record_to_js(
            MessageEventRecord {
                channel: MessageChannelRecord::from(&channel()),
                sender: AuthenticatedPeerRecord {
                    peer_id: "sender".into(),
                    subject: "subject".into(),
                    peer_type: Some("robot".into()),
                    domain_ids: vec!["domain".into()],
                    scopes: vec!["message:send".into()],
                    application: None,
                    verified_until: "2026-09-01T00:00:00Z".into(),
                },
                message_type: "example.pose".into(),
                timestamp_ns: i64::MAX,
            },
            &[1, 2, 3],
        )
        .unwrap();

        assert!(
            Reflect::get(&value, &JsValue::from_str("timestampNs"))
                .unwrap()
                .is_bigint()
        );
        let payload = Reflect::get(&value, &JsValue::from_str("payload")).unwrap();
        assert!(payload.is_instance_of::<Uint8Array>());
        assert_eq!(Uint8Array::new(&payload).to_vec(), vec![1, 2, 3]);
        assert!(
            Reflect::get(&value, &JsValue::from_str("sender"))
                .unwrap()
                .is_object()
        );
    }

    #[wasm_bindgen_test(async)]
    async fn receiver_close_signal_is_idempotent_and_wakes_pending_waiters() {
        let (sender, receiver) = async_channel::bounded::<()>(1);
        sender.close();
        sender.close();
        assert!(receiver.recv().await.is_err());
    }
}
