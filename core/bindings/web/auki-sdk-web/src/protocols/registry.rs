use std::cell::RefCell;

use auki_protocols::registry::{
    RegistryClient, RegistryEndpoint, RegistryProvider,
    v3::{
        ID, RegistryEntryEnvelope, RegistryKind, RegistryListEntry, RegistryRequest,
        RegistryResponse,
    },
};
use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorBody, SensorRegistryEntry,
};
use auki_sdk::{AuthenticatedPeer, PeerId};
use js_sys::{Function, Promise};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, javascript_error_reason, js_context, js_error,
        parse_exact_target, peer_protocols, to_js_value,
    },
};

/// Validate and content-address one typed Registry entry in Rust.
///
/// The returned envelope derives its owner, Registry ID, canonical JSON, and
/// XXH3-128 hash from the decoded entry. JavaScript supplies no parallel
/// envelope metadata that could disagree with those typed fields.
#[wasm_bindgen(
    js_name = prepareRegistryEntry,
    unchecked_return_type = "AukiRegistryEntryEnvelope"
)]
pub fn prepare_registry_entry(
    #[wasm_bindgen(unchecked_param_type = "AukiRegistryKind")] kind: String,
    #[wasm_bindgen(unchecked_param_type = "AukiRegistryEntry")] entry: JsValue,
) -> Result<JsValue, JsValue> {
    let envelope = match parse_registry_kind(&kind)? {
        RegistryKind::Sensor => prepare_typed_registry_entry::<SensorRegistryEntry>(entry),
        RegistryKind::Clock => prepare_typed_registry_entry::<ClockRegistryEntry>(entry),
        RegistryKind::Frame => prepare_typed_registry_entry::<FrameRegistryEntry>(entry),
        RegistryKind::Detector => prepare_typed_registry_entry::<DetectorRegistryEntry>(entry),
        RegistryKind::Map => prepare_typed_registry_entry::<MapRegistryEntry>(entry),
        RegistryKind::DeviceModel => {
            prepare_typed_registry_entry::<DeviceModelRegistryEntry>(entry)
        }
    }?;
    to_js_value("convert prepared Registry entry", &envelope)
}

trait PreparedRegistryEntry: DeserializeOwned {
    const KIND: RegistryKind;

    fn owner_peer_id(&self) -> &str;
    fn registry_id(&self) -> &str;
    fn canonical_bytes(&self) -> Vec<u8>;
    fn hash(&self) -> String;

    fn validate_entry(&self) -> Result<(), String> {
        Ok(())
    }
}

fn prepare_typed_registry_entry<T>(entry: JsValue) -> Result<RegistryEntryEnvelope, JsValue>
where
    T: PreparedRegistryEntry,
{
    let entry: T = serde_wasm_bindgen::from_value(entry).map_err(|error| {
        js_context(
            "prepare Registry entry",
            format!("invalid {} entry: {error}", T::KIND),
        )
    })?;
    entry.owner_peer_id().parse::<PeerId>().map_err(|error| {
        js_context(
            "prepare Registry entry",
            format!("invalid {} peer_id: {error}", T::KIND),
        )
    })?;
    auki_registry::validate_registry_id(entry.registry_id()).map_err(|error| {
        js_context(
            "prepare Registry entry",
            format!("invalid {} id: {error}", T::KIND),
        )
    })?;
    entry
        .validate_entry()
        .map_err(|error| js_context("prepare Registry entry", error))?;

    let canonical_json = String::from_utf8(entry.canonical_bytes()).map_err(|error| {
        js_context(
            "prepare Registry entry",
            format!("{} canonical JSON is not UTF-8: {error}", T::KIND),
        )
    })?;
    Ok(RegistryEntryEnvelope {
        kind: T::KIND,
        id: entry.registry_id().to_owned(),
        hash: entry.hash(),
        canonical_json,
    })
}

impl PreparedRegistryEntry for SensorRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Sensor;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.sensor_id
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes()
    }

    fn hash(&self) -> String {
        self.hash()
    }

    fn validate_entry(&self) -> Result<(), String> {
        match &self.body {
            SensorBody::Camera(camera) => camera
                .validate_image_layout()
                .and_then(|()| camera.validate_calibration()),
            SensorBody::Scalar(scalar) => scalar.validate(),
            _ => Ok(()),
        }
        .map_err(|error| error.to_string())
    }
}

