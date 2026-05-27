use crate::{BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, core};
use auki_identity::Wallet;
use auki_proto::message::MessageEnvelope;
use auki_proto::stream::{
    DeclineReason, EndReason, StreamEntry, StreamManifest, StreamMessage, StreamRequest,
    decline_reason, end_reason, stream_message,
};
use js_sys::{Error, Function, Object, Promise, Reflect};
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

const JOIN_PROTOCOL: &str = "/auki/join/0.0.1";
const RESOURCES_PROTOCOL: &str = "/auki/resources/0.0.1";
const SENSORS_PROTOCOL: &str = "/auki/sensors/0.0.1";
const STREAM_PROTOCOL: &str = "/auki/stream/0.1.0";

#[wasm_bindgen(js_name = peerDerivationLabel)]
pub fn peer_derivation_label() -> String {
    core::PEER_DERIVATION_LABEL.to_string()
}

#[wasm_bindgen(js_name = peerIdFromSeed)]
pub fn peer_id_from_seed(seed: &[u8]) -> Result<String, JsValue> {
    Ok(core::PeerIdentity::from_seed(&seed32(seed)?)
        .peer_id()
        .to_string())
}

#[wasm_bindgen(js_name = peerIdFromWalletSeed)]
pub fn peer_id_from_wallet_seed(seed: &[u8]) -> Result<String, JsValue> {
    let wallet = Wallet::from_seed(&seed32(seed)?);
    Ok(core::PeerIdentity::from_wallet(&wallet)
        .peer_id()
        .to_string())
}

#[wasm_bindgen(js_name = peerPublicKeyProtobufFromSeed)]
pub fn peer_public_key_protobuf_from_seed(seed: &[u8]) -> Result<Vec<u8>, JsValue> {
    Ok(core::PeerIdentity::from_seed(&seed32(seed)?)
        .public_key()
        .encode_protobuf())
}

#[wasm_bindgen(js_name = peerPrivateKeyProtobufFromSeed)]
pub fn peer_private_key_protobuf_from_seed(seed: &[u8]) -> Result<Vec<u8>, JsValue> {
    Ok(core::PeerIdentity::from_seed(&seed32(seed)?).private_key_protobuf())
}

#[wasm_bindgen(js_name = peerPrivateKeyProtobufFromWalletSeed)]
pub fn peer_private_key_protobuf_from_wallet_seed(seed: &[u8]) -> Result<Vec<u8>, JsValue> {
    let wallet = Wallet::from_seed(&seed32(seed)?);
    Ok(core::PeerIdentity::from_wallet(&wallet).private_key_protobuf())
}

#[wasm_bindgen(js_name = browserProbeProtocol)]
pub fn browser_probe_protocol() -> String {
    BROWSER_PROBE_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = messageProtocol)]
pub fn message_protocol() -> String {
    core::MESSAGE_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = joinProtocol)]
pub fn join_protocol() -> String {
    JOIN_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = resourcesProtocol)]
pub fn resources_protocol() -> String {
    RESOURCES_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = sensorsProtocol)]
pub fn sensors_protocol() -> String {
    SENSORS_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = streamProtocol)]
pub fn stream_protocol() -> String {
    STREAM_PROTOCOL.to_string()
}

#[wasm_bindgen(js_name = aukiNetworkProtocolsJson)]
pub fn auki_network_protocols_json() -> Result<String, JsValue> {
    serde_json::to_string(&serde_json::json!({
        "browser_probe": BROWSER_PROBE_PROTOCOL,
        "message": core::MESSAGE_PROTOCOL,
        "join": JOIN_PROTOCOL,
        "resources": RESOURCES_PROTOCOL,
        "sensors": SENSORS_PROTOCOL,
        "stream": STREAM_PROTOCOL,
    }))
    .map_err(json_error)
}

