use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = canonicalManifestJson)]
pub fn canonical_manifest_json(manifest_json: String) -> Result<String, JsValue> {
    core::canonical_manifest_json_str(&manifest_json).map_err(log_error)
}

#[wasm_bindgen(js_name = encodeSegmentEntriesJson)]
pub fn encode_segment_entries_json(
    start_ns: i64,
    entries_json: String,
) -> Result<Vec<u8>, JsValue> {
    let entries = core::bytes_entries_from_json(&entries_json).map_err(log_error)?;
    core::encode_segment_bytes(start_ns, &entries).map_err(log_error)
}

#[wasm_bindgen(js_name = decodeSegmentEntriesJson)]
pub fn decode_segment_entries_json(segment_bytes: Vec<u8>) -> Result<String, JsValue> {
    let entries = core::decode_segment_bytes(&segment_bytes).map_err(log_error)?;
    Ok(core::bytes_entries_to_json(&entries))
}

fn log_error(err: core::Error) -> JsValue {
    JsValue::from_str(&err.to_string())
}