macro_rules! prepared_registry_entry {
    ($entry:ty, $kind:expr, $owner:ident, $id:ident) => {
        impl PreparedRegistryEntry for $entry {
            const KIND: RegistryKind = $kind;

            fn owner_peer_id(&self) -> &str {
                &self.$owner
            }

            fn registry_id(&self) -> &str {
                &self.$id
            }

            fn canonical_bytes(&self) -> Vec<u8> {
                self.canonical_bytes()
            }

            fn hash(&self) -> String {
                self.hash()
            }
        }
    };
}

prepared_registry_entry!(ClockRegistryEntry, RegistryKind::Clock, peer_id, clock_id);
prepared_registry_entry!(
    DetectorRegistryEntry,
    RegistryKind::Detector,
    peer_id,
    detector_id
);

impl PreparedRegistryEntry for FrameRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Frame;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.frame_id
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes()
    }

    fn hash(&self) -> String {
        self.hash()
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

impl PreparedRegistryEntry for MapRegistryEntry {
    const KIND: RegistryKind = RegistryKind::Map;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.map_id
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes()
    }

    fn hash(&self) -> String {
        self.hash()
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

impl PreparedRegistryEntry for DeviceModelRegistryEntry {
    const KIND: RegistryKind = RegistryKind::DeviceModel;

    fn owner_peer_id(&self) -> &str {
        &self.peer_id
    }

    fn registry_id(&self) -> &str {
        &self.device_model_id
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes()
    }

    fn hash(&self) -> String {
        self.hash()
    }

    fn validate_entry(&self) -> Result<(), String> {
        self.validate().map_err(|error| error.to_string())
    }
}

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
    use auki_registry::{
        AxisConvention, AxisDirection, Camera, ClockBody, ClockMeta, DetectorBody, DeviceModelBody,
        DeviceModelFormat, FiniteF64, Handedness, LengthUnit, MapBody, Qr, RegistryRef, Scope,
        VoxelMap, VoxelValueModel,
    };
    use js_sys::{Array, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    const PEER_ID: &str = "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan";

    fn prepared<T: Serialize>(kind: &str, entry: &T) -> RegistryEntryEnvelope {
        let value = serde_wasm_bindgen::to_value(entry).unwrap();
        let prepared = prepare_registry_entry(kind.to_owned(), value).unwrap();
        serde_wasm_bindgen::from_value(prepared).unwrap()
    }

    fn camera_sensor() -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: PEER_ID.into(),
            sensor_id: "camera/front".into(),
            body: SensorBody::Camera(Camera {
                r#type: "rgb".into(),
                width: 320,
                height: 240,
                frame_rate_hz: 5,
                image_encoding: "jpeg".into(),
                pixel_format: "rgb8".into(),
                row_stride_bytes: 0,
                color_space: "srgb".into(),
                intrinsics_model: "none".into(),
                distortion_model: "none".into(),
                calibration: None,
                frame: RegistryRef {
                    peer_id: PEER_ID.into(),
                    id: "camera/front/optical".into(),
                    hash: "0".repeat(32),
                },
            }),
        }
    }

    #[wasm_bindgen_test]
    fn prepares_all_six_typed_registry_kinds() {
        let sensor = camera_sensor();
        let clock = ClockRegistryEntry {
            peer_id: PEER_ID.into(),
            session_id: "browser-session".into(),
            clock_id: "session/monotonic".into(),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "nanoseconds".into(),
                monotonic: true,
                epoch: None,
                scope: Scope::DeviceLocal,
            }),
        };
        let frame = FrameRegistryEntry::ros_optical(PEER_ID, "camera/front/optical");
        let detector = DetectorRegistryEntry {
            peer_id: PEER_ID.into(),
            detector_id: "qr".into(),
            body: DetectorBody::Qr(Qr {}),
            input_types: Vec::new(),
            output_types: vec!["qr".into()],
        };
        let map = MapRegistryEntry {
            peer_id: PEER_ID.into(),
            map_id: "map/world".into(),
            body: MapBody::Voxel(VoxelMap {
                frame: RegistryRef {
                    peer_id: PEER_ID.into(),
                    id: "world".into(),
                    hash: "1".repeat(32),
                },
                voxel_size_m: FiniteF64(0.1),
                chunk_dimension: 16,
                value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                color_model: None,
                semantic_classes: Vec::new(),
            }),
        };
        let device_model = DeviceModelRegistryEntry {
            peer_id: PEER_ID.into(),
            device_model_id: "browser-camera".into(),
            body: DeviceModelBody {
                model_id: "browser-camera".into(),
                format: DeviceModelFormat::Urdf {
                    urdf_sha256: "2".repeat(64),
                    meshes: Vec::new(),
                },
                root_convention: None,
            },
        };

        let cases = [
            ("sensor", prepared("sensor", &sensor), sensor.hash()),
            ("clock", prepared("clock", &clock), clock.hash()),
            ("frame", prepared("frame", &frame), frame.hash()),
            ("detector", prepared("detector", &detector), detector.hash()),
            ("map", prepared("map", &map), map.hash()),
            (
                "device_model",
                prepared("device_model", &device_model),
                device_model.hash(),
            ),
        ];

        for (kind, envelope, expected_hash) in cases {
            assert_eq!(envelope.kind.as_str(), kind);
            assert_eq!(envelope.hash, expected_hash);
            assert_eq!(envelope.hash.len(), 32);
            assert!(envelope.hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        assert_eq!(prepared("sensor", &sensor).id, sensor.sensor_id);
        assert_eq!(prepared("clock", &clock).id, clock.clock_id);
        assert_eq!(prepared("frame", &frame).id, frame.frame_id);
        assert_eq!(prepared("detector", &detector).id, detector.detector_id);
        assert_eq!(prepared("map", &map).id, map.map_id);
        assert_eq!(
            prepared("device_model", &device_model).id,
            device_model.device_model_id
        );
    }

    #[wasm_bindgen_test]
    fn prepared_envelope_uses_the_typed_entry_identity_and_canonical_json() {
        let sensor = camera_sensor();
        let envelope = prepared("sensor", &sensor);

        assert_eq!(envelope.kind, RegistryKind::Sensor);
        assert_eq!(envelope.id, "camera/front");
        assert_eq!(envelope.hash, sensor.hash());
        assert_eq!(
            envelope.canonical_json.as_bytes(),
            sensor.canonical_bytes().as_slice()
        );
        let decoded: SensorRegistryEntry =
            serde_wasm_bindgen::from_value(js_sys::JSON::parse(&envelope.canonical_json).unwrap())
                .unwrap();
        assert_eq!(decoded.peer_id, PEER_ID);
        assert_eq!(decoded.sensor_id, envelope.id);
    }

    #[wasm_bindgen_test]
    fn rejects_kind_shape_mismatches_and_invalid_typed_semantics() {
        let sensor = camera_sensor();
        let wrong_kind = prepare_registry_entry(
            "frame".into(),
            serde_wasm_bindgen::to_value(&sensor).unwrap(),
        )
        .unwrap_err();
        assert!(
            String::from(js_sys::Error::from(wrong_kind).message()).contains("invalid frame entry")
        );

        let mut invalid_owner = sensor.clone();
        invalid_owner.peer_id = "not-a-peer-id".into();
        let invalid_owner = prepare_registry_entry(
            "sensor".into(),
            serde_wasm_bindgen::to_value(&invalid_owner).unwrap(),
        )
        .unwrap_err();
        assert!(
            String::from(js_sys::Error::from(invalid_owner).message())
                .contains("invalid sensor peer_id")
        );

        let mut invalid_id = sensor.clone();
        invalid_id.sensor_id = "camera front".into();
        let invalid_id = prepare_registry_entry(
            "sensor".into(),
            serde_wasm_bindgen::to_value(&invalid_id).unwrap(),
        )
        .unwrap_err();
        assert!(
            String::from(js_sys::Error::from(invalid_id).message()).contains("invalid sensor id")
        );

        let mut invalid_camera = sensor;
        let SensorBody::Camera(camera) = &mut invalid_camera.body else {
            unreachable!()
        };
        camera.width = 0;
        let invalid_camera = prepare_registry_entry(
            "sensor".into(),
            serde_wasm_bindgen::to_value(&invalid_camera).unwrap(),
        )
        .unwrap_err();
        assert!(
            String::from(js_sys::Error::from(invalid_camera).message())
                .contains("invalid image layout")
        );

        let invalid_frame = FrameRegistryEntry {
            peer_id: PEER_ID.into(),
            frame_id: "camera/front/optical".into(),
            handedness: Handedness::Right,
            axes: AxisConvention {
                x: AxisDirection::Forward,
                y: AxisDirection::Backward,
                z: AxisDirection::Up,
            },
            units: LengthUnit::Meters,
        };
        let invalid_frame = prepare_registry_entry(
            "frame".into(),
            serde_wasm_bindgen::to_value(&invalid_frame).unwrap(),
        )
        .unwrap_err();
        assert!(
            String::from(js_sys::Error::from(invalid_frame).message()).contains("invalid axes")
        );
    }

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
