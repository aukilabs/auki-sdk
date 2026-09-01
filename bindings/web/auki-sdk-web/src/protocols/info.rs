use auki_protocols::info::{
    InfoClient,
    v1::{AuthenticatedParticipantInfo, ID},
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    AukiPeer,
    protocol_support::{js_context, parse_exact_target, peer_protocols, to_js_value},
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
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
"#;

#[cfg(test)]
mod tests {
    use js_sys::Reflect;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
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
}
