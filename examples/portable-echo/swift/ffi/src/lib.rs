//! Mechanical Swift adapter for the shared Rust portable-echo endpoint.
//!
//! The final Apple artifact contains this adapter and `auki-sdk-swift` in one
//! Rust library. UniFFI object handles therefore never cross independently
//! loaded Rust runtimes.

use std::sync::Arc;

use auki_echo_protocol::{
    EchoClient, EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID,
};
use auki_sdk_binding::{AukiPeer, AukiPeerRoutes, CleanupResult, DetachedCleanup, wait_cleanup};
use auki_sdk_rs::{Multiaddr, PeerId};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AukiEchoError {
    #[error("{message}")]
    Operation { message: String },
}

fn operation_error(context: &'static str, error: impl std::fmt::Display) -> AukiEchoError {
    AukiEchoError::Operation {
        message: format!("{context}: {error}"),
    }
}

fn parse_target(peer_id: &str, route: &str) -> Result<(PeerId, Multiaddr), AukiEchoError> {
    let peer_id = peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse remote Peer ID", error))?;
    let route = route
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse remote route", error))?;
    Ok((peer_id, route))
}

/// Exact authenticated peer target chosen from an advertised peer card.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiPeerTarget {
    pub domain_id: String,
    pub peer_id: String,
    pub route: String,
}

