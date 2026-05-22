#![allow(dead_code)]

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserAudioPublication {
    Off,
    Generated,
    Microphone,
}

pub struct BrowserFullPeer {
    identity: PeerIdentity,
    advertised_multiaddrs: RefCell<Vec<Multiaddr>>,
    members: RefCell<BTreeMap<String, BrowserPeerMember>>,
    local_participant: RefCell<BrowserSessionParticipant>,
    audio_publication: RefCell<BrowserAudioPublication>,
    #[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
    stream_control: RefCell<Option<libp2p_stream::Control>>,
}

impl BrowserFullPeer {
    pub fn new(identity: PeerIdentity, participant: BrowserSessionParticipant) -> Rc<Self> {
        Rc::new(Self {
            identity,
            advertised_multiaddrs: RefCell::new(Vec::new()),
            members: RefCell::new(BTreeMap::new()),
            local_participant: RefCell::new(participant),
            audio_publication: RefCell::new(BrowserAudioPublication::Off),
            #[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
            stream_control: RefCell::new(None),
        })
    }

    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    pub fn set_advertised_multiaddrs(&self, addrs: Vec<Multiaddr>) {
        self.advertised_multiaddrs.replace(addrs);
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn set_local_sensors(&self, sensors: Vec<BrowserSessionSensor>) {
        self.local_participant.borrow_mut().sensors = sensors;
    }

    pub fn set_local_media(&self, media: BrowserMediaPresence) {
        self.local_participant.borrow_mut().media_presence = media;
    }

    pub fn set_audio_publication(&self, publication: BrowserAudioPublication) {
        self.audio_publication.replace(publication);
    }

    pub fn audio_publication(&self) -> BrowserAudioPublication {
        *self.audio_publication.borrow()
    }

    pub fn member_multiaddrs(&self, peer_id: &str) -> Vec<Multiaddr> {
        self.members
            .borrow()
            .get(peer_id)
            .map(|member| {
                member
                    .multiaddrs
                    .iter()
                    .filter_map(|addr| addr.parse::<Multiaddr>().ok())
                    .collect()
            })
            .unwrap_or_default()
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
            let participant =
                (member.peer_id == local_peer_id).then(|| self.local_participant.borrow().clone());
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
            let mut participant =
                member
                    .participant
                    .clone()
                    .unwrap_or_else(|| BrowserSessionParticipant {
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
    pub fn stream_control(&self) -> Option<libp2p_stream::Control> {
        self.stream_control.borrow().clone()
    }

    fn set_stream_control(&self, control: libp2p_stream::Control) {
        self.stream_control.replace(Some(control));
    }

    pub async fn join_via_relayed_peer(
        self: Rc<Self>,
        manager_address: Multiaddr,
        relay_address: Multiaddr,
    ) -> Result<auki_network::join_protocol::JoinResponse, String> {
        use futures::{FutureExt as _, StreamExt as _, select};
        use libp2p::{
            StreamProtocol, SwarmBuilder,
            core::{Transport as _, muxing::StreamMuxerBox, upgrade},
            identify,
            multiaddr::Protocol,
            noise,
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

        if let auki_network::join_protocol::JoinResponse::Accept {
            membership_json, ..
        } = &response
        {
            self.apply_membership_json(membership_json)?;
        }

        let local_peer_id = self.identity.peer_id();
        for member in self.members.borrow().values() {
            let Ok(peer_id) = member.peer_id.parse::<PeerId>() else {
                continue;
            };
            if peer_id == local_peer_id {
                continue;
            }
            let dial_addresses = self.member_multiaddrs(&member.peer_id);
            for mut addr in self.member_multiaddrs(&member.peer_id) {
                if addr.iter().last().is_some_and(
                    |protocol| matches!(protocol, Protocol::P2p(peer) if peer == peer_id),
                ) {
                    let _ = addr.pop();
                }
                swarm.add_peer_address(peer_id, addr);
            }
            if !dial_addresses.is_empty() {
                let _ = swarm.dial(DialOpts::peer_id(peer_id).addresses(dial_addresses).build());
            }
        }

        let mut stream_control = swarm.behaviour().stream.new_control();
        let mut stream_listener = stream_control
            .accept(StreamProtocol::new(crate::browser_stream::STREAM_PROTOCOL))
            .map_err(|err| format!("stream protocol accept setup failed: {err}"))?;
        self.set_stream_control(stream_control);
        let full_peer = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                select! {
                    incoming = stream_listener.next().fuse() => {
                        let Some((peer, stream)) = incoming else {
                            continue;
                        };
                        let full_peer = full_peer.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            full_peer.handle_inbound_stream(peer, stream).await;
                        });
                    }
                    _event = swarm.select_next_some().fuse() => {}
                }
            }
        });

        Ok(response)
    }