#[wasm_bindgen(js_name = formatSignaledAddress)]
pub fn format_signaled_address_js(
    discovery_url: String,
    peer_id: String,
) -> Result<String, JsValue> {
    crate::signaled_address::format_signaled_address(discovery_url, peer_id)
        .map_err(|err| js_error(err.to_string()))
}

#[wasm_bindgen(js_name = parseSignaledAddressJson)]
pub fn parse_signaled_address_json(address: String) -> Result<String, JsValue> {
    let parsed = crate::signaled_address::parse_signaled_address(&address)
        .map_err(|err| js_error(err.to_string()))?;
    serde_json::to_string(&serde_json::json!({
        "discovery_url": parsed.discovery_url,
        "peer_id": parsed.peer_id,
    }))
    .map_err(json_error)
}

#[wasm_bindgen]
pub struct DiscoveryDirectoryClient {
    base_url: String,
}

#[wasm_bindgen]
impl DiscoveryDirectoryClient {
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String) -> Result<DiscoveryDirectoryClient, JsValue> {
        let base_url = base_url.trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(js_error("DiscoveryDirectoryClient requires baseUrl"));
        }
        Ok(Self { base_url })
    }

    #[wasm_bindgen(js_name = registerPeerJson)]
    pub async fn register_peer_json(&self, registration_json: String) -> Result<String, JsValue> {
        let registration: DiscoveryRegistration =
            serde_json::from_str(&registration_json).map_err(json_error)?;
        let mode = registration.mode.as_deref().unwrap_or("create");
        match mode {
            "create" | "register" => {
                let body = json!({
                    "manager_peer_id": required_string(registration.manager_peer_id, "manager_peer_id")?,
                    "manager_multiaddrs": required_vec(registration.manager_multiaddrs, "manager_multiaddrs")?,
                    "relay_multiaddrs": registration.relay_multiaddrs.unwrap_or_default(),
                });
                let (status, text) = self
                    .fetch_text(
                        "POST",
                        &format!("/clusters/{}", encode_path_segment(&registration.name)),
                        Some(&serde_json::to_string(&body).map_err(json_error)?),
                    )
                    .await?;
                if status == 409 {
                    return serde_json::to_string(&json!({ "kind": "already_exists" }))
                        .map_err(json_error);
                }
                ensure_success(status, &text)?;
                Ok(compact_json(&json!({
                    "kind": "created",
                    "entry": normalize_cluster_entry(&text)?,
                }))?)
            }
            "rotate" | "rotate_manager" => {
                let body = json!({
                    "manager_peer_id": required_string(registration.manager_peer_id, "manager_peer_id")?,
                    "manager_multiaddrs": required_vec(registration.manager_multiaddrs, "manager_multiaddrs")?,
                    "relay_multiaddrs": registration.relay_multiaddrs.unwrap_or_default(),
                });
                let text = self
                    .fetch_success_text(
                        "POST",
                        &format!(
                            "/clusters/{}/manager",
                            encode_path_segment(&registration.name)
                        ),
                        Some(&serde_json::to_string(&body).map_err(json_error)?),
                    )
                    .await?;
                Ok(compact_json(&json!({
                    "kind": "updated",
                    "entry": normalize_cluster_entry(&text)?,
                }))?)
            }
            "liveness" => {
                let peer_count = registration
                    .peer_count
                    .ok_or_else(|| js_error("missing numeric field `peer_count`"))?;
                let body = json!({ "peer_count": peer_count });
                let text = self
                    .fetch_success_text(
                        "POST",
                        &format!(
                            "/clusters/{}/liveness",
                            encode_path_segment(&registration.name)
                        ),
                        Some(&serde_json::to_string(&body).map_err(json_error)?),
                    )
                    .await?;
                Ok(compact_json(&json!({
                    "kind": "liveness",
                    "entry": normalize_cluster_entry(&text)?,
                }))?)
            }
            other => Err(js_error(format!(
                "unsupported discovery registration mode: {other}"
            ))),
        }
    }

    #[wasm_bindgen(js_name = discoverPeersJson)]
    pub async fn discover_peers_json(&self, query_json: String) -> Result<String, JsValue> {
        let query: DiscoveryPeerQuery = serde_json::from_str(&query_json).map_err(json_error)?;
        let text = self.fetch_success_text("GET", "/clusters", None).await?;
        let mut list: DiscoveryClusterList = serde_json::from_str(&text).map_err(json_error)?;
        if let Some(name) = query.name {
            list.clusters.retain(|entry| entry.name == name);
        }
        serde_json::to_string(&list).map_err(json_error)
    }

    #[wasm_bindgen(js_name = listNodesJson)]
    pub async fn list_nodes_json(&self, query_json: String) -> Result<String, JsValue> {
        let query: DiscoveryNodeQuery = serde_json::from_str(&query_json).map_err(json_error)?;
        let path = match query.node_type {
            Some(node_type) => format!("/nodes?type={}", encode_path_segment(&node_type)),
            None => "/nodes".to_string(),
        };
        let text = self.fetch_success_text("GET", &path, None).await?;
        let list: DiscoveryNodeList = serde_json::from_str(&text).map_err(json_error)?;
        serde_json::to_string(&list).map_err(json_error)
    }

    #[wasm_bindgen(js_name = sendSignalJson)]
    pub async fn send_signal_json(&self, signal_json: String) -> Result<String, JsValue> {
        let signal: DiscoverySignalRequest =
            serde_json::from_str(&signal_json).map_err(json_error)?;
        let body = json!({
            "from_peer_id": required_nonempty(signal.from_peer_id, "from_peer_id")?,
            "connection_id": required_nonempty(signal.connection_id, "connection_id")?,
            "kind": required_nonempty(signal.kind, "kind")?,
            "payload": signal.payload.unwrap_or(serde_json::Value::Null),
        });
        let text = self
            .fetch_success_text(
                "POST",
                &format!(
                    "/signals/{}",
                    encode_path_segment(&required_nonempty(
                        signal.recipient_peer_id,
                        "recipient_peer_id"
                    )?)
                ),
                Some(&serde_json::to_string(&body).map_err(json_error)?),
            )
            .await?;
        let message: DiscoverySignalMessage = serde_json::from_str(&text).map_err(json_error)?;
        serde_json::to_string(&message).map_err(json_error)
    }

    #[wasm_bindgen(js_name = pollSignalsJson)]
    pub async fn poll_signals_json(&self, query_json: String) -> Result<String, JsValue> {
        let query: DiscoverySignalPollQuery =
            serde_json::from_str(&query_json).map_err(json_error)?;
        let path = format!(
            "/signals/{}?since={}&timeout_ms={}",
            encode_path_segment(&required_nonempty(query.peer_id, "peer_id")?),
            query.since.unwrap_or(0),
            query.timeout_ms.unwrap_or(0)
        );
        let text = self.fetch_success_text("GET", &path, None).await?;
        let list: DiscoverySignalList = serde_json::from_str(&text).map_err(json_error)?;
        serde_json::to_string(&list).map_err(json_error)
    }
}

