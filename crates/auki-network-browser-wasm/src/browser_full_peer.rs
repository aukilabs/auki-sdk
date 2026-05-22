use auki_network::PeerIdentity;
use auki_network::browser_session_protocol::{
    BrowserMediaPresence, BrowserRosterSnapshot, BrowserSessionParticipant, BrowserSessionSensor,
};
use auki_network::sensors_protocol::SensorEntry;
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
    pub info_protocol_peer_count: usize,
    pub sensor_catalog_protocol_peer_count: usize,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct BrowserPeerMember {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub participant: Option<BrowserSessionParticipant>,
    pub info_protocol_fetched: bool,
    pub sensor_catalog_protocol_fetched: bool,
}

pub struct BrowserFullPeer {
    identity: PeerIdentity,
    advertised_multiaddrs: RefCell<Vec<Multiaddr>>,
    members: RefCell<BTreeMap<String, BrowserPeerMember>>,
    local_participant: RefCell<BrowserSessionParticipant>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
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
            info_protocol_peer_count: self
                .members
                .borrow()
                .values()
                .filter(|member| member.info_protocol_fetched)
                .count(),
            sensor_catalog_protocol_peer_count: self
                .members
                .borrow()
                .values()
                .filter(|member| member.sensor_catalog_protocol_fetched)
                .count(),
        }
    }

    pub fn update_local_participant(&self, participant: BrowserSessionParticipant) {
        self.local_participant.replace(participant);
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
        let previous = self.members.borrow().clone();
        let mut members = BTreeMap::new();
        for member in membership.peers {
            let previous_member = previous.get(&member.peer_id);
            let participant = if member.peer_id == local_peer_id {
                Some(self.local_participant.borrow().clone())
            } else {
                previous_member.and_then(|member| member.participant.clone())
            };
            members.insert(
                member.peer_id.clone(),
                BrowserPeerMember {
                    peer_id: member.peer_id,
                    multiaddrs: member.multiaddrs,
                    participant,
                    info_protocol_fetched: previous_member
                        .is_some_and(|member| member.info_protocol_fetched),
                    sensor_catalog_protocol_fetched: previous_member
                        .is_some_and(|member| member.sensor_catalog_protocol_fetched),
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

    fn local_sensor_entries(&self) -> Vec<SensorEntry> {
        self.local_participant
            .borrow()
            .sensors
            .iter()
            .map(|sensor| SensorEntry {
                sensor_id: sensor.id.clone(),
                sensor_hash: String::new(),
                kind: sensor.kind.clone(),
                sensor_entry_json: None,
                frame_entry_json: None,
            })
            .collect()
    }

    fn apply_protocol_catalog(
        &self,
        peer_id: &str,
        info: Option<auki_network::ParticipantInfo>,
        sensors: Vec<SensorEntry>,
    ) {
        let mut members = self.members.borrow_mut();
        let Some(member) = members.get_mut(peer_id) else {
            return;
        };
        let mut participant =
            member
                .participant
                .clone()
                .unwrap_or_else(|| BrowserSessionParticipant {
                    peer_id: peer_id.to_string(),
                    app_id: "auki-browser-peer".to_string(),
                    display_name: peer_id.to_string(),
                    is_self: false,
                    connected: true,
                    sensors: Vec::new(),
                    media_presence: BrowserMediaPresence::default(),
                });
        if let Some(info) = info {
            participant.app_id = info.app;
            participant.display_name = info.name;
            member.info_protocol_fetched = true;
        }
        participant.sensors = sensors.into_iter().map(browser_sensor_from_entry).collect();
        if participant
            .sensors
            .iter()
            .any(|sensor| sensor.kind == "audio")
        {
            participant.media_presence.mic_available = true;
        }
        member.sensor_catalog_protocol_fetched = true;
        member.participant = Some(participant);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn browser_sensor_from_entry(entry: SensorEntry) -> BrowserSessionSensor {
    BrowserSessionSensor {
        label: if entry.sensor_id.is_empty() {
            entry.kind.clone()
        } else {
            entry.sensor_id.clone()
        },
        id: entry.sensor_id,
        kind: entry.kind,
        publishable: true,
        subscribable: true,
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

        let mut info_listener = swarm
            .behaviour()
            .stream
            .new_control()
            .accept(StreamProtocol::new(
                auki_network::info_protocol::INFO_PROTOCOL,
            ))
            .map_err(|err| format!("info protocol accept setup failed: {err}"))?;
        let mut sensors_listener = swarm
            .behaviour()
            .stream
            .new_control()
            .accept(StreamProtocol::new(
                auki_network::sensors_protocol::SENSORS_PROTOCOL,
            ))
            .map_err(|err| format!("sensors protocol accept setup failed: {err}"))?;

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
            let _ = self
                .clone()
                .fetch_remote_peer_catalogs(
                    &mut swarm,
                    &mut info_listener,
                    &mut sensors_listener,
                    manager_peer_id,
                )
                .await;
        }

        let background_peer = self.clone();
        let manager_peer_id = manager_peer_id.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                select! {
                    inbound = info_listener.next().fuse() => {
                        if let Some((_peer, stream)) = inbound {
                            background_peer.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                        }
                    }
                    inbound = sensors_listener.next().fuse() => {
                        if let Some((_peer, stream)) = inbound {
                            background_peer.clone().spawn_sensors_handler(stream);
                        }
                    }
                    _event = swarm.select_next_some().fuse() => {}
                }
            }
        });

        Ok(response)
    }

    async fn fetch_remote_peer_catalogs<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: PeerId,
    ) -> Result<(), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        let local_peer_id = self.identity.peer_id();
        let targets = self
            .members
            .borrow()
            .values()
            .filter_map(|member| {
                let peer_id: PeerId = member.peer_id.parse().ok()?;
                if peer_id == local_peer_id || peer_id == manager_peer_id {
                    return None;
                }
                let multiaddrs = member
                    .multiaddrs
                    .iter()
                    .filter_map(|addr| addr.parse::<Multiaddr>().ok())
                    .collect::<Vec<_>>();
                (!multiaddrs.is_empty()).then_some((peer_id, multiaddrs))
            })
            .collect::<Vec<_>>();

        let mut pending = targets;
        let mut last_error = None;
        for attempt in 0..5 {
            let mut next_pending = Vec::new();
            for (peer_id, multiaddrs) in pending {
                match self
                    .clone()
                    .fetch_peer_catalog(
                        swarm,
                        info_listener,
                        sensors_listener,
                        manager_peer_id.to_string(),
                        peer_id,
                        multiaddrs.clone(),
                    )
                    .await
                {
                    Ok((info, sensors)) => {
                        self.apply_protocol_catalog(&peer_id.to_string(), Some(info), sensors);
                    }
                    Err(err) => {
                        last_error = Some(err);
                        next_pending.push((peer_id, multiaddrs));
                    }
                }
            }
            if next_pending.is_empty() {
                return Ok(());
            }
            pending = next_pending;
            if attempt < 4 {
                self.clone()
                    .delay_with_inbound(
                        swarm,
                        info_listener,
                        sensors_listener,
                        manager_peer_id.to_string(),
                        250,
                    )
                    .await?;
            }
        }

        Err(last_error.unwrap_or_else(|| "remote browser catalog fetch failed".to_string()))
    }

    async fn fetch_peer_catalog<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        remote_peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
    ) -> Result<(auki_network::ParticipantInfo, Vec<SensorEntry>), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        self.clone()
            .ensure_peer_connection(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
                multiaddrs,
            )
            .await?;
        let info = self
            .clone()
            .request_info(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
            )
            .await?;
        let sensors = self
            .request_sensors(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id,
                remote_peer_id,
            )
            .await?;
        Ok((info, sensors))
    }

    async fn ensure_peer_connection<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        remote_peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
    ) -> Result<(), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use futures::{FutureExt as _, StreamExt as _, select};
        use libp2p::swarm::{SwarmEvent, dial_opts::DialOpts};

        if swarm.is_connected(&remote_peer_id) {
            return Ok(());
        }
        swarm
            .dial(
                DialOpts::peer_id(remote_peer_id)
                    .addresses(multiaddrs)
                    .build(),
            )
            .map_err(|err| format!("remote browser dial setup failed: {err}"))?;

        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(timeout);
        loop {
            select! {
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("remote browser connection timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("remote browser connection timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                event = swarm.select_next_some().fuse() => {
                    match event {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == remote_peer_id => {
                            return Ok(());
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id: Some(peer), error, .. } if peer == remote_peer_id => {
                            return Err(format!("remote browser dial failure for {peer}: {error}"));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn delay_with_inbound<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        delay_ms: i32,
    ) -> Result<(), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use futures::{FutureExt as _, StreamExt as _, select};

        let timeout = js_timeout(delay_ms).fuse();
        futures::pin_mut!(timeout);
        loop {
            select! {
                timeout_result = timeout => return timeout_result,
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    async fn request_info<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        remote_peer_id: PeerId,
    ) -> Result<auki_network::ParticipantInfo, String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::info_protocol::InfoRequest;

        let mut stream = self
            .clone()
            .open_protocol_stream(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
                auki_network::info_protocol::INFO_PROTOCOL,
                "info",
            )
            .await?;
        self.clone()
            .write_info_request_with_inbound(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
                &mut stream,
                &InfoRequest::default(),
            )
            .await?;
        let response = self
            .read_info_response_with_inbound(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id,
                remote_peer_id,
                &mut stream,
            )
            .await?;
        serde_json::from_str(&response.participant_info_json)
            .map_err(|err| format!("participant info json decode failed: {err}"))
    }

    async fn request_sensors<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        remote_peer_id: PeerId,
    ) -> Result<Vec<SensorEntry>, String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::sensors_protocol::SensorsRequest;

        let mut stream = self
            .clone()
            .open_protocol_stream(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
                auki_network::sensors_protocol::SENSORS_PROTOCOL,
                "sensors",
            )
            .await?;
        self.clone()
            .write_sensors_request_with_inbound(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id.clone(),
                remote_peer_id,
                &mut stream,
                &SensorsRequest::catalog(),
            )
            .await?;
        let response = self
            .read_sensors_response_with_inbound(
                swarm,
                info_listener,
                sensors_listener,
                manager_peer_id,
                remote_peer_id,
                &mut stream,
            )
            .await?;
        Ok(response.sensors)
    }

    async fn open_protocol_stream<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        remote_peer_id: PeerId,
        protocol: &'static str,
        label: &'static str,
    ) -> Result<libp2p::Stream, String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use futures::{FutureExt as _, StreamExt as _, select};
        use libp2p::{StreamProtocol, swarm::SwarmEvent};

        let mut control = swarm.behaviour().stream.new_control();
        let proto = StreamProtocol::new(protocol);
        let open = control.open_stream(remote_peer_id, proto).fuse();
        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(open, timeout);

        loop {
            select! {
                result = open => {
                    return result.map_err(|err| format!("open {label} stream failed: {err}"));
                }
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("{label} stream open timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("{label} stream open timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                event = swarm.select_next_some().fuse() => {
                    if let SwarmEvent::OutgoingConnectionError { peer_id: Some(peer), error, .. } = event {
                        if peer == remote_peer_id {
                            return Err(format!("{label} stream dial failure for {peer}: {error}"));
                        }
                    }
                }
            }
        }
    }

    async fn write_info_request_with_inbound<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        _remote_peer_id: PeerId,
        stream: &mut libp2p::Stream,
        request: &auki_network::info_protocol::InfoRequest,
    ) -> Result<(), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::info_protocol::write_info_request;
        use futures::{FutureExt as _, StreamExt as _, select};

        let write = write_info_request(stream, request).fuse();
        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(write, timeout);
        loop {
            select! {
                result = write => return result.map_err(|err| format!("write info request failed: {err}")),
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("info request write timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("info request write timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    async fn read_info_response_with_inbound<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        _remote_peer_id: PeerId,
        stream: &mut libp2p::Stream,
    ) -> Result<auki_network::info_protocol::InfoResponse, String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::info_protocol::read_info_response;
        use futures::{FutureExt as _, StreamExt as _, select};

        let read = read_info_response(stream).fuse();
        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(read, timeout);
        loop {
            select! {
                result = read => return result.map_err(|err| format!("read info response failed: {err}")),
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("info response read timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("info response read timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    async fn write_sensors_request_with_inbound<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        _remote_peer_id: PeerId,
        stream: &mut libp2p::Stream,
        request: &auki_network::sensors_protocol::SensorsRequest,
    ) -> Result<(), String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::sensors_protocol::write_sensors_request;
        use futures::{FutureExt as _, StreamExt as _, select};

        let write = write_sensors_request(stream, request).fuse();
        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(write, timeout);
        loop {
            select! {
                result = write => return result.map_err(|err| format!("write sensors request failed: {err}")),
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("sensors request write timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("sensors request write timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    async fn read_sensors_response_with_inbound<I, S>(
        self: Rc<Self>,
        swarm: &mut libp2p::Swarm<BrowserFullPeerBehaviour>,
        info_listener: &mut I,
        sensors_listener: &mut S,
        manager_peer_id: String,
        _remote_peer_id: PeerId,
        stream: &mut libp2p::Stream,
    ) -> Result<auki_network::sensors_protocol::SensorsResponse, String>
    where
        I: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
        S: futures::Stream<Item = (PeerId, libp2p::Stream)> + Unpin,
    {
        use auki_network::sensors_protocol::read_sensors_response;
        use futures::{FutureExt as _, StreamExt as _, select};

        let read = read_sensors_response(stream).fuse();
        let timeout = js_timeout(BROWSER_FULL_PEER_TIMEOUT_MS).fuse();
        futures::pin_mut!(read, timeout);
        loop {
            select! {
                result = read => return result.map_err(|err| format!("read sensors response failed: {err}")),
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("sensors response read timed out after {BROWSER_FULL_PEER_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("sensors response read timeout setup failed: {err}")),
                    };
                }
                inbound = info_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_info_handler(manager_peer_id.clone(), stream);
                    }
                }
                inbound = sensors_listener.next().fuse() => {
                    if let Some((_peer, stream)) = inbound {
                        self.clone().spawn_sensors_handler(stream);
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    fn spawn_info_handler(self: Rc<Self>, manager_peer_id: String, mut stream: libp2p::Stream) {
        wasm_bindgen_futures::spawn_local(async move {
            use auki_network::info_protocol::{
                InfoResponse, read_info_request, write_info_response,
            };

            if read_info_request(&mut stream).await.is_err() {
                return;
            }
            let Ok(participant_info_json) =
                serde_json::to_string(&self.local_participant_info(&manager_peer_id))
            else {
                return;
            };
            let _ = write_info_response(
                &mut stream,
                &InfoResponse {
                    participant_info_json,
                },
            )
            .await;
        });
    }

    fn spawn_sensors_handler(self: Rc<Self>, mut stream: libp2p::Stream) {
        wasm_bindgen_futures::spawn_local(async move {
            use auki_network::sensors_protocol::{
                SensorsResponse, read_sensors_request, write_sensors_response,
            };

            if read_sensors_request(&mut stream).await.is_err() {
                return;
            }
            let _ = write_sensors_response(
                &mut stream,
                &SensorsResponse {
                    sensors: self.local_sensor_entries(),
                },
            )
            .await;
        });
    }

    fn local_participant_info(&self, manager_peer_id: &str) -> auki_network::ParticipantInfo {
        let participant = self.local_participant.borrow();
        auki_network::ParticipantInfo {
            app: participant.app_id.clone(),
            name: participant.display_name.clone(),
            session_id: format!("browser-{}", self.peer_id()),
            session_clock_id: format!("{}/browser-clock", self.peer_id()),
            session_clock_hash: String::new(),
            session_now_ns: 0,
            cluster_joined_at_ns: Some(0),
            peer_id: self.identity.peer_id(),
            app_instance: "browser".to_string(),
            is_manager: false,
            manager_peer_id: manager_peer_id.to_string(),
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
