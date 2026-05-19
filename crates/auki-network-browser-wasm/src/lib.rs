use auki_identity::Wallet;
use auki_network::PeerIdentity;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sdkName)]
pub fn sdk_name() -> String {
    "auki-network-browser-wasm".to_string()
}

#[wasm_bindgen(js_name = peerIdFromSeed)]
pub fn peer_id_from_seed(seed: &[u8]) -> Result<String, JsValue> {
    let seed = seed_array(seed)?;
    peer_id_from_seed_bytes(&seed).map_err(|err| JsValue::from_str(&err))
}

pub fn peer_id_from_seed_bytes(seed: &[u8; 32]) -> Result<String, String> {
    let wallet = Wallet::from_seed(seed);
    let identity = PeerIdentity::from_wallet(&wallet);
    Ok(identity.peer_id().to_string())
}

#[wasm_bindgen(js_name = supportedTransports)]
pub fn supported_transports() -> js_sys::Array {
    supported_transports_vec()
        .into_iter()
        .map(JsValue::from_str)
        .collect()
}

pub fn supported_transports_vec() -> Vec<&'static str> {
    #[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
    {
        // These imports intentionally prove the libp2p umbrella crate exposes
        // the browser transport modules under the selected feature set.
        use libp2p::webrtc_websys as _;
        use libp2p::websocket_websys as _;
        use libp2p::webtransport_websys as _;

        return vec![
            "libp2p-webrtc-websys",
            "libp2p-webtransport-websys",
            "libp2p-websocket-websys",
        ];
    }

    #[cfg(not(all(target_arch = "wasm32", feature = "browser_libp2p")))]
    {
        vec!["identity-only"]
    }
}

fn seed_array(seed: &[u8]) -> Result<[u8; 32], String> {
    if seed.len() != 32 {
        return Err(format!("seed must be 32 bytes, got {}", seed.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(seed);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_seed_03_peer_id_matches_sdk_vector() {
        assert_eq!(
            peer_id_from_seed_bytes(&[3u8; 32]).expect("valid seed"),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
    }

    #[test]
    fn rejects_wrong_length_seed() {
        let err = seed_array(&[1, 2, 3]).expect_err("short seed rejected");
        assert_eq!(err, "seed must be 32 bytes, got 3");
    }
}

#[cfg(test)]
mod transport_feature_tests {
    use super::*;

    #[test]
    fn base_build_reports_no_transport_features() {
        let features = supported_transports_vec();
        assert_eq!(features, vec!["identity-only"]);
    }
}