impl DiscoveryDirectoryClient {
    async fn fetch_success_text(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String, JsValue> {
        let (status, text) = self.fetch_text(method, path, body).await?;
        ensure_success(status, &text)?;
        Ok(text)
    }

    async fn fetch_text(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, String), JsValue> {
        let global = js_sys::global();
        let fetch = Reflect::get(&global, &JsValue::from_str("fetch"))?
            .dyn_into::<Function>()
            .map_err(|_| js_error("DiscoveryDirectoryClient requires fetch"))?;
        let init = Object::new();
        Reflect::set(
            &init,
            &JsValue::from_str("method"),
            &JsValue::from_str(method),
        )?;
        if let Some(body) = body {
            let headers = Object::new();
            Reflect::set(
                &headers,
                &JsValue::from_str("content-type"),
                &JsValue::from_str("application/json"),
            )?;
            Reflect::set(&init, &JsValue::from_str("headers"), &headers)?;
            Reflect::set(&init, &JsValue::from_str("body"), &JsValue::from_str(body))?;
        }

        let url = format!("{}{}", self.base_url, path);
        let promise = fetch
            .call2(&global, &JsValue::from_str(&url), &init)?
            .dyn_into::<Promise>()
            .map_err(|_| js_error("fetch did not return a Promise"))?;
        let response = JsFuture::from(promise).await?;
        let status = Reflect::get(&response, &JsValue::from_str("status"))?
            .as_f64()
            .unwrap_or(0.0) as u16;
        let text_fn = Reflect::get(&response, &JsValue::from_str("text"))?
            .dyn_into::<Function>()
            .map_err(|_| js_error("fetch response did not expose text()"))?;
        let text_promise = text_fn
            .call0(&response)?
            .dyn_into::<Promise>()
            .map_err(|_| js_error("response.text() did not return a Promise"))?;
        let text = JsFuture::from(text_promise)
            .await?
            .as_string()
            .unwrap_or_default();
        Ok((status, text))
    }
}

#[wasm_bindgen(js_name = encodeBrowserProbeRequest)]
pub fn encode_browser_probe_request(nonce: String, payload: &[u8]) -> Result<Vec<u8>, JsValue> {
    serde_json::to_vec(&BrowserProbeRequest {
        nonce,
        payload: payload.to_vec(),
    })
    .map_err(json_error)
}

#[wasm_bindgen(js_name = decodeBrowserProbeResponse)]
pub fn decode_browser_probe_response(bytes: &[u8]) -> Result<String, JsValue> {
    let response: BrowserProbeResponse = serde_json::from_slice(bytes).map_err(json_error)?;
    serde_json::to_string(&response).map_err(json_error)
}

#[wasm_bindgen(js_name = encodeMessageEnvelopeBytes)]
pub fn encode_message_envelope_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let envelope: MessageEnvelopeJson = serde_json::from_str(json).map_err(json_error)?;
    Ok(MessageEnvelope {
        type_url: envelope.type_url,
        body: envelope.body,
        request_id: envelope.request_id,
    }
    .encode_to_vec())
}

