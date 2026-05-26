use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = canonicalizeJson)]
pub fn canonicalize_json(json: String) -> Result<Vec<u8>, JsValue> {
    core::canonicalize_json_str(&json)
        .map_err(|err| JsValue::from_str(&format!("JSON is not valid: {err}")))
}
