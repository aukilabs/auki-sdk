use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sdkName)]
pub fn sdk_name() -> String {
    "auki-network-browser-wasm".to_string()
}
