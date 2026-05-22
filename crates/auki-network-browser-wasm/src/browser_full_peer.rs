use auki_network::PeerIdentity;
use auki_network::browser_session_protocol::{
    BrowserMediaPresence, BrowserRosterSnapshot, BrowserSessionParticipant, BrowserSessionSensor,
};
use libp2p::Multiaddr;
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
use libp2p::PeerId;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
const BROWSER_FULL_PEER_TIMEOUT_MS: i32 = 10_000;

#[derive(Debug, Clone)]
pub struct BrowserPeerDebugState {
    pub uses_browser_session: bool,
    pub advertised_multiaddrs: Vec<String>,
    pub membership_peer_count: usize,
}

#[derive(Debug, Clone)]
pub struct BrowserPeerMember {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub participant: Option<BrowserSessionParticipant>,
}

pub struct BrowserFullPeer {
    identity: PeerIdentity,
    advertised_multiaddrs: RefCell<Vec<Multiaddr>>,
    members: RefCell<BTreeMap<String, BrowserPeerMember>>,
    local_participant: RefCell<BrowserSessionParticipant>,
}

impl BrowserFullPeer {
    pub fn new(identity: PeerIdentity, participant: BrowserSessionParticipant) -> Rc<Self> {
        Rc::new(Self {
            identity,
            advertised_multiaddrs: RefCell::new(Vec::new()),
            members: RefCell::new(BTreeMap::new()),
            local_participant: RefCell::new(participant),
        })
    }

    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    pub fn set_advertised_multiaddrs(&self, addrs: Vec<Multiaddr>) {
        self.advertised_multiaddrs.replace(addrs);
    }

    pub fn advertised_multiaddrs(&self) -> Vec<Multiaddr> {
        self.advertised_multiaddrs.borrow().clone()
    }

    pub fn debug_state(&self) -> BrowserPeerDebugState {
        BrowserPeerDebugState {
            uses_browser_session: false,
            advertised_multiaddrs: self
                .advertised_multiaddrs
                .borrow()
                .iter()
                .map(ToString::to_string)
                .collect(),
            membership_peer_count: self.members.borrow().len(),
        }
    }

    pub fn update_local_participant(&self, participant: BrowserSessionParticipant) {
        self.local_participant.replace(participant);
    }

    pub fn set_local_sensors(&self, sensors: Vec<BrowserSessionSensor>) {
        self.local_participant.borrow_mut().sensors = sensors;
    }

    pub fn set_local_media(&self, media: BrowserMediaPresence) {
        self.local_participant.borrow_mut().media_presence = media;
    }

    pub fn apply_membership_json(&self, membership_json: &str) -> Result<(), String> {
        #[derive(serde::Deserialize)]
        struct Membership {
            peers: Vec<Member>,
        }

        #[derive(serde::Deserialize)]
        struct Member {
            peer_id: String,
            multiaddrs: Vec<String>,
        }

        let membership: Membership = serde_json::from_str(membership_json)
            .map_err(|err| format!("membership json decode failed: {err}"))?;
        let local_peer_id = self.peer_id();
        let mut members = BTreeMap::new();
        for member in membership.peers {
            let participant = (member.peer_id == local_peer_id)
                .then(|| self.local_participant.borrow().clone());
            members.insert(
                member.peer_id.clone(),
                BrowserPeerMember {
                    peer_id: member.peer_id,
                    multiaddrs: member.multiaddrs,
                    participant,
                },
            );
        }
        self.members.replace(members);
        Ok(())
    }

