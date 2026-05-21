use auki_identity::Wallet;
use auki_network::PeerIdentity;
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
const BROWSER_PROBE_TIMEOUT_MS: i32 = 10_000;

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
    Ok(peer_identity_from_seed_bytes(seed).peer_id().to_string())
}

fn peer_identity_from_seed_bytes(seed: &[u8; 32]) -> PeerIdentity {
    // Post Plan A, `Wallet::from_seed` takes `Vec<u8>` and returns
    // `Result<Arc<Wallet>, IdentityError>`; the 32-byte length is
    // structurally guaranteed by the caller (a fixed-size array).
    let wallet = Wallet::from_seed(seed.to_vec()).expect("32-byte seed");
    PeerIdentity::from_wallet(wallet)
}

#[cfg_attr(feature = "browser_libp2p", derive(serde::Serialize))]
pub struct BrowserProbeResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub protocol: String,
    pub payload: Vec<u8>,
    pub error: Option<String>,
}

impl BrowserProbeResult {
    pub fn ok(
        local_peer_id: impl Into<String>,
        protocol: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            protocol: protocol.into(),
            payload,
            error: None,
        }
    }

    pub fn err(
        local_peer_id: impl Into<String>,
        protocol: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            protocol: protocol.into(),
            payload: Vec::new(),
            error: Some(error.into()),
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen(js_name = dialBrowserProbe)]
pub async fn dial_browser_probe(
    seed: &[u8],
    address: String,
    payload: &[u8],
) -> Result<JsValue, JsValue> {
    let seed = seed_array(seed).map_err(|err| JsValue::from_str(&err))?;
    let identity = peer_identity_from_seed_bytes(&seed);
    let local_peer_id = identity.peer_id().to_string();
    let outcome = dial_browser_probe_inner(identity, address, payload.to_vec()).await;
    let result = match outcome {
        Ok(payload) => {
            BrowserProbeResult::ok(local_peer_id, auki_network::BROWSER_PROBE_PROTOCOL, payload)
        }
        Err(err) => {
            BrowserProbeResult::err(local_peer_id, auki_network::BROWSER_PROBE_PROTOCOL, err)
        }
    };

    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[derive(libp2p::swarm::NetworkBehaviour)]
struct BrowserProbeBehaviour {
    probe: libp2p::request_response::json::Behaviour<
        auki_network::BrowserProbeRequest,
        auki_network::BrowserProbeResponse,
    >,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn dial_browser_probe_inner(
    identity: PeerIdentity,
    address: String,
    payload: Vec<u8>,
) -> Result<Vec<u8>, String> {
    use futures::{FutureExt as _, StreamExt as _, future};
    use libp2p::{
        Multiaddr, StreamProtocol, SwarmBuilder,
        request_response::{self, ProtocolSupport},
        swarm::SwarmEvent,
    };

    let address: Multiaddr = address
        .parse()
        .map_err(|err| format!("malformed multiaddr: {err}"))?;
    let remote_peer = peer_id_from_multiaddr(&address)?;

    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_wasm_bindgen()
        .with_other_transport(|keypair| {
            libp2p::webrtc_websys::Transport::new(libp2p::webrtc_websys::Config::new(keypair))
                .boxed()
        })
        .map_err(|err| format!("transport setup failed: {err}"))?
        .with_behaviour(|_| BrowserProbeBehaviour {
            probe: request_response::json::Behaviour::new(
                [(
                    StreamProtocol::new(auki_network::BROWSER_PROBE_PROTOCOL),
                    ProtocolSupport::Full,
                )],
                request_response::Config::default(),
            ),
        })
        .map_err(|err| format!("behaviour setup failed: {err}"))?
        .build();

    let nonce = "browser-probe-1".to_string();
    let request = auki_network::BrowserProbeRequest {
        nonce: nonce.clone(),
        payload,
    };
    let request_id = swarm.behaviour_mut().probe.send_request_with_addresses(
        &remote_peer,
        request,
        vec![address],
    );

    let probe = async move {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::Behaviour(BrowserProbeBehaviourEvent::Probe(
                    request_response::Event::Message {
                        peer,
                        message:
                            request_response::Message::Response {
                                request_id: response_id,
                                response,
                            },
                        ..
                    },
                )) if peer == remote_peer && response_id == request_id => {
                    if response.nonce != nonce {
                        return Err(format!(
                            "response nonce mismatch: expected {nonce}, got {}",
                            response.nonce
                        ));
                    }
                    return Ok(response.payload);
                }
                SwarmEvent::Behaviour(BrowserProbeBehaviourEvent::Probe(
                    request_response::Event::OutboundFailure {
                        peer,
                        request_id: failure_id,
                        error,
                        ..
                    },
                )) if peer == remote_peer && failure_id == request_id => {
                    return Err(format!("outbound failure for {peer}: {error}"));
                }
                SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(peer),
                    error,
                    ..
                } if peer == remote_peer => {
                    return Err(format!("dial failure for {peer}: {error}"));
                }
                _ => {}
            }
        }
    }
    .fuse();

    let timeout = js_timeout(BROWSER_PROBE_TIMEOUT_MS).fuse();
    futures::pin_mut!(probe, timeout);

    match future::select(probe, timeout).await {
        future::Either::Left((result, _)) => result,
        future::Either::Right((timeout_result, _)) => match timeout_result {
            Ok(()) => Err(format!(
                "probe timed out after {BROWSER_PROBE_TIMEOUT_MS}ms"
            )),
            Err(err) => Err(format!("probe timeout setup failed: {err}")),
        },
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn peer_id_from_multiaddr(address: &libp2p::Multiaddr) -> Result<libp2p::PeerId, String> {
    use libp2p::multiaddr::Protocol;

    address
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::P2p(peer_id) => Some(peer_id),
            _ => None,
        })
        .ok_or_else(|| format!("multiaddr is missing /p2p/<peer-id>: {address}"))
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn js_timeout(ms: i32) -> Result<(), String> {
    use wasm_bindgen::JsCast as _;

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let Some(window) = web_sys::window() else {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("window unavailable"),
            );
            return;
        };

        let callback = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });

        if let Err(err) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.as_ref().unchecked_ref(),
            ms,
        ) {
            let _ = reject.call1(&JsValue::UNDEFINED, &err);
        }
    });

    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|err| {
            err.as_string()
                .unwrap_or_else(|| "JavaScript timer rejected".to_string())
        })
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

#[cfg(test)]
mod browser_probe_result_tests {
    use super::*;

    #[test]
    fn browser_probe_result_carries_peer_protocol_and_payload() {
        let result = BrowserProbeResult::ok(
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
            "/auki/browser-probe/0.0.1",
            vec![1, 2, 3],
        );

        assert!(result.ok);
        assert_eq!(
            result.local_peer_id,
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
        assert_eq!(result.protocol, "/auki/browser-probe/0.0.1");
        assert_eq!(result.payload, vec![1, 2, 3]);
        assert!(result.error.is_none());
    }
}
