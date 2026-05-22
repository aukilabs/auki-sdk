use auki_identity::Wallet;
use auki_network::PeerIdentity;
#[cfg(feature = "browser_libp2p")]
mod browser_audio;
#[cfg(feature = "browser_libp2p")]
mod browser_full_peer;
#[cfg(feature = "browser_libp2p")]
mod browser_stream;
#[cfg(feature = "browser_libp2p")]
use auki_network::browser_session_protocol::{
    BrowserMediaPresence, BrowserRosterSnapshot, BrowserSessionClientMessage,
    BrowserSessionParticipant, BrowserSessionSensor,
};
#[cfg(feature = "browser_libp2p")]
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
const BROWSER_PROBE_TIMEOUT_MS: i32 = 10_000;
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
const BROWSER_JOIN_TIMEOUT_MS: i32 = 10_000;

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

#[cfg(feature = "browser_libp2p")]
#[wasm_bindgen]
pub struct BrowserDomainSession {
    inner: auki_domain::browser_session::BrowserDomainSession,
    state: Rc<BrowserDomainSessionState>,
    full_peer: Rc<browser_full_peer::BrowserFullPeer>,
}

#[cfg(feature = "browser_libp2p")]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserMetadata {
    app_id: String,
    display_name: String,
}

#[cfg(feature = "browser_libp2p")]
impl Default for BrowserMetadata {
    fn default() -> Self {
        Self {
            app_id: "park".to_string(),
            display_name: "Browser Peer".to_string(),
        }
    }
}

#[cfg(feature = "browser_libp2p")]
struct BrowserDomainSessionState {
    peer_id: String,
    metadata: RefCell<BrowserMetadata>,
    sensors: RefCell<Vec<BrowserSessionSensor>>,
    media_presence: RefCell<BrowserMediaPresence>,
    snapshot: RefCell<Option<BrowserRosterSnapshot>>,
    observers: RefCell<BTreeMap<u32, js_sys::Function>>,
    next_observer_id: Cell<u32>,
    sender: RefCell<Option<futures::channel::mpsc::UnboundedSender<BrowserSessionClientMessage>>>,
}

#[cfg(feature = "browser_libp2p")]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl BrowserDomainSessionState {
    fn new(peer_id: String) -> Self {
        Self {
            metadata: RefCell::new(BrowserMetadata {
                app_id: "park".to_string(),
                display_name: peer_id.clone(),
            }),
            peer_id,
            sensors: RefCell::new(Vec::new()),
            media_presence: RefCell::new(BrowserMediaPresence::default()),
            snapshot: RefCell::new(None),
            observers: RefCell::new(BTreeMap::new()),
            next_observer_id: Cell::new(1),
            sender: RefCell::new(None),
        }
    }

    fn local_participant(&self) -> BrowserSessionParticipant {
        browser_session_participant(
            self.peer_id.clone(),
            self.metadata.borrow().clone(),
            self.sensors.borrow().clone(),
            self.media_presence.borrow().clone(),
            true,
        )
    }

    fn queue(&self, message: BrowserSessionClientMessage) -> bool {
        self.sender
            .borrow()
            .as_ref()
            .is_some_and(|sender| sender.unbounded_send(message).is_ok())
    }

    fn set_sender(
        &self,
        sender: futures::channel::mpsc::UnboundedSender<BrowserSessionClientMessage>,
    ) {
        self.sender.replace(Some(sender));
    }

    fn clear_sender(&self) {
        self.sender.replace(None);
    }

    fn clear_snapshot(&self) {
        self.snapshot.replace(None);
        self.emit();
    }

    fn apply_snapshot(&self, mut snapshot: BrowserRosterSnapshot) {
        for participant in &mut snapshot.participants {
            participant.is_self = participant.peer_id == self.peer_id;
        }
        self.snapshot.replace(Some(snapshot));
        self.emit();
    }

    fn update_local_snapshot(&self) {
        let Some(mut snapshot) = self.snapshot.borrow().clone() else {
            return;
        };
        let local = self.local_participant();
        for participant in &mut snapshot.participants {
            if participant.peer_id == self.peer_id {
                *participant = local.clone();
            }
            participant.is_self = participant.peer_id == self.peer_id;
        }
        self.snapshot.replace(Some(snapshot));
        self.emit();
    }

    fn emit(&self) {
        let snapshot = self.snapshot_js_value();
        for observer in self.observers.borrow().values() {
            let _ = observer.call1(&JsValue::NULL, &snapshot);
        }
    }

    fn snapshot_js_value(&self) -> JsValue {
        if let Some(snapshot) = self.snapshot.borrow().as_ref() {
            return serde_wasm_bindgen::to_value(snapshot).unwrap_or(JsValue::NULL);
        }
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct IdleSnapshot<'a> {
            self_peer_id: &'a str,
            domain_name: Option<&'a str>,
            participants: Vec<BrowserSessionParticipant>,
            manager_peer_id: Option<&'a str>,
            election_state: &'static str,
        }
        serde_wasm_bindgen::to_value(&IdleSnapshot {
            self_peer_id: &self.peer_id,
            domain_name: None,
            participants: Vec::new(),
            manager_peer_id: None,
            election_state: "unknown",
        })
        .unwrap_or(JsValue::NULL)
    }
}

