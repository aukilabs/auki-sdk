use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = hashJcsBytes)]
pub fn hash_jcs_bytes(bytes: &[u8]) -> String {
    core::hash_jcs_bytes(bytes)
}
