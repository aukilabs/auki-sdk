//! Wasm binding for the portable Catalog v3 resources and v4 maps client.

use auki_protocols::catalog::{CatalogClient, v3, v4};
use wasm_bindgen::prelude::*;

use crate::{
    AukiPeer,
    protocol_support::{js_context, parse_exact_target, peer_protocols, to_js_value},
};

const RESOURCE_VARIANT_NAMES: &str =
    "sensor_log, pose_log, time_transform_log, detection_log, or message_channel";

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

fn maps_to_js(response: &v4::ResourcesResponse) -> Result<JsValue, JsValue> {
    to_js_value("convert Catalog maps", response)
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
"#;

#[cfg(test)]
mod tests {
    use js_sys::{Array, Object, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
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
    }
}
