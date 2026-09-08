//! Wasm binding for the portable Catalog v3 resources and v4 maps client.

use std::cell::RefCell;

use auki_protocols::catalog::{CatalogClient, CatalogEndpoint, CatalogProvider, v3, v4};
use auki_sdk::AuthenticatedPeer;
use js_sys::{Function, Promise};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

#[cfg(not(test))]
use crate::protocol_support::{javascript_error_reason, js_error};
use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, js_context, parse_exact_target, peer_protocols,
        to_js_value,
    },
};

#[cfg(not(test))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(value: &JsValue);
}

#[cfg(not(test))]
fn report_local_catalog_error(context: &str, error: &JsValue) {
    console_error(&js_error(format!(
        "{context}: {}",
        javascript_error_reason(error)
    )));
}

#[cfg(test)]
fn report_local_catalog_error(_context: &str, _error: &JsValue) {}

const RESOURCE_VARIANT_NAMES: &str =
    "sensor_log, pose_log, time_transform_log, detection_log, or message_channel";

/// Validate and normalize one Catalog v3 provider snapshot in Rust.
///
/// Preparing static snapshots when an application mounts its endpoint turns
/// schema mistakes into local startup errors instead of an empty, fail-closed
/// response observed only by a remote peer.
#[wasm_bindgen(
    js_name = prepareCatalogResources,
    unchecked_return_type = "AukiCatalogResourcesResponse"
)]
pub fn prepare_catalog_resources(
    #[wasm_bindgen(unchecked_param_type = "AukiCatalogResourcesResponse")] response: JsValue,
) -> Result<JsValue, JsValue> {
    let response = resources_from_js(response)?;
    resources_to_js(&response)
}

/// Outbound Catalog v3/v4 client backed by the portable Rust protocols.
#[wasm_bindgen]
pub struct AukiCatalogClient {
    inner: CatalogClient,
}

#[wasm_bindgen]
impl AukiCatalogClient {
    /// Bind an outbound Catalog client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiCatalogClient, JsValue> {
        Ok(Self {
            inner: CatalogClient::new(peer_protocols(peer, "Catalog")?),
        })
    }

    /// Immutable Catalog v3 resources protocol identifier.
    #[wasm_bindgen(getter, js_name = resourceProtocol)]
    pub fn resource_protocol(&self) -> String {
        v3::ID.to_owned()
    }

    /// Immutable Catalog v4 maps protocol identifier.
    #[wasm_bindgen(getter, js_name = mapsProtocol)]
    pub fn maps_protocol(&self) -> String {
        v4::ID.to_owned()
    }

    /// Fetch Catalog v3 resources through one exact advertised route.
    ///
    /// An empty variant list requests every resource family.
    #[wasm_bindgen(
        js_name = fetchResourcesExact,
        unchecked_return_type = "AukiCatalogResourcesResponse"
    )]
    pub async fn fetch_resources_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        #[wasm_bindgen(unchecked_param_type = "AukiCatalogResourceVariant[]")] variants: Vec<
            String,
        >,
    ) -> Result<JsValue, JsValue> {
        let request = resources_request(variants)?;
        let (peer_id, route) = parse_exact_target(target)?;
        let response = self
            .inner
            .fetch_resources_exact(peer_id, route, request)
            .await
            .map_err(|error| js_context("fetch Catalog resources", error))?;
        resources_to_js(&response)
    }

    /// Fetch Catalog v4 Map Log resources through one exact advertised route.
    #[wasm_bindgen(
        js_name = fetchMapsExact,
        unchecked_return_type = "AukiCatalogMapsResponse"
    )]
    pub async fn fetch_maps_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
    ) -> Result<JsValue, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let response = self
            .inner
            .fetch_maps_exact(peer_id, route)
            .await
            .map_err(|error| js_context("fetch Catalog maps", error))?;
        maps_to_js(&response)
    }
}

