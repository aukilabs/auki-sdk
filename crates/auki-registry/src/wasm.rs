use crate::core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sensorEntryCanonicalJson)]
pub fn sensor_entry_canonical_json(entry_json: String) -> Result<String, JsValue> {
    let entry: core::SensorRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[wasm_bindgen(js_name = sensorEntryHash)]
pub fn sensor_entry_hash(entry_json: String) -> Result<String, JsValue> {
    let entry: core::SensorRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[wasm_bindgen(js_name = clockEntryCanonicalJson)]
pub fn clock_entry_canonical_json(entry_json: String) -> Result<String, JsValue> {
    let entry: core::ClockRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[wasm_bindgen(js_name = clockEntryHash)]
pub fn clock_entry_hash(entry_json: String) -> Result<String, JsValue> {
    let entry: core::ClockRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[wasm_bindgen(js_name = frameEntryCanonicalJson)]
pub fn frame_entry_canonical_json(entry_json: String) -> Result<String, JsValue> {
    let entry: core::FrameRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[wasm_bindgen(js_name = frameEntryHash)]
pub fn frame_entry_hash(entry_json: String) -> Result<String, JsValue> {
    let entry: core::FrameRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[wasm_bindgen(js_name = detectorEntryCanonicalJson)]
pub fn detector_entry_canonical_json(entry_json: String) -> Result<String, JsValue> {
    let entry: core::DetectorRegistryEntry = parse_json(&entry_json)?;
    Ok(utf8(entry.canonical_bytes()))
}

#[wasm_bindgen(js_name = detectorEntryHash)]
pub fn detector_entry_hash(entry_json: String) -> Result<String, JsValue> {
    let entry: core::DetectorRegistryEntry = parse_json(&entry_json)?;
    Ok(entry.hash())
}

#[wasm_bindgen(js_name = frameRosBodyJson)]
pub fn frame_ros_body_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::ros_body(frame_id).canonical_bytes())
}

#[wasm_bindgen(js_name = frameRosOpticalJson)]
pub fn frame_ros_optical_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::ros_optical(frame_id).canonical_bytes())
}

#[wasm_bindgen(js_name = frameOpenglJson)]
pub fn frame_opengl_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::opengl(frame_id).canonical_bytes())
}

#[wasm_bindgen(js_name = frameUnityJson)]
pub fn frame_unity_json(frame_id: String) -> String {
    utf8(core::FrameRegistryEntry::unity(frame_id).canonical_bytes())
}

fn parse_json<T>(json: &str) -> Result<T, JsValue>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(json)
        .map_err(|err| JsValue::from_str(&format!("JSON is not valid: {err}")))
}

fn utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("JCS output is valid UTF-8")
}
