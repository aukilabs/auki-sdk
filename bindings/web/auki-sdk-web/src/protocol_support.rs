use std::fmt::Display;

use auki_sdk::{AukiPeerProtocols, Multiaddr, PeerId};
use js_sys::Error as JsError;
#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::AukiPeer;

pub(crate) fn js_context(context: &'static str, error: impl Display) -> JsValue {
    js_error(format!("{context}: {error}"))
}

pub(crate) fn js_error(message: impl AsRef<str>) -> JsValue {
    JsError::new(message.as_ref()).into()
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExactTarget {
    peer_id: String,
    route: String,
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
pub(crate) fn peer_protocols(
    peer: &AukiPeer,
    protocol: &'static str,
) -> Result<AukiPeerProtocols, JsValue> {
    peer.protocols().ok_or_else(|| {
        js_error(format!(
            "cannot create {protocol} client: Auki peer is stopped"
        ))
    })
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
pub(crate) fn parse_exact_target(target: JsValue) -> Result<(PeerId, Multiaddr), JsValue> {
    let target: ExactTarget = serde_wasm_bindgen::from_value(target)
        .map_err(|error| js_context("read exact peer target", error))?;
    let peer_id = target
        .peer_id
        .parse::<PeerId>()
        .map_err(|error| js_context("parse target Peer ID", error))?;
    let route = target
        .route
        .parse::<Multiaddr>()
        .map_err(|error| js_context("parse target route", error))?;
    Ok((peer_id, route))
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
pub(crate) fn to_js_value(
    context: &'static str,
    value: &impl Serialize,
) -> Result<JsValue, JsValue> {
    let serializer =
        serde_wasm_bindgen::Serializer::new().serialize_large_number_types_as_bigints(true);
    value
        .serialize(&serializer)
        .map_err(|error| js_context(context, error))
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "registry"
))]
#[wasm_bindgen(typescript_custom_section)]
const EXACT_TARGET_TYPESCRIPT: &str = r#"
/** Exact advertised route for one mutually authenticated Auki peer. */
export interface AukiExactTarget {
    readonly peerId: string;
    readonly route: string;
}
"#;

#[cfg(all(
    test,
    any(
        feature = "blob",
        feature = "catalog",
        feature = "info",
        feature = "registry"
    )
))]
mod tests {
    use js_sys::{Object, Reflect};
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test]
    fn exact_target_is_validated_at_the_wasm_boundary() {
        let target = Object::new();
        Reflect::set(
            &target,
            &JsValue::from_str("peerId"),
            &JsValue::from_str("not-a-peer"),
        )
        .unwrap();
        Reflect::set(
            &target,
            &JsValue::from_str("route"),
            &JsValue::from_str("not-a-route"),
        )
        .unwrap();

        let error = parse_exact_target(target.into()).unwrap_err();
        assert!(
            js_sys::Error::from(error)
                .message()
                .as_string()
                .unwrap()
                .contains("Peer ID")
        );
    }
}