    async fn handle_inbound_stream(self: Rc<Self>, _peer: PeerId, mut stream: libp2p::Stream) {
        use crate::browser_stream::{
            DeclineReason, StreamEntry, StreamManifest, StreamMessage, StreamRequest, read_message,
            stream_message, write_message,
        };
        use prost::Message as _;

        let request = match read_message(&mut stream).await {
            Ok(message) => match message.variant {
                Some(stream_message::Variant::Request(request)) => request,
                _ => return,
            },
            Err(_) => return,
        };

        if self.audio_publication() != BrowserAudioPublication::Generated
            || request
                != (StreamRequest {
                    sensor_id: "audio".to_string(),
                })
        {
            let _ = write_message(
                &mut stream,
                &StreamMessage::decline(DeclineReason::sensor_unavailable()),
            )
            .await;
            return;
        }

        let accept = StreamMessage::accept(StreamManifest {
            sensor_id: "audio".to_string(),
            sensor_hash: String::new(),
            clock_id: String::new(),
            clock_hash: String::new(),
            frame_id: String::new(),
            frame_hash: String::new(),
        });
        if write_message(&mut stream, &accept).await.is_err() {
            return;
        }

        for frame_index in 0..250_u32 {
            let payload = crate::browser_stream::audio::Data {
                data: crate::browser_audio::generated_audio_frame(frame_index),
            }
            .encode_to_vec();
            let entry = StreamMessage::entry(StreamEntry {
                timestamp_ns: (js_sys::Date::now() * 1_000_000.0) as i64,
                seq: u64::from(frame_index),
                payload,
            });
            if write_message(&mut stream, &entry).await.is_err() {
                return;
            }
            let _ = js_timeout(crate::browser_audio::AUDIO_FRAME_MS as i32).await;
        }
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
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::JsValue;

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
            let _ = reject.call1(&JsValue::UNDEFINED, &JsValue::from_str("setTimeout failed"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use auki_identity::Wallet;

    fn full_peer() -> Rc<BrowserFullPeer> {
        let wallet = Wallet::from_seed(vec![7u8; 32]).expect("32-byte seed");
        let identity = PeerIdentity::from_wallet(wallet);
        let peer_id = identity.peer_id().to_string();
        BrowserFullPeer::new(
            identity,
            BrowserSessionParticipant {
                peer_id,
                app_id: "park".to_string(),
                display_name: "Browser Peer".to_string(),
                is_self: true,
                connected: true,
                sensors: Vec::new(),
                media_presence: BrowserMediaPresence::default(),
            },
        )
    }

    #[test]
    fn audio_publication_defaults_off() {
        assert_eq!(
            full_peer().audio_publication(),
            BrowserAudioPublication::Off
        );
    }

    #[test]
    fn enables_generated_audio_publication_for_smokes() {
        let peer = full_peer();
        peer.set_audio_publication(BrowserAudioPublication::Generated);
        assert_eq!(peer.audio_publication(), BrowserAudioPublication::Generated);
    }
}
