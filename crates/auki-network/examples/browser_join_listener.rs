use auki_network::{
    PeerIdentity, browser_probe,
    browser_session_protocol::{
        BROWSER_SESSION_PROTOCOL, BrowserMediaPresence, BrowserRosterSnapshot,
        BrowserSessionClientMessage, BrowserSessionParticipant, BrowserSessionServerMessage,
        read_client_message, write_server_message,
    },
    join_protocol::{JOIN_PROTOCOL, JoinResponse, read_join_request, write_join_response},
};
use futures::{AsyncReadExt as _, StreamExt as _};
use libp2p::{Multiaddr, StreamProtocol, swarm::SwarmEvent};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};

#[derive(Debug, Clone)]
struct BrowserRosterState {
    domain_name: String,
    manager_peer_id: String,
    participants: BTreeMap<String, BrowserSessionParticipant>,
}

impl BrowserRosterState {
    fn new(domain_name: impl Into<String>, manager_peer_id: impl Into<String>) -> Self {
        Self {
            domain_name: domain_name.into(),
            manager_peer_id: manager_peer_id.into(),
            participants: BTreeMap::new(),
        }
    }

    fn upsert(&mut self, mut participant: BrowserSessionParticipant) {
        participant.connected = true;
        self.participants
            .insert(participant.peer_id.clone(), participant);
    }

    fn remove(&mut self, peer_id: &str) {
        self.participants.remove(peer_id);
    }

    fn set_publication(&mut self, peer_id: &str, sensor_id: &str, enabled: bool) {
        let Some(participant) = self.participants.get_mut(peer_id) else {
            return;
        };
        if sensor_id == "audio" {
            participant.media_presence.mic_available = true;
            participant.media_presence.mic_publication_enabled = enabled;
            participant.media_presence.mic_capture_healthy = enabled;
        }
    }

    fn subscribe(&mut self, peer_id: &str, target_peer_id: String, sensor_id: String) {
        let Some(participant) = self.participants.get_mut(peer_id) else {
            return;
        };
        participant.media_presence.listening_to_peer_id = Some(target_peer_id);
        participant.media_presence.listening_to_sensor_id = Some(sensor_id);
        participant.media_presence.selected_remote_stream_state = "connecting".to_string();
    }

    fn unsubscribe(&mut self, peer_id: &str, target_peer_id: &str, sensor_id: &str) {
        let Some(participant) = self.participants.get_mut(peer_id) else {
            return;
        };
        if participant.media_presence.listening_to_peer_id.as_deref() == Some(target_peer_id)
            && participant.media_presence.listening_to_sensor_id.as_deref() == Some(sensor_id)
        {
            participant.media_presence.listening_to_peer_id = None;
            participant.media_presence.listening_to_sensor_id = None;
            participant.media_presence.selected_remote_stream_state = "off".to_string();
        }
    }

    fn snapshot_for(&self, self_peer_id: &str) -> BrowserRosterSnapshot {
        let mut participants = vec![manager_participant(&self.manager_peer_id, self_peer_id)];
        participants.extend(self.participants.values().cloned().map(|mut participant| {
            participant.is_self = participant.peer_id == self_peer_id;
            participant
        }));
        BrowserRosterSnapshot {
            self_peer_id: self_peer_id.to_string(),
            domain_name: self.domain_name.clone(),
            manager_peer_id: self.manager_peer_id.clone(),
            election_state: "stable".to_string(),
            participants,
        }
    }

