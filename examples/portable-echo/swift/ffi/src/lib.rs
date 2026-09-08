//! Mechanical Swift adapter for the shared Rust portable-echo endpoint.
//!
//! The final Apple artifact contains this adapter and `auki-sdk-swift` in one
//! Rust library. UniFFI object handles therefore never cross independently
//! loaded Rust runtimes.

use std::sync::Arc;

use auki_echo_protocol::{
    EchoClient, EchoEndpoint, EchoEventReceiver, EchoServeEvent, PROTOCOL_ID,
};
use auki_sdk_binding::{
    AukiPeer, AukiPeerCard, AukiPeerTarget, CleanupResult, DetachedCleanup, parse_target,
    wait_cleanup,
};
use parking_lot::Mutex;
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
        self.peer
            .card(vec![PROTOCOL_ID.into()])
            .map_err(|error| operation_error("build portable echo peer card", error))
    }

    /// Send one bounded echo through an exact authenticated route.
    pub async fn send_exact(
        &self,
        target: AukiPeerTarget,
        payload: Vec<u8>,
    ) -> Result<AukiEchoSendReceipt, AukiEchoError> {
        self.owner.ensure_open()?;
        let target_domain_id = Uuid::parse_str(&target.domain_id)
            .map_err(|error| operation_error("parse remote Domain ID", error))?;
        let local_domain_id = Uuid::parse_str(&self.peer.domain_id())
            .map_err(|error| operation_error("parse local Domain ID", error))?;
        if target_domain_id != local_domain_id {
            return Err(AukiEchoError::Operation {
                message: format!(
                    "remote peer Domain {} does not match local Domain {}",
                    target.domain_id,
                    self.peer.domain_id()
                ),
            });
        }
        let (remote_peer_id, route) =
            parse_target(target).map_err(|error| operation_error("parse exact target", error))?;
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

    /// Fence new endpoint operations and stop serving behind one detached,
    /// replayable cleanup barrier. Already-admitted operations may finish.
    pub async fn close(&self) -> Result<(), AukiEchoError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close portable echo", error))
    }
}

#[cfg(test)]
mod tests {
    use auki_sdk_binding::{
        AukiPeerRoutes, native_peer_target, peer_card_from_json, peer_card_to_json,
    };

    use super::*;

    fn peer_card() -> AukiPeerCard {
        let peer_id = auki_sdk_rs::Identity::generate().peer_id().to_string();
        let relay_peer_id = auki_sdk_rs::Identity::generate().peer_id().to_string();
        AukiPeerCard {
            version: 1,
            domain_id: "00000000-0000-0000-0000-000000000001".into(),
            peer_id: peer_id.clone(),
            protocols: vec![PROTOCOL_ID.into()],
            routes: AukiPeerRoutes {
                tcp: format!(
                    "/dns4/relay.example.com/tcp/443/p2p/{relay_peer_id}/p2p-circuit/p2p/{peer_id}"
                ),
                wss: format!(
                    "/dns4/relay.example.com/tcp/4443/wss/p2p/{relay_peer_id}/p2p-circuit/p2p/{peer_id}"
                ),
            },
        }
    }

    #[test]
    fn adapter_uses_the_shared_protocol_id() {
        assert_eq!(PROTOCOL_ID, "/example/echo/1.0.0");
    }

    #[test]
    fn invalid_exact_targets_fail_at_the_binding_boundary() {
        assert!(
            parse_target(AukiPeerTarget {
                domain_id: "00000000-0000-0000-0000-000000000001".into(),
                peer_id: "not-a-peer".into(),
                route: "/ip4/127.0.0.1/tcp/1".into(),
            })
            .is_err()
        );
        assert!(
            parse_target(AukiPeerTarget {
                domain_id: "00000000-0000-0000-0000-000000000001".into(),
                peer_id: "12D3KooWJ5Xw8jCxxbVZXcaUpf7h8fWgpcnH9tGgNfZQ1nSJXUL3".into(),
                route: "not-a-route".into(),
            })
            .is_err()
        );
    }

    #[test]
    fn peer_card_json_round_trip_accepts_compatible_extensions() {
        let card = peer_card();
        let json = peer_card_to_json(card.clone()).expect("encode peer card");

        assert_eq!(peer_card_from_json(json).expect("decode peer card"), card);

        let extended_json = format!(
            r#"{{"version":1,"runtime":"browser","domainId":"{}","peerId":"{}","protocols":["{}"],"routes":{{"tcp":"{}","wss":"{}","future":"ignored"}},"future":true}}"#,
            card.domain_id, card.peer_id, PROTOCOL_ID, card.routes.tcp, card.routes.wss,
        );
        assert_eq!(
            peer_card_from_json(extended_json).expect("decode extended peer card"),
            card
        );
    }

    #[test]
    fn peer_card_rejects_mismatched_relay_routes() {
        let mut card = peer_card();
        let other_peer_id = auki_sdk_rs::Identity::generate().peer_id().to_string();
        card.routes.wss = card.routes.wss.replace(&card.peer_id, &other_peer_id);

        assert!(native_peer_target(card, Some(PROTOCOL_ID.into())).is_err());
    }

    #[test]
    fn peer_card_canonicalizes_domain_ids_and_bounds_json() {
        let mut card = peer_card();
        card.domain_id = "DE66FDF4-A830-4017-95DD-5741C30A6D0F".into();
        let json = peer_card_to_json(card).expect("encode peer card");
        let decoded = peer_card_from_json(json).expect("decode peer card");

        assert_eq!(decoded.domain_id, "de66fdf4-a830-4017-95dd-5741c30a6d0f");
        assert!(peer_card_from_json("x".repeat(16 * 1024 + 1)).is_err());
    }

    #[test]
    fn native_target_uses_the_tcp_route() {
        let card = peer_card();
        let target = native_peer_target(card.clone(), Some(PROTOCOL_ID.into()))
            .expect("select native target");

        assert_eq!(target.domain_id, card.domain_id);
        assert_eq!(target.peer_id, card.peer_id);
        assert_eq!(target.route, card.routes.tcp);
    }
}
