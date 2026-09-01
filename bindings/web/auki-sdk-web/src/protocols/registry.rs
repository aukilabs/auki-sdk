use std::cell::RefCell;

use auki_protocols::registry::{
    RegistryClient, RegistryEndpoint, RegistryProvider,
    v3::{ID, RegistryKind, RegistryListEntry, RegistryRequest, RegistryResponse},
};
use auki_sdk::AuthenticatedPeer;
use js_sys::{Function, Promise};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, javascript_error_reason, js_context, js_error,
        parse_exact_target, peer_protocols, to_js_value,
    },
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

/// Mounted inbound Registry v3 service backed by one synchronous JavaScript provider.
#[wasm_bindgen]
pub struct AukiRegistryEndpoint {
    inner: RefCell<Option<RegistryEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiRegistryEndpoint {
    /// Mount Registry v3 on one running browser peer.
    ///
    /// The provider receives already-authenticated requester metadata and one
    /// validated canonical Registry request. It must return synchronously;
    /// asynchronous storage belongs behind Blob or another application layer.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(
        peer: &AukiPeer,
        #[wasm_bindgen(unchecked_param_type = "AukiRegistryProvider")] provider: Function,
    ) -> Result<AukiRegistryEndpoint, JsValue> {
        let endpoint = RegistryEndpoint::mount(
            peer_protocols(peer, "Registry endpoint")?,
            JavaScriptRegistryProvider { provider },
        )
        .map_err(|error| js_context("mount Registry endpoint", error))?;
        Ok(Self {
            inner: RefCell::new(Some(endpoint)),
            closing: CloseBarrier::default(),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this endpoint.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Idempotently stop accepting requests and await every admitted handler.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Registry endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

struct JavaScriptRegistryProvider {
    provider: Function,
}

impl RegistryProvider for JavaScriptRegistryProvider {
    fn respond(
        &self,
        requester: &AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        invoke_registry_provider(&self.provider, requester, request).unwrap_or_else(|reason| {
            RegistryResponse::Error {
                reason: format!("Registry provider failed: {reason}"),
            }
        })
    }
}

fn invoke_registry_provider(
    provider: &Function,
    requester: &AuthenticatedPeer,
    request: &RegistryRequest,
) -> Result<RegistryResponse, String> {
    let requester = authenticated_peer_to_js("convert authenticated Registry requester", requester)
        .map_err(|error| javascript_error_reason(&error))?;
    let request = to_js_value("convert Registry provider request", request)
        .map_err(|error| javascript_error_reason(&error))?;
    let response = provider
        .call2(&JsValue::UNDEFINED, &requester, &request)
        .map_err(|error| javascript_error_reason(&error))?;
    registry_response_from_js(response)
}

fn registry_response_from_js(value: JsValue) -> Result<RegistryResponse, String> {
    serde_wasm_bindgen::from_value(value).map_err(|error| format!("invalid response: {error}"))
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

/** Validated canonical request delivered to an inbound Registry provider. */
export type AukiRegistryProviderRequest =
    | {
          readonly op: "get";
          readonly kind: AukiRegistryKind;
          readonly id: string;
          readonly hash: string;
      }
    | {
          readonly op: "list";
          readonly kind: AukiRegistryKind;
      };

/** Exact content-addressed envelope returned by an inbound Registry provider. */
export interface AukiRegistryEntryEnvelope {
    readonly kind: AukiRegistryKind;
    readonly id: string;
    readonly hash: string;
    readonly canonical_json: string;
}

/** Canonical Registry response returned synchronously by a provider. */
export type AukiRegistryProviderResponse =
    | {
          readonly op: "get";
          readonly entry: AukiRegistryEntryEnvelope | null;
      }
    | {
          readonly op: "list";
          readonly entries: readonly AukiRegistryListEntry[];
      }
    | {
          readonly op: "error";
          readonly reason: string;
      };

export type AukiRegistryProvider = (
    requester: AukiAuthenticatedPeer,
    request: AukiRegistryProviderRequest,
) => AukiRegistryProviderResponse;
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

    #[wasm_bindgen_test]
    fn registry_provider_values_preserve_the_canonical_wire_shape() {
        let request = RegistryRequest::get(
            RegistryKind::Sensor,
            "camera",
            "0123456789abcdef0123456789abcdef",
        );
        let value = to_js_value("convert Registry request fixture", &request).unwrap();
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("op"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("get")
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("kind"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("sensor")
        );

        let expected = RegistryResponse::List {
            entries: vec![RegistryListEntry {
                id: "camera".into(),
                hash: "0123456789abcdef0123456789abcdef".into(),
            }],
        };
        let value = to_js_value("convert Registry response fixture", &expected).unwrap();
        let actual = registry_response_from_js(value).unwrap();
        assert_eq!(actual, expected);
    }

    #[wasm_bindgen_test]
    fn invalid_registry_provider_values_are_remote_safe_errors() {
        let invalid = js_sys::Object::new();
        let error = registry_response_from_js(invalid.into()).unwrap_err();
        assert!(error.contains("invalid response"));
    }
}
