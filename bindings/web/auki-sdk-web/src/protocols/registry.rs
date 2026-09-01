use auki_protocols::registry::{
    RegistryClient,
    v3::{ID, RegistryKind, RegistryListEntry},
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    AukiPeer,
    protocol_support::{js_context, js_error, parse_exact_target, peer_protocols, to_js_value},
};

/// Outbound Registry v3 client backed by the portable Rust protocol.
#[wasm_bindgen]
pub struct AukiRegistryClient {
    inner: RegistryClient,
}

#[wasm_bindgen]
impl AukiRegistryClient {
    /// Bind an outbound Registry client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiRegistryClient, JsValue> {
        Ok(Self {
            inner: RegistryClient::new(peer_protocols(peer, "Registry")?),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// List the current IDs and content hashes in one Registry namespace.
    #[wasm_bindgen(
        js_name = listExact,
        unchecked_return_type = "AukiRegistryListEntry[]"
    )]
    pub async fn list_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        #[wasm_bindgen(unchecked_param_type = "AukiRegistryKind")] kind: String,
    ) -> Result<JsValue, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let kind = parse_registry_kind(&kind)?;
        let entries = self
            .inner
            .list_exact(peer_id, route, kind)
            .await
            .map_err(|error| js_context("list Registry entries", error))?;
        registry_list_to_js(&entries)
    }

    /// Fetch one exact content-addressed entry after Rust validates its hash,
    /// authenticated owner, Registry ID, and typed schema.
    #[wasm_bindgen(js_name = fetchExact, unchecked_return_type = "AukiRegistryEntry")]
    pub async fn fetch_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        #[wasm_bindgen(unchecked_param_type = "AukiRegistryKind")] kind: String,
        id: String,
        hash: String,
    ) -> Result<JsValue, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let kind = parse_registry_kind(&kind)?;

        match kind {
            RegistryKind::Sensor => {
                let entry = self
                    .inner
                    .fetch_sensor_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Sensor Registry entry", error))?;
                registry_entry_to_js("convert Sensor Registry entry", &entry)
            }
            RegistryKind::Clock => {
                let entry = self
                    .inner
                    .fetch_clock_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Clock Registry entry", error))?;
                registry_entry_to_js("convert Clock Registry entry", &entry)
            }
            RegistryKind::Frame => {
                let entry = self
                    .inner
                    .fetch_frame_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Frame Registry entry", error))?;
                registry_entry_to_js("convert Frame Registry entry", &entry)
            }
            RegistryKind::Detector => {
                let entry = self
                    .inner
                    .fetch_detector_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Detector Registry entry", error))?;
                registry_entry_to_js("convert Detector Registry entry", &entry)
            }
            RegistryKind::Map => {
                let entry = self
                    .inner
                    .fetch_map_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Map Registry entry", error))?;
                registry_entry_to_js("convert Map Registry entry", &entry)
            }
            RegistryKind::DeviceModel => {
                let entry = self
                    .inner
                    .fetch_device_model_exact(peer_id, route, id, hash)
                    .await
                    .map_err(|error| js_context("fetch Device Model Registry entry", error))?;
                registry_entry_to_js("convert Device Model Registry entry", &entry)
            }
        }
    }
}

fn parse_registry_kind(kind: &str) -> Result<RegistryKind, JsValue> {
    match kind {
        "sensor" => Ok(RegistryKind::Sensor),
        "clock" => Ok(RegistryKind::Clock),
        "frame" => Ok(RegistryKind::Frame),
        "detector" => Ok(RegistryKind::Detector),
        "map" => Ok(RegistryKind::Map),
        "device_model" => Ok(RegistryKind::DeviceModel),
        _ => Err(js_error(format!(
            "unsupported Registry kind {kind:?}; expected sensor, clock, frame, detector, map, or device_model"
        ))),
    }
}

fn registry_list_to_js(entries: &[RegistryListEntry]) -> Result<JsValue, JsValue> {
    to_js_value("convert Registry list", &entries)
}

fn registry_entry_to_js(context: &'static str, entry: &impl Serialize) -> Result<JsValue, JsValue> {
    to_js_value(context, entry)
}