    pub fn roster_snapshot(
        &self,
        domain_name: String,
        manager_peer_id: String,
    ) -> BrowserRosterSnapshot {
        let local_peer_id = self.peer_id();
        let mut participants = Vec::new();
        for member in self.members.borrow().values() {
            let mut participant = member.participant.clone().unwrap_or_else(|| {
                BrowserSessionParticipant {
                    peer_id: member.peer_id.clone(),
                    app_id: if member.peer_id == manager_peer_id {
                        "auki-network".to_string()
                    } else {
                        "auki-browser-peer".to_string()
                    },
                    display_name: member.peer_id.clone(),
                    is_self: false,
                    connected: true,
                    sensors: Vec::new(),
                    media_presence: BrowserMediaPresence::default(),
                }
            });
            participant.is_self = participant.peer_id == local_peer_id;
            participants.push(participant);
        }
        BrowserRosterSnapshot {
            self_peer_id: local_peer_id,
            domain_name,
            manager_peer_id,
            election_state: "stable".to_string(),
            participants,
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[derive(libp2p::swarm::NetworkBehaviour)]
struct BrowserFullPeerBehaviour {
    stream: libp2p_stream::Behaviour,
    identify: libp2p::identify::Behaviour,
    relay_client: libp2p::relay::client::Behaviour,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
impl BrowserFullPeer {
    pub async fn join_via_relayed_peer(
        self: Rc<Self>,
        manager_address: Multiaddr,
        relay_address: Multiaddr,
    ) -> Result<auki_network::join_protocol::JoinResponse, String> {
        use futures::StreamExt as _;
        use libp2p::{
            SwarmBuilder, identify, noise,
            core::{Transport as _, muxing::StreamMuxerBox, upgrade},
            multiaddr::Protocol,
            swarm::dial_opts::DialOpts,
            yamux,
        };

        let manager_peer_id = peer_id_from_multiaddr(&manager_address)?;
        let relay_peer_id = peer_id_from_multiaddr(&relay_address)?;
        let advertised = relay_address
            .clone()
            .with(Protocol::P2pCircuit)
            .with(Protocol::P2p(self.identity.peer_id()));
        self.set_advertised_multiaddrs(vec![advertised.clone()]);

        let response = crate::dial_browser_join_inner(
            self.identity.clone(),
            manager_address,
            vec![advertised],
            manager_peer_id.to_string(),
        )
        .await?;

        let mut swarm = SwarmBuilder::with_existing_identity(self.identity.keypair().clone())
            .with_wasm_bindgen()
            .with_other_transport(|keypair| {
                let webrtc = libp2p::webrtc_websys::Transport::new(
                    libp2p::webrtc_websys::Config::new(keypair),
                )
                .boxed();
                let websocket = libp2p::websocket_websys::Transport::default()
                    .upgrade(upgrade::Version::V1Lazy)
                    .authenticate(
                        noise::Config::new(keypair)
                            .expect("browser websocket noise config should build"),
                    )
                    .multiplex(yamux::Config::default())
                    .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
                    .boxed();
                webrtc
                    .or_transport(websocket)
                    .map(|either, _| either.into_inner())
                    .boxed()
            })
            .map_err(|err| format!("transport setup failed: {err}"))?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|err| format!("relay client setup failed: {err}"))?
            .with_behaviour(|key, relay_client| BrowserFullPeerBehaviour {
                stream: libp2p_stream::Behaviour::new(),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/auki/browser-full-peer/0.0.1".into(),
                    key.public(),
                )),
                relay_client,
            })
            .map_err(|err| format!("behaviour setup failed: {err}"))?
            .build();

        swarm
            .dial(
                DialOpts::peer_id(relay_peer_id)
                    .addresses(vec![relay_address.clone()])
                    .build(),
            )
            .map_err(|err| format!("relay dial setup failed: {err}"))?;

        wait_for_peer_connection(&mut swarm, relay_peer_id, "relay").await?;

        let circuit_addr = relay_address.clone().with(Protocol::P2pCircuit);
        swarm
            .listen_on(circuit_addr)
            .map_err(|err| format!("relay listen setup failed: {err}"))?;
        let advertised = wait_for_relay_address(&mut swarm, self.identity.peer_id()).await?;
        self.set_advertised_multiaddrs(vec![advertised]);

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let _ = swarm.select_next_some().await;
            }
        });

        if let auki_network::join_protocol::JoinResponse::Accept {
            membership_json, ..
        } = &response
        {
            self.apply_membership_json(membership_json)?;
        }

        Ok(response)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn wait_for_peer_connection(
    swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
    remote_peer_id: PeerId,
    label: &'static str,
) -> Result<(), String> {
    use futures::{FutureExt as _, StreamExt as _, select};
    use libp2p::swarm::SwarmEvent;

    let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
    futures::pin_mut!(timeout);
    loop {
        select! {
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("{label} connection timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("{label} connection timeout setup failed: {err}")),
                };
            }
            event = swarm.select_next_some().fuse() => {
                match event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == remote_peer_id => {
                        return Ok(());
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id: Some(peer), error, .. } if peer == remote_peer_id => {
                        return Err(format!("{label} dial failure for {peer}: {error}"));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn wait_for_relay_address(
    swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
    local_peer_id: PeerId,
) -> Result<Multiaddr, String> {
    use futures::{FutureExt as _, StreamExt as _, select};
    use libp2p::{multiaddr::Protocol, swarm::SwarmEvent};

    let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
    futures::pin_mut!(timeout);
    loop {
        select! {
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("relay reservation timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("relay reservation timeout setup failed: {err}")),
                };
            }
            event = swarm.select_next_some().fuse() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) => {
                        return Ok(with_peer_id(address, local_peer_id));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn peer_id_from_multiaddr(address: &Multiaddr) -> Result<PeerId, String> {
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
fn with_peer_id(address: Multiaddr, peer_id: PeerId) -> Multiaddr {
    use libp2p::multiaddr::Protocol;

    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(existing) if existing == peer_id))
    {
        address
    } else {
        address.with(Protocol::P2p(peer_id))
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn js_timeout(ms: i32) -> Result<(), String> {
    use wasm_bindgen::JsValue;
    use wasm_bindgen::JsCast as _;

    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let Some(window) = web_sys::window() else {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("window unavailable"),
            );
            return;
        };
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::once(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        if window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                ms,
            )
            .is_err()
        {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("setTimeout failed"),
            );
            return;
        }
        closure.forget();
    });

    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|err| {
            err.as_string().unwrap_or_else(|| {
                js_sys::Reflect::get(&err, &JsValue::from_str("message"))
                    .ok()
                    .and_then(|message| message.as_string())
                    .unwrap_or_else(|| "timeout promise failed".to_string())
            })
        })
}
