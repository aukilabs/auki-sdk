use crate::{BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, core};
use auki_identity::Wallet;
use js_sys::Error;
use wasm_bindgen::prelude::*;

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
