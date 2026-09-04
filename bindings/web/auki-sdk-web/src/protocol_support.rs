#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
use std::cell::RefCell;
use std::fmt::Display;

use auki_sdk::{AukiPeerProtocols, AuthenticatedPeer, Multiaddr, PeerId};
use js_sys::Error as JsError;
#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
use js_sys::{Promise, Reflect};
#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
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
    feature = "message",
    feature = "registry",
    feature = "stream"
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
    feature = "message",
    feature = "registry",
    feature = "stream"
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
    feature = "message",
    feature = "registry",
    feature = "stream"
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
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
pub(crate) fn to_js_value(
    context: &'static str,
    value: &impl Serialize,
) -> Result<JsValue, JsValue> {
    let serializer = serde_wasm_bindgen::Serializer::new()
        .serialize_maps_as_objects(true)
        .serialize_large_number_types_as_bigints(true);
    value
        .serialize(&serializer)
        .map_err(|error| js_context(context, error))
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthenticatedPeerRecord {
    pub(crate) peer_id: String,
    pub(crate) subject: String,
    pub(crate) peer_type: Option<String>,
    pub(crate) domain_ids: Vec<String>,
    pub(crate) scopes: Vec<String>,
    pub(crate) application: Option<ApplicationMetadataRecord>,
    pub(crate) verified_until: String,
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
impl From<&AuthenticatedPeer> for AuthenticatedPeerRecord {
    fn from(peer: &AuthenticatedPeer) -> Self {
        Self {
            peer_id: peer.peer_id.to_string(),
            subject: peer.subject.to_string(),
            peer_type: peer.peer_type.clone(),
            domain_ids: peer.domain_ids.iter().map(ToString::to_string).collect(),
            scopes: peer.scopes.clone(),
            application: peer
                .application
                .as_ref()
                .map(|application| ApplicationMetadataRecord {
                    name: application.name.clone(),
                    version: application.version.clone(),
                }),
            verified_until: peer.verified_until.to_rfc3339(),
        }
    }
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ApplicationMetadataRecord {
    pub(crate) name: String,
    pub(crate) version: String,
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
pub(crate) fn authenticated_peer_to_js(
    context: &'static str,
    peer: &AuthenticatedPeer,
) -> Result<JsValue, JsValue> {
    to_js_value(context, &AuthenticatedPeerRecord::from(peer))
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
pub(crate) fn javascript_error_reason(error: &JsValue) -> String {
    if let Some(message) = error.as_string() {
        return message;
    }
    if let Ok(message) = Reflect::get(error, &JsValue::from_str("message"))
        && let Some(message) = message.as_string()
    {
        return message;
    }
    "JavaScript callback failed".to_owned()
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
#[wasm_bindgen(typescript_custom_section)]
const AUTHENTICATED_PEER_TYPESCRIPT: &str = r#"
/** Safe metadata authenticated from the remote peer's DDS credential. */
export interface AukiAuthenticatedPeer {
    readonly peerId: string;
    readonly subject: string;
    readonly peerType?: string;
    readonly domainIds: readonly string[];
    readonly scopes: readonly string[];
    readonly application?: {
        readonly name: string;
        readonly version: string;
    };
    readonly verifiedUntil: string;
}
"#;

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
#[wasm_bindgen(typescript_custom_section)]
const EXACT_TARGET_TYPESCRIPT: &str = r#"
/** Exact advertised route for one mutually authenticated Auki peer. */
export interface AukiExactTarget {
    readonly peerId: string;
    readonly route: string;
}
"#;

/// Shared idempotent close promise for Wasm handles that own asynchronous
/// protocol cleanup.
#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
#[derive(Default)]
pub(crate) struct CloseBarrier {
    promise: RefCell<Option<Promise>>,
}

#[cfg(any(
    feature = "blob",
    feature = "catalog",
    feature = "info",
    feature = "message",
    feature = "registry",
    feature = "stream"
))]
impl CloseBarrier {
    pub(crate) fn get_or_start(&self, start: impl FnOnce() -> Promise) -> Promise {
        if let Some(closing) = self.promise.borrow().clone() {
            return closing;
        }
        let closing = start();
        self.promise.borrow_mut().replace(closing.clone());
        closing
    }
}

#[cfg(all(
    test,
    any(
        feature = "blob",
        feature = "catalog",
        feature = "info",
        feature = "message",
        feature = "registry",
        feature = "stream"
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
