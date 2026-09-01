use std::cell::RefCell;

use auki_protocols::info::{
    InfoClient, InfoEndpoint, InfoProvider,
    v1::{AuthenticatedParticipantInfo, ID},
};
use auki_sdk::AuthenticatedPeer;
use js_sys::{Function, Promise};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, js_context, parse_exact_target, peer_protocols,
        to_js_value,
    },
};

/// Outbound participant-information v1 client backed by the portable Rust protocol.
#[wasm_bindgen]
pub struct AukiInfoClient {
    inner: InfoClient,
}

#[wasm_bindgen]
impl AukiInfoClient {
    /// Bind an outbound Info client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiInfoClient, JsValue> {
        Ok(Self {
            inner: InfoClient::new(peer_protocols(peer, "Info")?),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Fetch participant metadata through one exact advertised route.
    #[wasm_bindgen(js_name = fetchExact, unchecked_return_type = "AukiParticipantInfo")]
    pub async fn fetch_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
    ) -> Result<JsValue, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let info = self
            .inner
            .fetch_exact(peer_id, route)
            .await
            .map_err(|error| js_context("fetch participant info", error))?;
        participant_info_to_js(info)
    }
}

/// Mounted inbound participant-information v1 service.
#[wasm_bindgen]
pub struct AukiInfoEndpoint {
    inner: RefCell<Option<InfoEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiInfoEndpoint {
    /// Mount Info v1 on one running browser peer.
    ///
    /// The provider is sampled synchronously for each mutually authenticated
    /// requester. Returning `null` or `undefined` declines the request.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(
        peer: &AukiPeer,
        #[wasm_bindgen(unchecked_param_type = "AukiInfoProvider")] provider: Function,
    ) -> Result<AukiInfoEndpoint, JsValue> {
        let endpoint = InfoEndpoint::mount(
            peer_protocols(peer, "Info endpoint")?,
            JavaScriptInfoProvider { callback: provider },
        )
        .map_err(|error| js_context("mount Info endpoint", error))?;
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

    /// Idempotently stop accepting Info requests and await admitted handlers.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Info endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

struct JavaScriptInfoProvider {
    callback: Function,
}

impl InfoProvider for JavaScriptInfoProvider {
    fn participant_info(
        &self,
        requester: &AuthenticatedPeer,
    ) -> Option<AuthenticatedParticipantInfo> {
        let requester =
            authenticated_peer_to_js("convert authenticated Info requester", requester).ok()?;
        let value = self.callback.call1(&JsValue::UNDEFINED, &requester).ok()?;
        if value.is_null() || value.is_undefined() {
            return None;
        }
        participant_info_from_js(value).ok()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParticipantInfo {
    app: String,
    app_version: String,
    name: String,
    session_id: String,
    session_clock_id: String,
    session_clock_hash: String,
    session_now_ns: u64,
    peer_id: String,
    app_instance: String,
}

fn participant_info_from_js(value: JsValue) -> Result<AuthenticatedParticipantInfo, JsValue> {
    let info: ParticipantInfo = serde_wasm_bindgen::from_value(value)
        .map_err(|error| js_context("read participant info", error))?;
    Ok(AuthenticatedParticipantInfo {
        app: info.app,
        app_version: info.app_version,
        name: info.name,
        session_id: info.session_id,
        session_clock_id: info.session_clock_id,
        session_clock_hash: info.session_clock_hash,
        session_now_ns: info.session_now_ns,
        peer_id: info
            .peer_id
            .parse()
            .map_err(|error| js_context("parse participant-info Peer ID", error))?,
        app_instance: info.app_instance,
    })
}

fn participant_info_to_js(info: AuthenticatedParticipantInfo) -> Result<JsValue, JsValue> {
    to_js_value(
        "convert participant info",
        &ParticipantInfo {
            app: info.app,
            app_version: info.app_version,
            name: info.name,
            session_id: info.session_id,
            session_clock_id: info.session_clock_id,
            session_clock_hash: info.session_clock_hash,
            session_now_ns: info.session_now_ns,
            peer_id: info.peer_id.to_string(),
            app_instance: info.app_instance,
        },
    )
}

#[wasm_bindgen(typescript_custom_section)]
const PARTICIPANT_INFO_TYPESCRIPT: &str = r#"
/** Authenticated diagnostic metadata returned by Info v1. */
export interface AukiParticipantInfo {
    readonly app: string;
    readonly appVersion: string;
    readonly name: string;
    readonly sessionId: string;
    readonly sessionClockId: string;
    readonly sessionClockHash: string;
    readonly sessionNowNs: bigint;
    readonly peerId: string;
    readonly appInstance: string;
}

/** Synchronous participant snapshot sampled once per authenticated request. */
export type AukiInfoProvider = (
    requester: AukiAuthenticatedPeer,
) => AukiParticipantInfo | null | undefined;
"#;

#[cfg(test)]
mod tests {
    use js_sys::{Object, Reflect};
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
            scopes: vec!["info:read".into()],
            application: None,
            verified_until: "2030-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    #[wasm_bindgen_test]
    fn participant_info_is_a_plain_record_with_lossless_time() {
        let value = participant_info_to_js(AuthenticatedParticipantInfo {
            app: "example".into(),
            app_version: "1.2.3".into(),
            name: "Robot".into(),
            session_id: "session".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: u64::MAX,
            peer_id: peer_id(),
            app_instance: "browser".into(),
        })
        .unwrap();

        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("appVersion"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("1.2.3")
        );
        assert!(
            Reflect::get(&value, &JsValue::from_str("sessionNowNs"))
                .unwrap()
                .is_bigint()
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("peerId"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some(peer_id().to_string().as_str())
        );
    }

    #[wasm_bindgen_test]
    fn participant_info_round_trips_from_the_provider_shape() {
        let expected = AuthenticatedParticipantInfo {
            app: "example".into(),
            app_version: "1.2.3".into(),
            name: "Robot".into(),
            session_id: "session".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: u64::MAX,
            peer_id: peer_id(),
            app_instance: "browser".into(),
        };
        let value = participant_info_to_js(expected.clone()).unwrap();
        assert_eq!(participant_info_from_js(value).unwrap(), expected);

        let invalid = Object::new();
        Reflect::set(
            &invalid,
            &JsValue::from_str("peerId"),
            &JsValue::from_str("not-a-peer"),
        )
        .unwrap();
        assert!(participant_info_from_js(invalid.into()).is_err());
    }

    #[wasm_bindgen_test]
    fn authenticated_requester_uses_the_public_camel_case_shape() {
        let requester = authenticated_peer();
        let value = authenticated_peer_to_js("convert test requester", &requester).unwrap();
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("peerId"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some(peer_id().to_string().as_str())
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("peerType"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("robot")
        );
    }

    #[wasm_bindgen_test]
    fn info_provider_is_invoked_synchronously_with_the_authenticated_requester() {
        let expected = AuthenticatedParticipantInfo {
            app: "example".into(),
            app_version: "1.2.3".into(),
            name: "Robot".into(),
            session_id: "session".into(),
            session_clock_id: "clock".into(),
            session_clock_hash: "hash".into(),
            session_now_ns: 42,
            peer_id: peer_id(),
            app_instance: "browser".into(),
        };
        let returned = expected.clone();
        let callback = Closure::<dyn FnMut(JsValue) -> JsValue>::new(move |requester| {
            assert_eq!(
                Reflect::get(&requester, &JsValue::from_str("peerId"))
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some(peer_id().to_string().as_str())
            );
            participant_info_to_js(returned.clone()).unwrap()
        });
        let provider = JavaScriptInfoProvider {
            callback: callback.as_ref().unchecked_ref::<Function>().clone(),
        };
        assert_eq!(
            provider.participant_info(&authenticated_peer()),
            Some(expected)
        );

        let decline = Closure::<dyn FnMut(JsValue) -> JsValue>::new(|_| JsValue::NULL);
        let provider = JavaScriptInfoProvider {
            callback: decline.as_ref().unchecked_ref::<Function>().clone(),
        };
        assert!(provider.participant_info(&authenticated_peer()).is_none());
    }
}