    fn membership_json_with(&self, joining_peer_id: &str) -> String {
        let mut peer_ids = vec![self.manager_peer_id.clone(), joining_peer_id.to_string()];
        peer_ids.extend(self.participants.keys().cloned());
        peer_ids.sort();
        peer_ids.dedup();
        let peers = peer_ids
            .into_iter()
            .map(|peer_id| {
                serde_json::json!({
                    "peer_id": peer_id,
                    "multiaddrs": [],
                    "join_ts_ns": 0,
                    "successor_token": [],
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "cluster_name": self.domain_name,
            "peers": peers,
        })
        .to_string()
    }
}

fn manager_participant(manager_peer_id: &str, self_peer_id: &str) -> BrowserSessionParticipant {
    BrowserSessionParticipant {
        peer_id: manager_peer_id.to_string(),
        app_id: "auki-network".to_string(),
        display_name: "Native Manager".to_string(),
        is_self: manager_peer_id == self_peer_id,
        connected: true,
        sensors: Vec::new(),
        media_presence: BrowserMediaPresence::default(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = [41u8; 32];
    let identity = PeerIdentity::from_seed(&seed);
    let domain_name = "browser-two-peer-smoke";
    let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/0/webrtc-direct".parse()?;
    let mut swarm = browser_probe::build_browser_probe_swarm(&identity)?;
    let roster = Arc::new(Mutex::new(BrowserRosterState::new(
        domain_name,
        identity.peer_id().to_string(),
    )));
    let (snapshot_tx, _) = broadcast::channel::<()>(64);
    let mut incoming = swarm
        .behaviour()
        .stream
        .new_control()
        .accept(StreamProtocol::new(JOIN_PROTOCOL))?;
    let mut browser_sessions = swarm
        .behaviour()
        .stream
        .new_control()
        .accept(StreamProtocol::new(BROWSER_SESSION_PROTOCOL))?;

    eprintln!("peer_id={}", identity.peer_id());
    swarm.listen_on(listen_addr)?;

    loop {
        tokio::select! {
            Some((peer, mut stream)) = incoming.next() => {
                let roster = roster.clone();
                tokio::spawn(async move {
                    let Ok(_request) = read_join_request(&mut stream).await else {
                        eprintln!("join request read failed from {peer}");
                        return;
                    };
                    let membership_json = roster.lock().await.membership_json_with(&peer.to_string());
                    let response = JoinResponse::Accept {
                        membership_json,
                        successor_token: Vec::new(),
                    };
                    if let Err(err) = write_join_response(&mut stream, &response).await {
                        eprintln!("join response write failed to {peer}: {err}");
                    }
                });
            }
            Some((peer, stream)) = browser_sessions.next() => {
                let roster = roster.clone();
                let snapshot_tx = snapshot_tx.clone();
                tokio::spawn(async move {
                    handle_browser_session(peer.to_string(), stream, roster, snapshot_tx).await;
                });
            }
            Some(event) = swarm.next() => {
                if let SwarmEvent::NewListenAddr { address, .. } = event {
                    println!("PARK_BROWSER_JOIN_ADDR={address}/p2p/{}", identity.peer_id());
                }
            }
        }
    }
}

async fn handle_browser_session(
    peer_id: String,
    stream: libp2p::Stream,
    roster: Arc<Mutex<BrowserRosterState>>,
    snapshot_tx: broadcast::Sender<()>,
) {
    let (mut reader, mut writer) = stream.split();
    let hello = match read_client_message(&mut reader).await {
        Ok(BrowserSessionClientMessage::Hello {
            domain_name: _,
            mut participant,
        }) => {
            participant.peer_id = peer_id.clone();
            participant.is_self = true;
            participant.connected = true;
            participant
        }
        Ok(other) => {
            eprintln!("browser session from {peer_id} started with {other:?}");
            return;
        }
        Err(err) => {
            eprintln!("browser session hello read failed from {peer_id}: {err}");
            return;
        }
    };

    {
        let mut state = roster.lock().await;
        state.upsert(hello);
    }
    let _ = snapshot_tx.send(());

    let mut snapshot_rx = snapshot_tx.subscribe();
    {
        let snapshot = roster.lock().await.snapshot_for(&peer_id);
        if let Err(err) = write_server_message(
            &mut writer,
            &BrowserSessionServerMessage::Snapshot { snapshot },
        )
        .await
        {
            eprintln!("initial browser snapshot write failed to {peer_id}: {err}");
            return;
        }
    }

    loop {
        tokio::select! {
            message = read_client_message(&mut reader) => {
                match message {
                    Ok(BrowserSessionClientMessage::UpdateParticipant { mut participant }) => {
                        participant.peer_id = peer_id.clone();
                        participant.is_self = true;
                        participant.connected = true;
                        roster.lock().await.upsert(participant);
                        let _ = snapshot_tx.send(());
                    }
                    Ok(BrowserSessionClientMessage::SetSensorPublication { sensor_id, enabled }) => {
                        roster.lock().await.set_publication(&peer_id, &sensor_id, enabled);
                        let _ = snapshot_tx.send(());
                    }
                    Ok(BrowserSessionClientMessage::Subscribe { peer_id: target_peer_id, sensor_id }) => {
                        roster.lock().await.subscribe(&peer_id, target_peer_id, sensor_id);
                        let _ = snapshot_tx.send(());
                    }
                    Ok(BrowserSessionClientMessage::Unsubscribe { peer_id: target_peer_id, sensor_id }) => {
                        roster.lock().await.unsubscribe(&peer_id, &target_peer_id, &sensor_id);
                        let _ = snapshot_tx.send(());
                    }
                    Ok(BrowserSessionClientMessage::Leave) => break,
                    Ok(BrowserSessionClientMessage::Hello { .. }) => {}
                    Err(err) => {
                        eprintln!("browser session read ended for {peer_id}: {err}");
                        break;
                    }
                }
            }
            signal = snapshot_rx.recv() => {
                if signal.is_err() {
                    break;
                }
                let snapshot = roster.lock().await.snapshot_for(&peer_id);
                if let Err(err) = write_server_message(
                    &mut writer,
                    &BrowserSessionServerMessage::Snapshot { snapshot },
                ).await {
                    eprintln!("browser snapshot write failed to {peer_id}: {err}");
                    break;
                }
            }
        }
    }

    roster.lock().await.remove(&peer_id);
    let _ = snapshot_tx.send(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_network::browser_session_protocol::{
        BrowserMediaPresence, BrowserSessionParticipant, BrowserSessionSensor,
    };

    fn sample_participant(peer_id: &str) -> BrowserSessionParticipant {
        BrowserSessionParticipant {
            peer_id: peer_id.to_string(),
            app_id: "park".to_string(),
            display_name: peer_id.to_string(),
            is_self: true,
            connected: true,
            sensors: vec![BrowserSessionSensor {
                id: "audio".to_string(),
                kind: "audio".to_string(),
                label: "Microphone".to_string(),
                publishable: true,
                subscribable: false,
            }],
            media_presence: BrowserMediaPresence {
                mic_available: true,
                mic_publication_enabled: false,
                mic_capture_healthy: true,
                listening_to_peer_id: None,
                listening_to_sensor_id: None,
                playback_healthy: false,
                selected_remote_stream_state: "off".to_string(),
                last_frame_unix_ms: None,
                input_level: None,
                output_level: None,
            },
        }
    }

    #[test]
    fn browser_roster_state_pushes_symmetric_snapshots() {
        let manager = "manager-peer".to_string();
        let mut roster = BrowserRosterState::new("browser-two-peer-smoke", manager.clone());
        roster.upsert(sample_participant("browser-a"));
        roster.upsert(sample_participant("browser-b"));

        let snapshot_a = roster.snapshot_for("browser-a");
        let snapshot_b = roster.snapshot_for("browser-b");

        assert!(
            snapshot_a
                .participants
                .iter()
                .any(|participant| participant.peer_id == "browser-b" && !participant.is_self)
        );
        assert!(
            snapshot_b
                .participants
                .iter()
                .any(|participant| participant.peer_id == "browser-a" && !participant.is_self)
        );
        assert_eq!(snapshot_a.manager_peer_id, manager);
        assert_eq!(snapshot_b.manager_peer_id, manager);
    }
}