/// Mounted inbound Catalog v3 resources and v4 maps services.
#[wasm_bindgen]
pub struct AukiCatalogEndpoint {
    inner: RefCell<Option<CatalogEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiCatalogEndpoint {
    /// Mount Catalog v3 and v4 on one running browser peer.
    ///
    /// Each configured provider is sampled synchronously for every mutually
    /// authenticated requester. A missing provider or a `null`/`undefined`
    /// result advertises an empty snapshot for that Catalog version.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(
        peer: &AukiPeer,
        #[wasm_bindgen(unchecked_param_type = "AukiCatalogResourcesProvider | null | undefined")]
        resources_provider: Option<Function>,
        #[wasm_bindgen(unchecked_param_type = "AukiCatalogMapsProvider | null | undefined")]
        maps_provider: Option<Function>,
    ) -> Result<AukiCatalogEndpoint, JsValue> {
        let endpoint = CatalogEndpoint::mount(
            peer_protocols(peer, "Catalog endpoint")?,
            JavaScriptCatalogProvider {
                resources: resources_provider,
                maps: maps_provider,
            },
        )
        .map_err(|error| js_context("mount Catalog endpoint", error))?;
        Ok(Self {
            inner: RefCell::new(Some(endpoint)),
            closing: CloseBarrier::default(),
        })
    }

    /// Immutable Catalog v3 resources protocol identifier.
    #[wasm_bindgen(getter, js_name = resourceProtocol)]
    pub fn resource_protocol(&self) -> String {
        v3::ID.to_owned()
    }

    /// Immutable Catalog v4 maps protocol identifier.
    #[wasm_bindgen(getter, js_name = mapsProtocol)]
    pub fn maps_protocol(&self) -> String {
        v4::ID.to_owned()
    }

    /// Idempotently stop both Catalog versions and await admitted handlers.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Catalog endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

struct JavaScriptCatalogProvider {
    resources: Option<Function>,
    maps: Option<Function>,
}

impl CatalogProvider for JavaScriptCatalogProvider {
    fn resources(
        &self,
        requester: &AuthenticatedPeer,
        request: &v3::ResourcesRequest,
    ) -> v3::ResourcesResponse {
        let Some(callback) = self.resources.as_ref() else {
            return empty_resources();
        };
        match invoke_resources_provider(callback, requester, request) {
            Ok(response) => response,
            Err(error) => {
                report_local_catalog_error("Catalog resources provider failed", &error);
                empty_resources()
            }
        }
    }