#[wasm_bindgen(typescript_custom_section)]
const REGISTRY_TYPESCRIPT: &str = r#"
/** Registry v3 namespace label. */
export type AukiRegistryKind =
    | "sensor"
    | "clock"
    | "frame"
    | "detector"
    | "map"
    | "device_model";

/** One content-addressed identity returned by Registry List. */
export interface AukiRegistryListEntry {
    readonly id: string;
    readonly hash: string;
}

/** Common owner field retained from canonical Registry JSON. */
export interface AukiRegistryEntryBase {
    readonly peer_id: string;
    readonly [field: string]: unknown;
}

export interface AukiSensorRegistryEntry extends AukiRegistryEntryBase {
    readonly sensor_id: string;
}

export interface AukiClockRegistryEntry extends AukiRegistryEntryBase {
    readonly session_id: string;
    readonly clock_id: string;
}

export interface AukiFrameRegistryEntry extends AukiRegistryEntryBase {
    readonly frame_id: string;
}

export interface AukiDetectorRegistryEntry extends AukiRegistryEntryBase {
    readonly detector_id: string;
}

export interface AukiMapRegistryEntry extends AukiRegistryEntryBase {
    readonly map_id: string;
}

export interface AukiDeviceModelRegistryEntry extends AukiRegistryEntryBase {
    readonly device_model_id: string;
}

/** Validated canonical Registry entry selected by `AukiRegistryKind`. */
export type AukiRegistryEntry =
    | AukiSensorRegistryEntry
    | AukiClockRegistryEntry
    | AukiFrameRegistryEntry
    | AukiDetectorRegistryEntry
    | AukiMapRegistryEntry
    | AukiDeviceModelRegistryEntry;
"#;

#[cfg(test)]
mod tests {
    use js_sys::{Array, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn parses_only_the_six_exact_snake_case_registry_kinds() {
        assert_eq!(parse_registry_kind("sensor").unwrap(), RegistryKind::Sensor);
        assert_eq!(parse_registry_kind("clock").unwrap(), RegistryKind::Clock);
        assert_eq!(parse_registry_kind("frame").unwrap(), RegistryKind::Frame);
        assert_eq!(
            parse_registry_kind("detector").unwrap(),
            RegistryKind::Detector
        );
        assert_eq!(parse_registry_kind("map").unwrap(), RegistryKind::Map);
        assert_eq!(
            parse_registry_kind("device_model").unwrap(),
            RegistryKind::DeviceModel
        );

        for invalid in ["deviceModel", "DeviceModel", "device-model", "unknown"] {
            let error = parse_registry_kind(invalid).unwrap_err();
            let message = String::from(js_sys::Error::from(error).message());
            assert!(message.contains("unsupported Registry kind"));
        }
    }

    #[wasm_bindgen_test]
    fn registry_lists_convert_to_plain_javascript_records() {
        let value = registry_list_to_js(&[
            RegistryListEntry {
                id: "camera".into(),
                hash: "a".repeat(32),
            },
            RegistryListEntry {
                id: "lidar".into(),
                hash: "b".repeat(32),
            },
        ])
        .unwrap();

        assert!(Array::is_array(&value));
        let rows = Array::from(&value);
        assert_eq!(rows.length(), 2);
        assert_eq!(
            Reflect::get(&rows.get(0), &JsValue::from_str("id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("camera")
        );
        assert_eq!(
            Reflect::get(&rows.get(1), &JsValue::from_str("hash"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[derive(Serialize)]
    struct RegistryEntryFixture {
        peer_id: &'static str,
        clock_id: &'static str,
        ticks: u64,
    }

    #[wasm_bindgen_test]
    fn typed_entries_convert_to_plain_records_without_losing_large_integers() {
        let value = registry_entry_to_js(
            "convert fixture",
            &RegistryEntryFixture {
                peer_id: "peer",
                clock_id: "clock",
                ticks: u64::MAX,
            },
        )
        .unwrap();

        assert!(!Array::is_array(&value));
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("clock_id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("clock")
        );
        assert!(
            Reflect::get(&value, &JsValue::from_str("ticks"))
                .unwrap()
                .is_bigint()
        );
    }
}