#[wasm_bindgen(js_name = decodeMessageEnvelopeJson)]
pub fn decode_message_envelope_json(bytes: &[u8]) -> Result<String, JsValue> {
    let envelope = MessageEnvelope::decode(bytes).map_err(|err| js_error(err.to_string()))?;
    serde_json::to_string(&MessageEnvelopeJson {
        type_url: envelope.type_url,
        body: envelope.body,
        request_id: envelope.request_id,
    })
    .map_err(json_error)
}

#[wasm_bindgen(js_name = encodeJoinRequestBytes)]
pub fn encode_join_request_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    canonical_json_bytes(json)
}

#[wasm_bindgen(js_name = decodeJoinResponseJson)]
pub fn decode_join_response_json(bytes: &[u8]) -> Result<String, JsValue> {
    canonical_json_string(bytes)
}

#[wasm_bindgen(js_name = encodeCatalogRequestBytes)]
pub fn encode_catalog_request_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    canonical_json_bytes(json)
}

#[wasm_bindgen(js_name = decodeCatalogResponseJson)]
pub fn decode_catalog_response_json(bytes: &[u8]) -> Result<String, JsValue> {
    canonical_json_string(bytes)
}

#[wasm_bindgen(js_name = encodeStreamRequestBytes)]
pub fn encode_stream_request_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let request: StreamRequestJson = serde_json::from_str(json).map_err(json_error)?;
    Ok(StreamMessage::request(StreamRequest {
        sensor_id: request.sensor_id,
    })
    .encode_to_vec())
}

#[wasm_bindgen(js_name = encodeStreamAcceptBytes)]
pub fn encode_stream_accept_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let manifest: StreamManifestJson = serde_json::from_str(json).map_err(json_error)?;
    Ok(StreamMessage::accept(StreamManifest {
        sensor_id: manifest.sensor_id,
        sensor_hash: manifest.sensor_hash,
        clock_id: manifest.clock_id,
        clock_hash: manifest.clock_hash,
        frame_id: manifest.frame_id,
        frame_hash: manifest.frame_hash,
    })
    .encode_to_vec())
}