    fn maps(&self, requester: &AuthenticatedPeer) -> v4::ResourcesResponse {
        let Some(callback) = self.maps.as_ref() else {
            return empty_maps();
        };
        match invoke_maps_provider(callback, requester) {
            Ok(response) => response,
            Err(error) => {
                report_local_catalog_error("Catalog maps provider failed", &error);
                empty_maps()
            }
        }
    }
}

fn invoke_resources_provider(
    callback: &Function,
    requester: &AuthenticatedPeer,
    request: &v3::ResourcesRequest,
) -> Result<v3::ResourcesResponse, JsValue> {
    let requester = authenticated_peer_to_js("convert authenticated Catalog requester", requester)?;
    let request = to_js_value("convert Catalog resources request", request)?;
    let value = callback.call2(&JsValue::UNDEFINED, &requester, &request)?;
    resources_from_js(value)
}

fn invoke_maps_provider(
    callback: &Function,
    requester: &AuthenticatedPeer,
) -> Result<v4::ResourcesResponse, JsValue> {
    let requester = authenticated_peer_to_js("convert authenticated Catalog requester", requester)?;
    let value = callback.call1(&JsValue::UNDEFINED, &requester)?;
    maps_from_js(value)
}

fn resources_request(variants: Vec<String>) -> Result<v3::ResourcesRequest, JsValue> {
    let variants = variants
        .into_iter()
        .map(|variant| match variant.as_str() {
            "sensor_log" => Ok(v3::ResourceVariant::SensorLog),
            "pose_log" => Ok(v3::ResourceVariant::PoseLog),
            "time_transform_log" => Ok(v3::ResourceVariant::TimeTransformLog),
            "detection_log" => Ok(v3::ResourceVariant::DetectionLog),
            "message_channel" => Ok(v3::ResourceVariant::MessageChannel),
            _ => Err(js_context(
                "parse Catalog resource variant",
                format!("expected {RESOURCE_VARIANT_NAMES}, got {variant:?}"),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let request = v3::ResourcesRequest { variants };
    request
        .validate()
        .map_err(|error| js_context("validate Catalog resource variants", error))?;
    Ok(request)
}

fn resources_to_js(response: &v3::ResourcesResponse) -> Result<JsValue, JsValue> {
    to_js_value("convert Catalog resources", response)
}

fn resources_from_js(value: JsValue) -> Result<v3::ResourcesResponse, JsValue> {
    let response: v3::ResourcesResponse = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read Catalog resources", error))?;
    response
        .validate()
        .map_err(|error| js_context("validate Catalog resources", error))?;
    Ok(response)
}

fn maps_to_js(response: &v4::ResourcesResponse) -> Result<JsValue, JsValue> {
    to_js_value("convert Catalog maps", response)
}

fn maps_from_js(value: JsValue) -> Result<v4::ResourcesResponse, JsValue> {
    let response: v4::ResourcesResponse = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read Catalog maps", error))?;
    response
        .validate()
        .map_err(|error| js_context("validate Catalog maps", error))?;
    Ok(response)
}

fn empty_resources() -> v3::ResourcesResponse {
    v3::ResourcesResponse {
        resources: Vec::new(),
    }
}

fn empty_maps() -> v4::ResourcesResponse {
    v4::ResourcesResponse {
        resources: Vec::new(),
    }
}

#[wasm_bindgen(typescript_custom_section)]
const CATALOG_TYPESCRIPT: &str = r#"
/** Exact Catalog v3 resource discriminator. */
export type AukiCatalogResourceVariant =
    | "sensor_log"
    | "pose_log"
    | "time_transform_log"
    | "detection_log"
    | "message_channel";

/** One validated Catalog v3 row in its canonical protocol shape. */
export interface AukiCatalogResource {
    readonly variant: AukiCatalogResourceVariant;
    readonly [field: string]: unknown;
}

export interface AukiCatalogResourcesResponse {
    readonly resources: readonly AukiCatalogResource[];
}

export interface AukiCatalogResourcesRequest {
    readonly variants: readonly AukiCatalogResourceVariant[];
}

export interface AukiCatalogRegistryRef {
    readonly peer_id: string;
    readonly id: string;
    readonly hash: string;
}

export interface AukiCatalogMapResource {
    readonly source_peer_id: string;
    readonly writer_peer_id: string;
    readonly resource_id: string;
    readonly map: AukiCatalogRegistryRef;
    readonly clock: AukiCatalogRegistryRef;
}

export interface AukiCatalogMapsResponse {
    readonly resources: readonly AukiCatalogMapResource[];
}

/** Synchronous Catalog v3 snapshot sampled once per authenticated request. */
export type AukiCatalogResourcesProvider = (
    requester: AukiAuthenticatedPeer,
    request: AukiCatalogResourcesRequest,
) => AukiCatalogResourcesResponse | null | undefined;

/** Synchronous Catalog v4 snapshot sampled once per authenticated request. */
export type AukiCatalogMapsProvider = (
    requester: AukiAuthenticatedPeer,
) => AukiCatalogMapsResponse | null | undefined;
"#;

#[cfg(test)]
mod tests {
    use js_sys::{Array, Object, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    fn authenticated_peer() -> AuthenticatedPeer {
        AuthenticatedPeer {
            peer_id: peer_id(),
            subject: "b03a67cb-45d4-4f60-a8b8-d9687e91d018".parse().unwrap(),
            peer_type: Some("robot".into()),
            domain_ids: vec!["4e990513-b110-467b-84ca-09a42d786f6d".parse().unwrap()],
            scopes: vec!["catalog:read".into()],
            application: None,
            verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    fn registry_ref(peer_id: &str, id: &str, hash: &str) -> JsValue {
        let value = Object::new();
        Reflect::set(
            &value,
            &JsValue::from_str("peer_id"),
            &JsValue::from_str(peer_id),
        )
        .unwrap();
        Reflect::set(&value, &JsValue::from_str("id"), &JsValue::from_str(id)).unwrap();
        Reflect::set(&value, &JsValue::from_str("hash"), &JsValue::from_str(hash)).unwrap();
        value.into()
    }

    #[wasm_bindgen_test]
    fn resource_variants_accept_only_the_five_exact_wire_names() {
        let names = [
            "sensor_log",
            "pose_log",
            "time_transform_log",
            "detection_log",
            "message_channel",
        ];
        let request = resources_request(names.iter().map(ToString::to_string).collect()).unwrap();
        assert_eq!(
            request.variants,
            vec![
                v3::ResourceVariant::SensorLog,
                v3::ResourceVariant::PoseLog,
                v3::ResourceVariant::TimeTransformLog,
                v3::ResourceVariant::DetectionLog,
                v3::ResourceVariant::MessageChannel,
            ]
        );
        assert!(resources_request(Vec::new()).unwrap().variants.is_empty());

        let error = resources_request(vec!["SensorLog".into()]).unwrap_err();
        assert!(
            js_sys::Error::from(error)
                .message()
                .as_string()
                .unwrap()
                .contains("sensor_log")
        );
        let duplicate = resources_request(vec!["sensor_log".into(), "sensor_log".into()]);
        assert!(duplicate.is_err());
    }

    #[wasm_bindgen_test]
    fn catalog_responses_are_plain_protocol_records() {
        let owner = peer_id();
        let response = v3::ResourcesResponse {
            resources: vec![v3::ResourceEntry::MessageChannel(
                v3::MessageChannelResource {
                    owner_peer_id: owner,
                    resource_id: "events".into(),
                    clock: serde_wasm_bindgen::from_value(registry_ref(
                        &owner.to_string(),
                        "clock",
                        "clock-hash",
                    ))
                    .unwrap(),
                },
            )],
        };
        let value = resources_to_js(&response).unwrap();
        let resources =
            Array::from(&Reflect::get(&value, &JsValue::from_str("resources")).unwrap());
        assert_eq!(resources.length(), 1);
        let row = resources.get(0);
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("variant"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("message_channel")
        );
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("resource_id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("events")
        );

        let maps = v4::ResourcesResponse {
            resources: vec![v4::MapLogResource {
                source_peer_id: owner.to_string(),
                writer_peer_id: owner.to_string(),
                resource_id: "map".into(),
                map: serde_wasm_bindgen::from_value(registry_ref(
                    &owner.to_string(),
                    "map",
                    "map-hash",
                ))
                .unwrap(),
                clock: serde_wasm_bindgen::from_value(registry_ref(
                    &owner.to_string(),
                    "clock",
                    "clock-hash",
                ))
                .unwrap(),
            }],
        };
        let value = maps_to_js(&maps).unwrap();
        let resources =
            Array::from(&Reflect::get(&value, &JsValue::from_str("resources")).unwrap());
        let row = resources.get(0);
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("resource_id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("map")
        );

        assert_eq!(
            resources_from_js(resources_to_js(&response).unwrap()).unwrap(),
            response
        );
        assert_eq!(maps_from_js(value).unwrap(), maps);
    }

    #[wasm_bindgen_test]
    fn flattened_catalog_rows_are_plain_javascript_records() {
        let fixture = js_sys::JSON::parse(include_str!(
            "../../../../../crates/auki-protocols/tests/locked/catalog_row_sensor_log_camera_live_rolling.json"
        ))
        .unwrap();
        let row: auki_protocols::catalog::v2::ResourceEntry =
            serde_wasm_bindgen::from_value(fixture).unwrap();
        let response = v3::ResourcesResponse {
            resources: vec![v3::ResourceEntry::V2(Box::new(row))],
        };

        let value = resources_to_js(&response).unwrap();
        let resources =
            Array::from(&Reflect::get(&value, &JsValue::from_str("resources")).unwrap());
        let row = resources.get(0);
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("variant"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("sensor_log")
        );
        assert_eq!(
            Reflect::get(&row, &JsValue::from_str("resource_id"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("head_left_rgb")
        );
    }

    #[wasm_bindgen_test]
    fn catalog_provider_values_are_validated_immediately() {
        let invalid_maps = Object::new();
        let invalid_rows = Array::new();
        let invalid_row = Object::new();
        Reflect::set(
            &invalid_row,
            &JsValue::from_str("source_peer_id"),
            &JsValue::from_str(""),
        )
        .unwrap();
        Reflect::set(
            &invalid_row,
            &JsValue::from_str("writer_peer_id"),
            &JsValue::from_str("writer"),
        )
        .unwrap();
        Reflect::set(
            &invalid_row,
            &JsValue::from_str("resource_id"),
            &JsValue::from_str("map"),
        )
        .unwrap();
        Reflect::set(
            &invalid_row,
            &JsValue::from_str("map"),
            &registry_ref("writer", "map", "map-hash"),
        )
        .unwrap();
        Reflect::set(
            &invalid_row,
            &JsValue::from_str("clock"),
            &registry_ref("writer", "clock", "clock-hash"),
        )
        .unwrap();
        invalid_rows.push(&invalid_row);
        Reflect::set(
            &invalid_maps,
            &JsValue::from_str("resources"),
            &invalid_rows,
        )
        .unwrap();
        assert!(maps_from_js(invalid_maps.into()).is_err());

        assert_eq!(
            resources_from_js(resources_to_js(&empty_resources()).unwrap()).unwrap(),
            empty_resources()
        );
        assert_eq!(
            maps_from_js(maps_to_js(&empty_maps()).unwrap()).unwrap(),
            empty_maps()
        );
    }

    #[wasm_bindgen_test]
    fn catalog_providers_are_invoked_synchronously_with_request_context() {
        let resources_callback =
            Closure::<dyn FnMut(JsValue, JsValue) -> JsValue>::new(|requester, request| {
                assert_eq!(
                    Reflect::get(&requester, &JsValue::from_str("peerId"))
                        .unwrap()
                        .as_string()
                        .as_deref(),
                    Some(peer_id().to_string().as_str())
                );
                let variants =
                    Array::from(&Reflect::get(&request, &JsValue::from_str("variants")).unwrap());
                assert_eq!(variants.length(), 1);
                assert_eq!(
                    variants.get(0).as_string().as_deref(),
                    Some("message_channel")
                );
                resources_to_js(&empty_resources()).unwrap()
            });
        let maps_callback = Closure::<dyn FnMut(JsValue) -> JsValue>::new(|requester| {
            assert_eq!(
                Reflect::get(&requester, &JsValue::from_str("peerType"))
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some("robot")
            );
            maps_to_js(&empty_maps()).unwrap()
        });
        let provider = JavaScriptCatalogProvider {
            resources: Some(
                resources_callback
                    .as_ref()
                    .unchecked_ref::<Function>()
                    .clone(),
            ),
            maps: Some(maps_callback.as_ref().unchecked_ref::<Function>().clone()),
        };
        let request = v3::ResourcesRequest {
            variants: vec![v3::ResourceVariant::MessageChannel],
        };
        assert_eq!(
            provider.resources(&authenticated_peer(), &request),
            empty_resources()
        );
        assert_eq!(provider.maps(&authenticated_peer()), empty_maps());

        let provider = JavaScriptCatalogProvider {
            resources: None,
            maps: None,
        };
        assert_eq!(
            provider.resources(&authenticated_peer(), &request),
            empty_resources()
        );
        assert_eq!(provider.maps(&authenticated_peer()), empty_maps());
    }
}
