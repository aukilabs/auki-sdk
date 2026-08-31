//! Compile-proven browser libp2p transport boundary.
//!
//! This is deliberately private until DDS authority installation, WSS relay
//! admission, reservation lifecycle, and stream opening form one usable node.

use std::error::Error as StdError;

use libp2p::{
    core::{upgrade, Transport as _},
    noise, relay,
    swarm::NetworkBehaviour,
    websocket_websys, yamux, Swarm, SwarmBuilder,
};

use crate::{Error, Identity, PeerId, Result};

#[derive(NetworkBehaviour)]
pub(crate) struct BrowserBehaviour {
    relay: relay::client::Behaviour,
    streams: libp2p_stream::Behaviour,
}

/// Private compile spike proving that the SDK identity can own a browser swarm.
pub(crate) struct BrowserNode {
    peer_id: PeerId,
    swarm: Swarm<BrowserBehaviour>,
}

impl BrowserNode {
    pub(crate) fn new(identity: Identity) -> Result<Self> {
        let peer_id = identity.peer_id();
        let swarm = build_swarm(identity.keypair())
            .map_err(|error| Error::TransportBuild(error.to_string()))?;
        Ok(Self { peer_id, swarm })
    }

    pub(crate) fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub(crate) fn swarm_mut(&mut self) -> &mut Swarm<BrowserBehaviour> {
        &mut self.swarm
    }
}

fn build_swarm(
    identity: libp2p::identity::Keypair,
) -> std::result::Result<Swarm<BrowserBehaviour>, Box<dyn StdError + Send + Sync>> {
    Ok(SwarmBuilder::with_existing_identity(identity)
        .with_wasm_bindgen()
        .with_other_transport(|identity| {
            websocket_websys::Transport::default()
                .upgrade(upgrade::Version::V1Lazy)
                .authenticate(
                    noise::Config::new(identity).expect("Ed25519 identity supports Noise"),
                )
                .multiplex(yamux::Config::default())
        })?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_identity, relay| BrowserBehaviour {
            relay,
            streams: libp2p_stream::Behaviour::new(),
        })?
        .build())
}