#[wasm_bindgen(js_name = encodeStreamEntryBytes)]
pub fn encode_stream_entry_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let entry: StreamEntryJson = serde_json::from_str(json).map_err(json_error)?;
    Ok(StreamMessage::entry(StreamEntry {
        timestamp_ns: entry.timestamp_ns,
        seq: entry.seq,
        payload: entry.payload,
    })
    .encode_to_vec())
}

#[wasm_bindgen(js_name = decodeStreamMessageJson)]
pub fn decode_stream_message_json(bytes: &[u8]) -> Result<String, JsValue> {
    let message = StreamMessage::decode(bytes).map_err(|err| js_error(err.to_string()))?;
    let value = match message
        .variant
        .ok_or_else(|| js_error("stream message has no variant"))?
    {
        stream_message::Variant::Request(request) => {
            json!({ "request": StreamRequestJson { sensor_id: request.sensor_id } })
        }
        stream_message::Variant::Accept(manifest) => {
            json!({ "accept": StreamManifestJson {
                sensor_id: manifest.sensor_id,
                sensor_hash: manifest.sensor_hash,
                clock_id: manifest.clock_id,
                clock_hash: manifest.clock_hash,
                frame_id: manifest.frame_id,
                frame_hash: manifest.frame_hash,
            } })
        }
        stream_message::Variant::Entry(entry) => {
            json!({ "entry": StreamEntryJson {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload: entry.payload,
            } })
        }
        stream_message::Variant::Decline(reason) => {
            json!({ "decline": decline_reason_json(reason) })
        }
        stream_message::Variant::EndOfStream(reason) => {
            json!({ "end_of_stream": end_reason_json(reason) })
        }
    };
    serde_json::to_string(&value).map_err(json_error)
}

#[derive(Serialize, Deserialize)]
struct MessageEnvelopeJson {
    type_url: String,
    #[serde(default)]
    body: Vec<u8>,
    request_id: String,
}

#[derive(Serialize, Deserialize)]
struct StreamRequestJson {
    sensor_id: String,
}

#[derive(Serialize, Deserialize)]
struct StreamManifestJson {
    sensor_id: String,
    sensor_hash: String,
    clock_id: String,
    clock_hash: String,
    frame_id: String,
    frame_hash: String,
}

#[derive(Serialize, Deserialize)]
struct StreamEntryJson {
    timestamp_ns: i64,
    seq: u64,
    payload: Vec<u8>,
}

#[derive(Deserialize)]
struct DiscoveryRegistration {
    name: String,
    manager_peer_id: Option<String>,
    manager_multiaddrs: Option<Vec<String>>,
    #[serde(default)]
    relay_multiaddrs: Option<Vec<String>>,
    peer_count: Option<u32>,
    mode: Option<String>,
}

#[derive(Deserialize)]
struct DiscoveryPeerQuery {
    name: Option<String>,
}

