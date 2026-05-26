use crate::{BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, core};
use auki_identity::Wallet;
use auki_proto::message::MessageEnvelope;
use js_sys::Error;
use prost::Message;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

const JOIN_PROTOCOL: &str = "/auki/join/0.0.1";
const RESOURCES_PROTOCOL: &str = "/auki/resources/0.0.1";
const SENSORS_PROTOCOL: &str = "/auki/sensors/0.0.1";

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

#[wasm_bindgen(js_name = aukiNetworkProtocolsJson)]
pub fn auki_network_protocols_json() -> Result<String, JsValue> {
    serde_json::to_string(&serde_json::json!({
        "browser_probe": BROWSER_PROBE_PROTOCOL,
        "message": core::MESSAGE_PROTOCOL,
        "join": JOIN_PROTOCOL,
        "resources": RESOURCES_PROTOCOL,
        "sensors": SENSORS_PROTOCOL,
    }))
    .map_err(json_error)
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

#[derive(Serialize, Deserialize)]
struct MessageEnvelopeJson {
    type_url: String,
    #[serde(default)]
    body: Vec<u8>,
    request_id: String,
}

fn canonical_json_bytes(json: &str) -> Result<Vec<u8>, JsValue> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(json_error)?;
    serde_json::to_vec(&value).map_err(json_error)
}

fn canonical_json_string(bytes: &[u8]) -> Result<String, JsValue> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(json_error)?;
    serde_json::to_string(&value).map_err(json_error)
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

fn js_error(message: impl AsRef<str>) -> JsValue {
    Error::new(message.as_ref()).into()
}