/// Portable card copied between examples until discovery supplies the same
/// bounded information automatically.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiPeerCard {
    pub version: u32,
    pub domain_id: String,
    pub peer_id: String,
    pub protocols: Vec<String>,
    pub routes: AukiPeerRoutes,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PeerCardWire {
    version: u32,
    domain_id: String,
    peer_id: String,
    protocols: Vec<String>,
    routes: PeerCardRoutesWire,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PeerCardRoutesWire {
    tcp: String,
    wss: String,
}

fn validate_card(card: &AukiPeerCard) -> Result<(), AukiEchoError> {
    if card.version != 1 {
        return Err(AukiEchoError::Operation {
            message: format!("unsupported peer card version {}", card.version),
        });
    }
    Uuid::parse_str(&card.domain_id)
        .map_err(|error| operation_error("parse peer card Domain ID", error))?;
    card.peer_id
        .parse::<PeerId>()
        .map_err(|error| operation_error("parse peer card Peer ID", error))?;
    card.routes
        .tcp
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse peer card TCP route", error))?;
    card.routes
        .wss
        .parse::<Multiaddr>()
        .map_err(|error| operation_error("parse peer card WSS route", error))?;
    if !card
        .protocols
        .iter()
        .any(|protocol| protocol == PROTOCOL_ID)
    {
        return Err(AukiEchoError::Operation {
            message: format!("peer card does not advertise {PROTOCOL_ID}"),
        });
    }
    Ok(())
}

#[uniffi::export]
pub fn peer_card_to_json(card: AukiPeerCard) -> Result<String, AukiEchoError> {
    validate_card(&card)?;
    serde_json::to_string(&PeerCardWire {
        version: card.version,
        domain_id: card.domain_id,
        peer_id: card.peer_id,
        protocols: card.protocols,
        routes: PeerCardRoutesWire {
            tcp: card.routes.tcp,
            wss: card.routes.wss,
        },
    })
    .map_err(|error| operation_error("encode peer card", error))
}

#[uniffi::export]
pub fn peer_card_from_json(json: String) -> Result<AukiPeerCard, AukiEchoError> {
    let wire = serde_json::from_str::<PeerCardWire>(&json)
        .map_err(|error| operation_error("decode peer card", error))?;
    let card = AukiPeerCard {
        version: wire.version,
        domain_id: wire.domain_id,
        peer_id: wire.peer_id,
        protocols: wire.protocols,
        routes: AukiPeerRoutes {
            tcp: wire.routes.tcp,
            wss: wire.routes.wss,
        },
    };
    validate_card(&card)?;
    Ok(card)
}

/// Select the native TCP member of one peer card's atomic relay-route pair.
#[uniffi::export]
pub fn native_target(card: AukiPeerCard) -> Result<AukiPeerTarget, AukiEchoError> {
    validate_card(&card)?;
    Ok(AukiPeerTarget {
        domain_id: card.domain_id,
        peer_id: card.peer_id,
        route: card.routes.tcp,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiEchoSendReceipt {
    pub remote_peer_id: String,
    pub payload: Vec<u8>,
    pub relayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiEchoServeReceipt {
    pub remote_peer_id: String,
    pub payload: Vec<u8>,
}

struct EchoOwner {
    endpoint: Mutex<Option<EchoEndpoint>>,
    cleanup: DetachedCleanup,
}

impl EchoOwner {
    fn new(endpoint: EchoEndpoint) -> Self {
        Self {
            endpoint: Mutex::new(Some(endpoint)),
            cleanup: DetachedCleanup::new(),
        }
    }

    fn ensure_open(&self) -> Result<(), AukiEchoError> {
        if self.endpoint.lock().is_some() {
            Ok(())
        } else {
            Err(AukiEchoError::Operation {
                message: "portable echo endpoint is stopped".into(),
            })
        }
    }

    fn begin_close(&self) -> watch::Receiver<Option<CleanupResult>> {
        self.cleanup.get_or_start(|| {
            let close = self.endpoint.lock().take().map(EchoEndpoint::close);
            async move {
                match close {
                    Some(close) => close.await.map_err(|error| error.to_string()),
                    None => Ok(()),
                }
            }
        })
    }
}

impl Drop for EchoOwner {
    fn drop(&mut self) {
        if self.endpoint.get_mut().is_some() {
            let _ = self.begin_close();
        }
    }
}

/// Mounted portable echo service and its outbound client.
#[derive(uniffi::Object)]
pub struct AukiEcho {
    owner: EchoOwner,
    client: EchoClient,
    events: EchoEventReceiver,
    peer: Arc<AukiPeer>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiEcho {
    /// Mount the shared Rust endpoint on a running Auki peer.
    ///
    /// This is async at the FFI boundary so the owner captures UniFFI's Tokio
    /// runtime for cleanup from arbitrary Swift threads.
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiEchoError> {
        let endpoint = EchoEndpoint::mount(peer.rust_protocols())
            .map_err(|error| operation_error("mount portable echo", error))?;
        let client = endpoint.client();
        let events = endpoint.events();
        Ok(Arc::new(Self {
            owner: EchoOwner::new(endpoint),
            client,
            events,
            peer,
        }))
    }

    pub fn protocol(&self) -> String {
        PROTOCOL_ID.into()
    }

    /// Build the exact copyable card for this running endpoint.
    pub fn card(&self) -> Result<AukiPeerCard, AukiEchoError> {
        let card = AukiPeerCard {
            version: 1,
            domain_id: self.peer.domain_id(),
            peer_id: self.peer.peer_id(),
            protocols: vec![PROTOCOL_ID.into()],
            routes: self
                .peer
                .routes()
                .map_err(|error| operation_error("read local peer routes", error))?,
        };
        validate_card(&card)?;
        Ok(card)
    }

    /// Send one bounded echo through an exact authenticated route.
    pub async fn send_exact(
        &self,
        target: AukiPeerTarget,
        payload: Vec<u8>,
    ) -> Result<AukiEchoSendReceipt, AukiEchoError> {
        self.owner.ensure_open()?;
        if target.domain_id != self.peer.domain_id() {
            return Err(AukiEchoError::Operation {
                message: format!(
                    "remote peer Domain {} does not match local Domain {}",
                    target.domain_id,
                    self.peer.domain_id()
                ),
            });
        }
        let (remote_peer_id, route) = parse_target(&target.peer_id, &target.route)?;
        let receipt = self
            .client
            .send_exact(remote_peer_id, route, payload)
            .await
            .map_err(|error| operation_error("run portable echo", error))?;
        Ok(AukiEchoSendReceipt {
            remote_peer_id: receipt.remote_peer_id.to_string(),
            payload: receipt.payload,
            relayed: receipt.relayed,
        })
    }

    /// Wait for one inbound completion from the bounded Rust event queue.
    pub async fn next_served(&self) -> Result<AukiEchoServeReceipt, AukiEchoError> {
        self.owner.ensure_open()?;
        match self.events.recv().await {
            Some(EchoServeEvent::Served(receipt)) => Ok(AukiEchoServeReceipt {
                remote_peer_id: receipt.remote_peer_id.to_string(),
                payload: receipt.payload,
            }),
            Some(EchoServeEvent::Failed {
                remote_peer_id,
                error,
            }) => Err(AukiEchoError::Operation {
                message: format!("serve portable echo from {remote_peer_id}: {error}"),
            }),
            Some(EchoServeEvent::Lagged { dropped }) => Err(AukiEchoError::Operation {
                message: format!("observe portable echo: event consumer fell behind by {dropped}"),
            }),
            None => Err(AukiEchoError::Operation {
                message: "portable echo endpoint is stopped".into(),
            }),
        }
    }

    /// Stop inbound serving behind one detached, replayable cleanup barrier.
    pub async fn close(&self) -> Result<(), AukiEchoError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close portable echo", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer_card() -> AukiPeerCard {
        let peer_id = auki_sdk_rs::Identity::generate().peer_id().to_string();
        AukiPeerCard {
            version: 1,
            domain_id: "00000000-0000-0000-0000-000000000001".into(),
            peer_id: peer_id.clone(),
            protocols: vec![PROTOCOL_ID.into()],
            routes: AukiPeerRoutes {
                tcp: format!("/dns4/relay.example.com/tcp/443/p2p/{peer_id}"),
                wss: format!("/dns4/relay.example.com/tcp/4443/wss/p2p/{peer_id}"),
            },
        }
    }

    #[test]
    fn adapter_uses_the_shared_protocol_id() {
        assert_eq!(PROTOCOL_ID, "/example/echo/1.0.0");
    }

    #[test]
    fn invalid_exact_targets_fail_at_the_binding_boundary() {
        assert!(parse_target("not-a-peer", "/ip4/127.0.0.1/tcp/1").is_err());
        assert!(
            parse_target(
                "12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3",
                "not-a-route"
            )
            .is_err()
        );
    }

    #[test]
    fn peer_card_json_round_trip_is_strict_and_stable() {
        let card = peer_card();
        let json = peer_card_to_json(card.clone()).expect("encode peer card");

        assert_eq!(peer_card_from_json(json).expect("decode peer card"), card);

        let json_with_unknown_field = format!(
            r#"{{"version":1,"domainId":"{}","peerId":"{}","protocols":["{}"],"routes":{{"tcp":"{}","wss":"{}"}},"surprise":true}}"#,
            card.domain_id, card.peer_id, PROTOCOL_ID, card.routes.tcp, card.routes.wss,
        );
        assert!(peer_card_from_json(json_with_unknown_field).is_err());
    }

    #[test]
    fn native_target_uses_the_tcp_route() {
        let card = peer_card();
        let target = native_target(card.clone()).expect("select native target");

        assert_eq!(target.domain_id, card.domain_id);
        assert_eq!(target.peer_id, card.peer_id);
        assert_eq!(target.route, card.routes.tcp);
    }
}