#[derive(Deserialize)]
struct DiscoveryNodeQuery {
    #[serde(rename = "type")]
    node_type: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct DiscoveryClusterList {
    clusters: Vec<DiscoveryClusterEntry>,
}

#[derive(Serialize, Deserialize)]
struct DiscoveryClusterEntry {
    name: String,
    manager_peer_id: String,
    manager_multiaddrs: Vec<String>,
    #[serde(default)]
    relay_multiaddrs: Vec<String>,
    peer_count: u32,
    created_ns: i64,
    last_liveness_check_ns: i64,
}

#[derive(Serialize, Deserialize)]
struct DiscoveryNodeList {
    nodes: Vec<DiscoveryNodeEntry>,
}

#[derive(Serialize, Deserialize)]
struct DiscoveryNodeEntry {
    peer_id: String,
    node_type: String,
    multiaddrs: Vec<String>,
    created_ns: i64,
    last_liveness_check_ns: i64,
}

#[derive(Deserialize)]
struct DiscoverySignalRequest {
    recipient_peer_id: String,
    from_peer_id: String,
    connection_id: String,
    kind: String,
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct DiscoverySignalPollQuery {
    peer_id: String,
    since: Option<u64>,
    timeout_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct DiscoverySignalList {
    messages: Vec<DiscoverySignalMessage>,
}

#[derive(Serialize, Deserialize)]
struct DiscoverySignalMessage {
    id: u64,
    recipient_peer_id: String,
    from_peer_id: String,
    connection_id: String,
    kind: String,
    payload: serde_json::Value,
    created_ns: i64,
}

fn required_string(value: Option<String>, field: &str) -> Result<String, JsValue> {
    value.ok_or_else(|| js_error(format!("missing string field `{field}`")))
}

fn required_vec(value: Option<Vec<String>>, field: &str) -> Result<Vec<String>, JsValue> {
    value.ok_or_else(|| js_error(format!("missing field `{field}`")))
}

fn required_nonempty(value: String, field: &str) -> Result<String, JsValue> {
    if value.is_empty() {
        Err(js_error(format!("missing string field `{field}`")))
    } else {
        Ok(value)
    }
}

fn normalize_cluster_entry(text: &str) -> Result<serde_json::Value, JsValue> {
    let entry: DiscoveryClusterEntry = serde_json::from_str(text).map_err(json_error)?;
    serde_json::to_value(entry).map_err(serde_value_error)
}

fn compact_json(value: &serde_json::Value) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(json_error)
}

fn ensure_success(status: u16, body: &str) -> Result<(), JsValue> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(js_error(format!("Discovery HTTP {status}: {body}")))
    }
}

fn encode_path_segment(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn canonical_json_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(json_error)?;
    serde_json::to_vec(&value).map_err(json_error)
}

fn canonical_json_string(bytes: &[u8]) -> Result<String, JsValue> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(json_error)?;
    serde_json::to_string(&value).map_err(json_error)
}

fn decline_reason_json(reason: DeclineReason) -> serde_json::Value {
    match reason.kind {
        Some(decline_reason::Kind::SensorNotFound(_)) => json!({ "kind": "sensor_not_found" }),
        Some(decline_reason::Kind::SensorUnavailable(_)) => {
            json!({ "kind": "sensor_unavailable" })
        }
        Some(decline_reason::Kind::ProducerShuttingDown(_)) => {
            json!({ "kind": "producer_shutting_down" })
        }
        Some(decline_reason::Kind::Other(other)) => {
            json!({ "kind": "other", "detail": other.detail })
        }
        None => json!({ "kind": "unspecified" }),
    }
}

fn end_reason_json(reason: EndReason) -> serde_json::Value {
    match reason.kind {
        Some(end_reason::Kind::SourceEnded(_)) => json!({ "kind": "source_ended" }),
        Some(end_reason::Kind::ProducerShuttingDown(_)) => {
            json!({ "kind": "producer_shutting_down" })
        }
        Some(end_reason::Kind::SessionEnded(_)) => json!({ "kind": "session_ended" }),
        Some(end_reason::Kind::ProducerError(error)) => {
            json!({ "kind": "producer_error", "detail": error.detail })
        }
        None => json!({ "kind": "unspecified" }),
    }
}

fn seed32(seed: &[u8]) -> Result<[u8; 32], JsValue> {
    if seed.len() != 32 {
        return Err(js_error(format!(
            "seed must be exactly 32 bytes, found {}",
            seed.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(seed);
    Ok(out)
}

fn json_error(err: serde_json::Error) -> JsValue {
    js_error(err.to_string())
}

fn serde_value_error(err: serde_json::Error) -> JsValue {
    js_error(err.to_string())
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    Error::new(message.as_ref()).into()
}