#[cfg(feature = "browser_libp2p")]
fn browser_session_participant(
    peer_id: String,
    metadata: BrowserMetadata,
    sensors: Vec<BrowserSessionSensor>,
    mut media_presence: BrowserMediaPresence,
    is_self: bool,
) -> BrowserSessionParticipant {
    if sensors.iter().any(|sensor| sensor.kind == "audio") {
        media_presence.mic_available = true;
    }
    BrowserSessionParticipant {
        peer_id,
        app_id: metadata.app_id,
        display_name: metadata.display_name,
        is_self,
        connected: true,
        sensors,
        media_presence,
    }
}

#[cfg(feature = "browser_libp2p")]
#[wasm_bindgen]
impl BrowserDomainSession {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: &[u8]) -> Result<Self, JsValue> {
        let seed = seed_array(seed).map_err(|err| JsValue::from_str(&err))?;
        let inner = auki_domain::browser_session::BrowserDomainSession::new(
            peer_identity_from_seed_bytes(&seed),
        );
        let state = Rc::new(BrowserDomainSessionState::new(inner.peer_id()));
        let full_peer =
            browser_full_peer::BrowserFullPeer::new(inner.identity(), state.local_participant());
        Ok(Self {
            inner,
            state,
            full_peer,
        })
    }

    #[wasm_bindgen(js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.state.peer_id.clone()
    }

    #[wasm_bindgen(js_name = debugState)]
    pub fn debug_state(&self) -> Result<JsValue, JsValue> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            uses_browser_session: bool,
            advertised_multiaddrs: Vec<String>,
            membership_peer_count: usize,
        }
        let state = self.full_peer.debug_state();
        serde_wasm_bindgen::to_value(&Wire {
            uses_browser_session: state.uses_browser_session,
            advertised_multiaddrs: state.advertised_multiaddrs,
            membership_peer_count: state.membership_peer_count,
        })
        .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    #[wasm_bindgen(js_name = enableGeneratedAudioForTests)]
    pub fn enable_generated_audio_for_tests(&self) {
        self.full_peer
            .set_audio_publication(browser_full_peer::BrowserAudioPublication::Generated);
    }

    #[wasm_bindgen(js_name = createDomain)]
    pub fn create_domain(
        &self,
        _discovery_url: String,
        _domain_name: String,
    ) -> Result<JsValue, JsValue> {
        browser_domain_result(self.inner.transport_unavailable())
    }

    #[wasm_bindgen(js_name = joinDomain)]
    pub async fn join_domain(
        &self,
        discovery_url: String,
        domain_name: String,
    ) -> Result<JsValue, JsValue> {
        let result = join_browser_domain(
            discovery_url.clone(),
            domain_name.clone(),
            self.full_peer.clone(),
        )
        .await;
        if result.ok {
            if let Some(value) = &result.value {
                self.state.apply_snapshot(
                    self.full_peer
                        .roster_snapshot(value.domain_name.clone(), value.manager_peer_id.clone()),
                );
            }
        }
        serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
    }

    #[wasm_bindgen(js_name = leaveDomain)]
    pub fn leave_domain(&self) -> Result<JsValue, JsValue> {
        let _ = self.state.queue(BrowserSessionClientMessage::Leave);
        self.state.clear_sender();
        self.state.clear_snapshot();
        browser_domain_result(self.inner.ok())
    }

    #[wasm_bindgen(js_name = observeParticipants)]
    pub fn observe_participants(&self, callback: js_sys::Function) -> js_sys::Function {
        use wasm_bindgen::JsCast as _;

        let id = self.state.next_observer_id.get();
        self.state.next_observer_id.set(id.saturating_add(1));
        self.state
            .observers
            .borrow_mut()
            .insert(id, callback.clone());
        let _ = callback.call1(&JsValue::NULL, &self.state.snapshot_js_value());

        let state = Rc::downgrade(&self.state);
        let unsubscribe = Closure::<dyn FnMut()>::new(move || {
            if let Some(state) = state.upgrade() {
                state.observers.borrow_mut().remove(&id);
            }
        });
        unsubscribe.into_js_value().unchecked_into()
    }

    #[wasm_bindgen(js_name = setParticipantMetadata)]
    pub fn set_participant_metadata(&self, metadata: JsValue) -> Result<JsValue, JsValue> {
        let metadata: BrowserMetadata = serde_wasm_bindgen::from_value(metadata)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        self.state.metadata.replace(metadata);
        let participant = self.state.local_participant();
        self.full_peer.update_local_participant(participant.clone());
        let _ = self
            .state
            .queue(BrowserSessionClientMessage::UpdateParticipant { participant });
        self.state.update_local_snapshot();
        browser_domain_result(self.inner.ok())
    }

    #[wasm_bindgen(js_name = declareLocalSensors)]
    pub fn declare_local_sensors(&self, sensors: JsValue) -> Result<JsValue, JsValue> {
        let sensors: Vec<BrowserSessionSensor> = serde_wasm_bindgen::from_value(sensors)
            .map_err(|err| JsValue::from_str(&err.to_string()))?;
        self.state.sensors.replace(sensors);
        let participant = self.state.local_participant();
        self.full_peer.update_local_participant(participant.clone());
        let _ = self
            .state
            .queue(BrowserSessionClientMessage::UpdateParticipant { participant });
        self.state.update_local_snapshot();
        browser_domain_result(self.inner.ok())
    }

    #[wasm_bindgen(js_name = setSensorPublication)]
    pub fn set_sensor_publication(
        &self,
        sensor_id: String,
        enabled: bool,
    ) -> Result<JsValue, JsValue> {
        if sensor_id == "audio" {
            if !enabled {
                self.full_peer
                    .set_audio_publication(browser_full_peer::BrowserAudioPublication::Off);
            } else if self.full_peer.audio_publication()
                == browser_full_peer::BrowserAudioPublication::Off
            {
                self.full_peer
                    .set_audio_publication(browser_full_peer::BrowserAudioPublication::Microphone);
            }
            let mut media = self.state.media_presence.borrow_mut();
            media.mic_available = true;
            media.mic_publication_enabled = enabled;
            media.mic_capture_healthy = enabled;
            self.full_peer.set_local_media(media.clone());
            drop(media);
            self.state.update_local_snapshot();
            return browser_domain_result(self.inner.ok());
        }
        if self.state.sender.borrow().is_none() {
            return browser_domain_result(self.inner.transport_unavailable());
        }
        let _ = self
            .state
            .queue(BrowserSessionClientMessage::SetSensorPublication { sensor_id, enabled });
        self.state.update_local_snapshot();
        browser_domain_result(self.inner.ok())
    }

    #[wasm_bindgen(js_name = subscribeToSensor)]
    pub fn subscribe_to_sensor(
        &self,
        peer_id: String,
        sensor_id: String,
    ) -> Result<JsValue, JsValue> {
        if sensor_id == "audio" {
            let mut media = self.state.media_presence.borrow_mut();
            media.listening_to_peer_id = Some(peer_id.clone());
            media.listening_to_sensor_id = Some(sensor_id.clone());
            media.selected_remote_stream_state = "connecting".to_string();
            media.playback_healthy = false;
            self.full_peer.set_local_media(media.clone());
            drop(media);
            self.state.update_local_snapshot();
            #[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
            {
                spawn_audio_subscription(
                    self.state.clone(),
                    self.full_peer.clone(),
                    peer_id,
                    sensor_id,
                );
                return browser_domain_result(self.inner.ok());
            }
            #[cfg(not(all(target_arch = "wasm32", feature = "browser_libp2p")))]
            {
                return browser_domain_result(self.inner.transport_unavailable());
            }
        }
        if self.state.sender.borrow().is_none() {
            return browser_domain_result(self.inner.transport_unavailable());
        }
        let _ = self
            .state
            .queue(BrowserSessionClientMessage::Subscribe { peer_id, sensor_id });
        self.state.update_local_snapshot();
        browser_domain_result(self.inner.ok())
    }

    #[wasm_bindgen(js_name = unsubscribeFromSensor)]
    pub fn unsubscribe_from_sensor(
        &self,
        peer_id: String,
        sensor_id: String,
    ) -> Result<JsValue, JsValue> {
        if self.state.sender.borrow().is_none() {
            return browser_domain_result(self.inner.transport_unavailable());
        }
        {
            let mut media = self.state.media_presence.borrow_mut();
            if media.listening_to_peer_id.as_deref() == Some(&peer_id)
                && media.listening_to_sensor_id.as_deref() == Some(&sensor_id)
            {
                media.listening_to_peer_id = None;
                media.listening_to_sensor_id = None;
                media.selected_remote_stream_state = "off".to_string();
                self.full_peer.set_local_media(media.clone());
            }
        }
        let _ = self
            .state
            .queue(BrowserSessionClientMessage::Unsubscribe { peer_id, sensor_id });
        self.state.update_local_snapshot();
        browser_domain_result(self.inner.ok())
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn spawn_audio_subscription(
    state: Rc<BrowserDomainSessionState>,
    full_peer: Rc<browser_full_peer::BrowserFullPeer>,
    peer_id: String,
    sensor_id: String,
) {
    wasm_bindgen_futures::spawn_local(async move {
        match subscribe_audio_stream(full_peer.clone(), peer_id.clone(), sensor_id.clone()).await {
            Ok(output_level) => {
                set_audio_media_state(
                    &state,
                    &full_peer,
                    Some(peer_id),
                    Some(sensor_id),
                    "connected",
                    true,
                    Some(js_sys::Date::now() as u64),
                    Some(output_level),
                );
            }
            Err(_err) => {
                set_audio_media_state(
                    &state,
                    &full_peer,
                    Some(peer_id),
                    Some(sensor_id),
                    "error",
                    false,
                    None,
                    None,
                );
            }
        }
    });
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn subscribe_audio_stream(
    full_peer: Rc<browser_full_peer::BrowserFullPeer>,
    peer_id: String,
    sensor_id: String,
) -> Result<u8, String> {
    use crate::browser_stream::{
        STREAM_PROTOCOL, StreamMessage, StreamRequest, read_message, stream_message, write_message,
    };
    use futures::{FutureExt as _, select};
    use libp2p::{PeerId, StreamProtocol};
    use prost::Message as _;

    let remote_peer: PeerId = peer_id
        .parse()
        .map_err(|err| format!("malformed peer id {peer_id}: {err}"))?;
    let mut control = full_peer
        .stream_control()
        .ok_or_else(|| "browser full peer stream control is unavailable".to_string())?;
    let proto = StreamProtocol::try_from_owned(STREAM_PROTOCOL.to_string())
        .expect("STREAM_PROTOCOL is a valid libp2p protocol id");

    let open = control.open_stream(remote_peer, proto).fuse();
    let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
    futures::pin_mut!(open, timeout);
    let mut substream = loop {
        select! {
            result = open => {
                break result.map_err(|err| format!("open audio stream failed: {err}"))?;
            }
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("audio stream open timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("audio stream timeout setup failed: {err}")),
                };
            }
        }
    };

    write_message(
        &mut substream,
        &StreamMessage::request(StreamRequest {
            sensor_id: sensor_id.clone(),
        }),
    )
    .await
    .map_err(|err| format!("write audio stream request failed: {err}"))?;

    let reply = read_message(&mut substream)
        .await
        .map_err(|err| format!("read audio stream reply failed: {err}"))?;
    match reply.variant {
        Some(stream_message::Variant::Accept(_manifest)) => {}
        Some(stream_message::Variant::Decline(reason)) => {
            return Err(format!("audio stream declined: {reason:?}"));
        }
        _ => return Err("audio stream expected Accept or Decline".to_string()),
    }

    loop {
        let message = read_message(&mut substream)
            .await
            .map_err(|err| format!("read audio stream entry failed: {err}"))?;
        if let Some(stream_message::Variant::Entry(entry)) = message.variant {
            let data = crate::browser_stream::audio::Data::decode(&*entry.payload)
                .map_err(|err| format!("decode audio payload failed: {err}"))?;
            return Ok(audio_output_level(&data.data));
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn set_audio_media_state(
    state: &Rc<BrowserDomainSessionState>,
    full_peer: &Rc<browser_full_peer::BrowserFullPeer>,
    peer_id: Option<String>,
    sensor_id: Option<String>,
    stream_state: &str,
    playback_healthy: bool,
    last_frame_unix_ms: Option<u64>,
    output_level: Option<u8>,
) {
    let mut media = state.media_presence.borrow_mut();
    media.listening_to_peer_id = peer_id;
    media.listening_to_sensor_id = sensor_id;
    media.selected_remote_stream_state = stream_state.to_string();
    media.playback_healthy = playback_healthy;
    media.last_frame_unix_ms = last_frame_unix_ms;
    media.output_level = output_level;
    full_peer.set_local_media(media.clone());
    drop(media);
    state.update_local_snapshot();
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn audio_output_level(pcm_s16le: &[u8]) -> u8 {
    let max_abs = pcm_s16le
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]).unsigned_abs() as u32)
        .max()
        .unwrap_or(0);
    ((max_abs * 100) / 32768).min(100) as u8
}

#[cfg(feature = "browser_libp2p")]
fn browser_domain_result(
    result: auki_domain::browser_session::BrowserDomainResult,
) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
}

#[cfg(feature = "browser_libp2p")]
#[derive(serde::Serialize)]
struct BrowserDomainJoinResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<BrowserDomainJoinValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BrowserDomainJoinError>,
}

