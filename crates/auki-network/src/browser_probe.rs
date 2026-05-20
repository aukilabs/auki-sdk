use std::time::Duration;

use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
    core::{Transport as _, muxing::StreamMuxerBox, transport::Boxed},
    request_response::{self, ProtocolSupport, json},
    swarm::{NetworkBehaviour, SwarmEvent},
};
use libp2p_webrtc as webrtc;
use rand::thread_rng;
use thiserror::Error;

use crate::{BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, PeerIdentity};

pub fn responder_label(identity: &PeerIdentity) -> String {
    format!("native:{}", identity.peer_id())
}

#[derive(NetworkBehaviour)]
pub struct BrowserProbeBehaviour {
    pub probe: json::Behaviour<BrowserProbeRequest, BrowserProbeResponse>,
}

#[derive(Debug, Error)]
pub enum BrowserProbeError {
    #[error("transport setup failed: {0}")]
    Transport(String),
    #[error("listen failed for {addr}: {source}")]
    Listen {
        addr: Multiaddr,
        source: libp2p::TransportError<std::io::Error>,
    },
    #[error("listener did not produce a dialable address within {0:?}")]
    ListenTimeout(Duration),
}

pub fn webrtc_direct_transport(
    keypair: &libp2p::identity::Keypair,
) -> Boxed<(PeerId, StreamMuxerBox)> {
    let certificate = webrtc::tokio::Certificate::generate(&mut thread_rng())
        .expect("WebRTC certificate generation should succeed");
    webrtc::tokio::Transport::new(keypair.clone(), certificate)
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed()
}

pub fn build_browser_probe_swarm(
    identity: &PeerIdentity,
) -> Result<Swarm<BrowserProbeBehaviour>, BrowserProbeError> {
    SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_other_transport(webrtc_direct_transport)
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))?
        .with_behaviour(|_| BrowserProbeBehaviour {
            probe: json::Behaviour::new(
                [(
                    StreamProtocol::new(BROWSER_PROBE_PROTOCOL),
                    ProtocolSupport::Full,
                )],
                request_response::Config::default(),
            ),
        })
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))
        .map(|builder| builder.build())
}

pub async fn listen_and_serve(
    identity: PeerIdentity,
    listen_addr: Multiaddr,
) -> Result<(), BrowserProbeError> {
    let mut swarm = build_browser_probe_swarm(&identity)?;
    swarm
        .listen_on(listen_addr.clone())
        .map_err(|source| BrowserProbeError::Listen {
            addr: listen_addr,
            source,
        })?;

    while let Some(event) = swarm.next().await {
        match event {
            SwarmEvent::Behaviour(BrowserProbeBehaviourEvent::Probe(
                request_response::Event::Message {
                    message:
                        request_response::Message::Request {
                            request, channel, ..
                        },
                    ..
                },
            )) => {
                let response =
                    BrowserProbeResponse::from_request(&request, responder_label(&identity));
                let _ = swarm.behaviour_mut().probe.send_response(channel, response);
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                println!(
                    "PARK_BROWSER_PROBE_ADDR={address}/p2p/{}",
                    identity.peer_id()
                );
            }
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_label_uses_native_peer_id() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);

        assert_eq!(
            responder_label(&identity),
            "native:12D3KooWSfMx5BpXVMrzyfMGHVLQe6UWNWX13ZBPLDmVoAKZ4oun"
        );
    }

    #[test]
    fn response_uses_native_responder_label() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        let request = BrowserProbeRequest {
            nonce: "n".to_string(),
            payload: vec![9],
        };

        let response = BrowserProbeResponse::from_request(&request, responder_label(&identity));

        assert_eq!(response.responder, responder_label(&identity));
    }

    #[test]
    fn browser_probe_swarm_uses_sdk_peer_identity() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        let swarm = build_browser_probe_swarm(&identity).expect("probe swarm builds");

        assert_eq!(*swarm.local_peer_id(), identity.peer_id());
    }
}