#[cfg(feature = "browser_libp2p")]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserDomainJoinValue {
    domain_name: String,
    manager_peer_id: String,
    membership_json: String,
}

#[cfg(feature = "browser_libp2p")]
#[derive(serde::Serialize)]
struct BrowserDomainJoinError {
    code: &'static str,
    message: String,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[derive(serde::Deserialize)]
struct BrowserDiscoveryList {
    clusters: Vec<BrowserDiscoveryCluster>,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[derive(serde::Deserialize)]
struct BrowserDiscoveryCluster {
    name: String,
    manager_peer_id: String,
    manager_multiaddrs: Vec<String>,
    #[serde(default)]
    relay_multiaddrs: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn join_browser_domain(
    discovery_url: String,
    domain_name: String,
    full_peer: Rc<browser_full_peer::BrowserFullPeer>,
) -> BrowserDomainJoinResult {
    match join_browser_domain_inner(discovery_url, domain_name, full_peer).await {
        Ok(value) => BrowserDomainJoinResult {
            ok: true,
            value: Some(value),
            error: None,
        },
        Err((code, message)) => BrowserDomainJoinResult {
            ok: false,
            value: None,
            error: Some(BrowserDomainJoinError { code, message }),
        },
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "browser_libp2p"))]
async fn join_browser_domain(
    _discovery_url: String,
    domain_name: String,
    _full_peer: Rc<browser_full_peer::BrowserFullPeer>,
) -> BrowserDomainJoinResult {
    BrowserDomainJoinResult {
        ok: false,
        value: None,
        error: Some(BrowserDomainJoinError {
            code: "domain_join_failed",
            message: format!(
                "Browser Domain join for {domain_name:?} requires wasm32 browser transport."
            ),
        }),
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "browser_libp2p"))]
async fn start_browser_session(
    _identity: PeerIdentity,
    _discovery_url: String,
    _domain_name: String,
    _state: Rc<BrowserDomainSessionState>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn start_browser_session(
    identity: PeerIdentity,
    discovery_url: String,
    domain_name: String,
    state: Rc<BrowserDomainSessionState>,
) -> Result<(), String> {
    use auki_network::browser_session_protocol::{
        BROWSER_SESSION_PROTOCOL, BrowserSessionServerMessage, read_server_message,
        write_client_message,
    };
    use futures::{AsyncReadExt as _, FutureExt as _, StreamExt as _, select};
    use libp2p::{
        PeerId, StreamProtocol, SwarmBuilder,
        swarm::{SwarmEvent, dial_opts::DialOpts},
    };

    let entry = fetch_browser_discovery_cluster(discovery_url, &domain_name)
        .await
        .map_err(|(code, message)| format!("{code}: {message}"))?;
    let address =
        browser_manager_address(&entry).map_err(|(code, message)| format!("{code}: {message}"))?;
    let manager_peer_id = entry.manager_peer_id.clone();
    let remote_peer: PeerId = manager_peer_id
        .parse()
        .map_err(|err| format!("malformed manager peer id: {err}"))?;
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_wasm_bindgen()
        .with_other_transport(|keypair| {
            libp2p::webrtc_websys::Transport::new(libp2p::webrtc_websys::Config::new(keypair))
                .boxed()
        })
        .map_err(|err| format!("transport setup failed: {err}"))?
        .with_behaviour(|_| BrowserJoinBehaviour {
            stream: libp2p_stream::Behaviour::new(),
        })
        .map_err(|err| format!("behaviour setup failed: {err}"))?
        .build();

    swarm
        .dial(
            DialOpts::peer_id(remote_peer)
                .addresses(vec![address])
                .build(),
        )
        .map_err(|err| format!("browser session dial setup failed: {err}"))?;

    {
        let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
        futures::pin_mut!(timeout);
        loop {
            select! {
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("browser session dial timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("browser session dial timeout setup failed: {err}")),
                    };
                }
                event = swarm.select_next_some().fuse() => {
                    match event {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == remote_peer => break,
                        SwarmEvent::OutgoingConnectionError {
                            peer_id: Some(peer),
                            error,
                            ..
                        } if peer == remote_peer => {
                            return Err(format!("browser session dial failure for {peer}: {error}"));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let mut control = swarm.behaviour().stream.new_control();
    let proto = StreamProtocol::try_from_owned(BROWSER_SESSION_PROTOCOL.to_string())
        .expect("BROWSER_SESSION_PROTOCOL is a valid libp2p protocol id");
    let open = control.open_stream(remote_peer, proto).fuse();
    let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
    futures::pin_mut!(open, timeout);

    let mut substream = loop {
        select! {
            result = open => {
                break result.map_err(|err| format!("open browser session stream failed: {err}"))?;
            }
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("browser session stream open timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("browser session stream open timeout setup failed: {err}")),
                };
            }
            event = swarm.select_next_some().fuse() => {
                if let SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(peer),
                    error,
                    ..
                } = event
                {
                    if peer == remote_peer {
                        return Err(format!("browser session dial failure for {peer}: {error}"));
                    }
                }
            }
        }
    };

    write_client_message(
        &mut substream,
        &BrowserSessionClientMessage::Hello {
            domain_name,
            participant: state.local_participant(),
        },
    )
    .await
    .map_err(|err| format!("write browser session hello failed: {err}"))?;

    let (mut reader, mut writer) = substream.split();
    let (sender, mut outbound) = futures::channel::mpsc::unbounded();
    state.set_sender(sender);
    let task_state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            select! {
                outbound_message = outbound.next().fuse() => {
                    let Some(message) = outbound_message else {
                        break;
                    };
                    let should_leave = matches!(message, BrowserSessionClientMessage::Leave);
                    if write_client_message(&mut writer, &message).await.is_err() {
                        break;
                    }
                    if should_leave {
                        break;
                    }
                }
                server_message = read_server_message(&mut reader).fuse() => {
                    match server_message {
                        Ok(BrowserSessionServerMessage::Snapshot { snapshot }) => {
                            task_state.apply_snapshot(snapshot);
                        }
                        Ok(BrowserSessionServerMessage::Ack) => {}
                        Ok(BrowserSessionServerMessage::Error { .. }) => {}
                        Err(_) => break,
                    }
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
        task_state.clear_sender();
    });

    Ok(())
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn join_browser_domain_inner(
    discovery_url: String,
    domain_name: String,
    full_peer: Rc<browser_full_peer::BrowserFullPeer>,
) -> Result<BrowserDomainJoinValue, (&'static str, String)> {
    let entry = fetch_browser_discovery_cluster(discovery_url, &domain_name).await?;
    let manager_address = browser_manager_address(&entry)?;
    let relay_address = browser_relay_address(&entry, &manager_address)?;
    let response = full_peer
        .clone()
        .join_via_relayed_peer(manager_address, relay_address)
        .await
        .map_err(|err| ("domain_join_failed", err))?;

    match response {
        auki_network::join_protocol::JoinResponse::Accept {
            membership_json, ..
        } => Ok(BrowserDomainJoinValue {
            domain_name,
            manager_peer_id: entry.manager_peer_id,
            membership_json,
        }),
        auki_network::join_protocol::JoinResponse::Reject { reason } => Err((
            "domain_join_failed",
            format!("Manager rejected browser join: {reason}"),
        )),
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn fetch_browser_discovery_cluster(
    discovery_url: String,
    domain_name: &str,
) -> Result<BrowserDiscoveryCluster, (&'static str, String)> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    if let Some(encoded) = discovery_url.strip_prefix("inline-manager://") {
        let inline_addr = js_sys::decode_uri_component(encoded)
            .map_err(|err| ("domain_join_failed", js_error_message(err.into())))?
            .as_string()
            .ok_or_else(|| {
                (
                    "domain_join_failed",
                    "inline Manager address did not decode to a string.".to_string(),
                )
            })?;
        let (manager_addr, relay_addr) = inline_addr
            .split_once('|')
            .map(|(manager, relay)| (manager.to_string(), Some(relay.to_string())))
            .unwrap_or((inline_addr, None));
        let manager_addr: libp2p::Multiaddr = manager_addr.parse().map_err(|err| {
            (
                "domain_join_failed",
                format!("inline Manager multiaddr is malformed: {err}"),
            )
        })?;
        let relay_addr = relay_addr
            .map(|addr| {
                addr.parse::<libp2p::Multiaddr>().map_err(|err| {
                    (
                        "domain_join_failed",
                        format!("inline relay multiaddr is malformed: {err}"),
                    )
                })
            })
            .transpose()?;
        let manager_peer_id = peer_id_from_multiaddr(&manager_addr)
            .map_err(|err| ("domain_join_failed", err))?
            .to_string();
        return Ok(BrowserDiscoveryCluster {
            name: domain_name.to_string(),
            manager_peer_id,
            manager_multiaddrs: vec![manager_addr.to_string()],
            relay_multiaddrs: relay_addr
                .map(|addr| vec![addr.to_string()])
                .unwrap_or_default(),
        });
    }

    let window = web_sys::window().ok_or_else(|| {
        (
            "discovery_unreachable",
            "Browser window is unavailable for Discovery fetch.".to_string(),
        )
    })?;
    let base = discovery_url.trim_end_matches('/');
    let response = JsFuture::from(window.fetch_with_str(&format!("{base}/clusters")))
        .await
        .map_err(|err| ("discovery_unreachable", js_error_message(err)))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| {
        (
            "domain_join_failed",
            "Discovery fetch did not return a Response object.".to_string(),
        )
    })?;

    if !response.ok() {
        return Err((
            "domain_join_failed",
            format!(
                "Discovery returned HTTP {} while joining {domain_name}.",
                response.status()
            ),
        ));
    }

    let body = JsFuture::from(response.json().map_err(|err| {
        (
            "domain_join_failed",
            format!(
                "Discovery response JSON setup failed: {}",
                js_error_message(err)
            ),
        )
    })?)
    .await
    .map_err(|err| {
        (
            "domain_join_failed",
            format!("Discovery response JSON failed: {}", js_error_message(err)),
        )
    })?;
    let list: BrowserDiscoveryList = serde_wasm_bindgen::from_value(body).map_err(|err| {
        (
            "domain_join_failed",
            format!("Discovery returned malformed cluster JSON: {err}"),
        )
    })?;

    list.clusters
        .into_iter()
        .find(|cluster| cluster.name == domain_name)
        .ok_or_else(|| {
            (
                "domain_join_failed",
                format!("Domain {domain_name:?} was not found in Discovery."),
            )
        })
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn browser_manager_address(
    entry: &BrowserDiscoveryCluster,
) -> Result<libp2p::Multiaddr, (&'static str, String)> {
    let address = entry
        .manager_multiaddrs
        .iter()
        .find(|addr| addr.contains("/webrtc-direct/"))
        .or_else(|| entry.manager_multiaddrs.first())
        .ok_or_else(|| {
            (
                "domain_join_failed",
                format!(
                    "Discovery entry {:?} has no Manager multiaddrs.",
                    entry.name
                ),
            )
        })?;
    let address = if address.contains("/p2p/") {
        address.clone()
    } else {
        format!("{address}/p2p/{}", entry.manager_peer_id)
    };
    address.parse().map_err(|err| {
        (
            "domain_join_failed",
            format!("Discovery Manager multiaddr is malformed: {err}"),
        )
    })
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn browser_relay_address(
    entry: &BrowserDiscoveryCluster,
    manager_address: &libp2p::Multiaddr,
) -> Result<libp2p::Multiaddr, (&'static str, String)> {
    let Some(address) = entry
        .relay_multiaddrs
        .iter()
        .find(|addr| addr.contains("/webrtc-direct/"))
        .or_else(|| entry.relay_multiaddrs.first())
    else {
        return Ok(manager_address.clone());
    };
    address.parse().map_err(|err| {
        (
            "domain_join_failed",
            format!("Discovery relay multiaddr is malformed: {err}"),
        )
    })
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
fn js_error_message(value: JsValue) -> String {
    if let Some(message) = value.as_string() {
        return message;
    }
    js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| "unknown JavaScript error".to_string())
}

#[cfg_attr(feature = "browser_libp2p", derive(serde::Serialize))]
pub struct BrowserProbeResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub protocol: String,
    pub payload: Vec<u8>,
    pub error: Option<String>,
}

#[cfg(feature = "browser_libp2p")]
#[derive(serde::Serialize)]
pub struct BrowserJoinResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub manager_peer_id: String,
    pub protocol: String,
    pub membership_json: Option<String>,
    pub successor_token: Vec<u8>,
    pub error: Option<String>,
}

#[cfg(feature = "browser_libp2p")]
impl BrowserJoinResult {
    pub fn accept(
        local_peer_id: impl Into<String>,
        manager_peer_id: impl Into<String>,
        membership_json: String,
        successor_token: Vec<u8>,
    ) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: manager_peer_id.into(),
            protocol: auki_network::join_protocol::JOIN_PROTOCOL.to_string(),
            membership_json: Some(membership_json),
            successor_token,
            error: None,
        }
    }

    pub fn reject(
        local_peer_id: impl Into<String>,
        manager_peer_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: manager_peer_id.into(),
            protocol: auki_network::join_protocol::JOIN_PROTOCOL.to_string(),
            membership_json: None,
            successor_token: Vec::new(),
            error: Some(reason.into()),
        }
    }
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
#[wasm_bindgen(js_name = dialBrowserJoin)]
pub async fn dial_browser_join(
    seed: &[u8],
    address: String,
    advertised_multiaddrs: js_sys::Array,
) -> Result<JsValue, JsValue> {
    let seed = seed_array(seed).map_err(|err| JsValue::from_str(&err))?;
    let identity = peer_identity_from_seed_bytes(&seed);
    let local_peer_id = identity.peer_id().to_string();
    let address: libp2p::Multiaddr = address
        .parse()
        .map_err(|err| JsValue::from_str(&format!("malformed manager multiaddr: {err}")))?;
    let manager_peer_id = peer_id_from_multiaddr(&address)
        .map_err(|err| JsValue::from_str(&err))?
        .to_string();
    let advertised_multiaddrs =
        multiaddrs_from_js_array(advertised_multiaddrs).map_err(|err| JsValue::from_str(&err))?;

    let outcome = dial_browser_join_inner(
        identity,
        address,
        advertised_multiaddrs,
        manager_peer_id.clone(),
    )
    .await;
    let result = match outcome {
        Ok(auki_network::join_protocol::JoinResponse::Accept {
            membership_json,
            successor_token,
        }) => BrowserJoinResult::accept(
            local_peer_id,
            manager_peer_id,
            membership_json,
            successor_token,
        ),
        Ok(auki_network::join_protocol::JoinResponse::Reject { reason }) => {
            BrowserJoinResult::reject(local_peer_id, manager_peer_id, reason)
        }
        Err(err) => BrowserJoinResult::reject(local_peer_id, manager_peer_id, err),
    };

    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
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
struct BrowserJoinBehaviour {
    stream: libp2p_stream::Behaviour,
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
pub(crate) async fn dial_browser_join_inner(
    identity: PeerIdentity,
    address: libp2p::Multiaddr,
    advertised_multiaddrs: Vec<libp2p::Multiaddr>,
    manager_peer_id: String,
) -> Result<auki_network::join_protocol::JoinResponse, String> {
    use auki_network::join_protocol::{JOIN_PROTOCOL, read_join_response, write_join_request};
    use futures::{FutureExt as _, StreamExt as _, select};
    use libp2p::{
        PeerId, StreamProtocol, SwarmBuilder,
        swarm::{SwarmEvent, dial_opts::DialOpts},
    };

    let remote_peer: PeerId = manager_peer_id
        .parse()
        .map_err(|err| format!("malformed manager peer id: {err}"))?;
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_wasm_bindgen()
        .with_other_transport(|keypair| {
            libp2p::webrtc_websys::Transport::new(libp2p::webrtc_websys::Config::new(keypair))
                .boxed()
        })
        .map_err(|err| format!("transport setup failed: {err}"))?
        .with_behaviour(|_| BrowserJoinBehaviour {
            stream: libp2p_stream::Behaviour::new(),
        })
        .map_err(|err| format!("behaviour setup failed: {err}"))?
        .build();

    swarm
        .dial(
            DialOpts::peer_id(remote_peer)
                .addresses(vec![address])
                .build(),
        )
        .map_err(|err| format!("dial setup failed: {err}"))?;

    {
        let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
        futures::pin_mut!(timeout);
        loop {
            select! {
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("join dial timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("join dial timeout setup failed: {err}")),
                    };
                }
                event = swarm.select_next_some().fuse() => {
                    match event {
                        SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == remote_peer => break,
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
        }
    }

    let mut control = swarm.behaviour().stream.new_control();
    let proto = StreamProtocol::try_from_owned(JOIN_PROTOCOL.to_string())
        .expect("JOIN_PROTOCOL is a valid libp2p protocol id");
    let open = control.open_stream(remote_peer, proto).fuse();
    let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
    futures::pin_mut!(open, timeout);

    let mut substream = loop {
        select! {
            result = open => {
                break result.map_err(|err| format!("open join stream failed: {err}"))?;
            }
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("join stream open timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("join stream open timeout setup failed: {err}")),
                };
            }
            event = swarm.select_next_some().fuse() => {
                if let SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(peer),
                    error,
                    ..
                } = event
                {
                    if peer == remote_peer {
                        return Err(format!("dial failure for {peer}: {error}"));
                    }
                }
            }
        }
    };

    let request = auki_network::join_protocol::JoinRequest {
        multiaddrs: advertised_multiaddrs,
    };
    {
        let write = write_join_request(&mut substream, &request).fuse();
        let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
        futures::pin_mut!(write, timeout);
        loop {
            select! {
                result = write => {
                    result.map_err(|err| format!("write join request failed: {err}"))?;
                    break;
                }
                timeout_result = timeout => {
                    return match timeout_result {
                        Ok(()) => Err(format!("join request write timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                        Err(err) => Err(format!("join request write timeout setup failed: {err}")),
                    };
                }
                _event = swarm.select_next_some().fuse() => {}
            }
        }
    }

    let read = read_join_response(&mut substream).fuse();
    let timeout = js_timeout(BROWSER_JOIN_TIMEOUT_MS).fuse();
    futures::pin_mut!(read, timeout);
    loop {
        select! {
            result = read => {
                return result.map_err(|err| format!("read join response failed: {err}"));
            }
            timeout_result = timeout => {
                return match timeout_result {
                    Ok(()) => Err(format!("join response read timed out after {BROWSER_JOIN_TIMEOUT_MS}ms")),
                    Err(err) => Err(format!("join response read timeout setup failed: {err}")),
                };
            }
            _event = swarm.select_next_some().fuse() => {}
        }
    }
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
fn multiaddrs_from_js_array(values: js_sys::Array) -> Result<Vec<libp2p::Multiaddr>, String> {
    values
        .iter()
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| "advertised multiaddr must be a string".to_string())
                .and_then(|value| {
                    value
                        .parse()
                        .map_err(|err| format!("malformed advertised multiaddr: {err}"))
                })
        })
        .collect()
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
mod browser_session_state_tests {
    use super::*;

    #[test]
    fn local_browser_participant_defaults_match_park_contract() {
        let participant = browser_session_participant(
            "browser-a".to_string(),
            BrowserMetadata {
                app_id: "park".to_string(),
                display_name: "Park A".to_string(),
            },
            vec![
                auki_network::browser_session_protocol::BrowserSessionSensor {
                    id: "audio".to_string(),
                    kind: "audio".to_string(),
                    label: "Microphone".to_string(),
                    publishable: true,
                    subscribable: false,
                },
            ],
            auki_network::browser_session_protocol::BrowserMediaPresence::default(),
            true,
        );

        assert_eq!(participant.peer_id, "browser-a");
        assert_eq!(participant.app_id, "park");
        assert_eq!(participant.display_name, "Park A");
        assert!(participant.is_self);
        assert!(participant.media_presence.mic_available);
        assert_eq!(
            participant.media_presence.selected_remote_stream_state,
            "off"
        );
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
