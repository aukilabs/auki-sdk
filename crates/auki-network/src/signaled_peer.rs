use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignaledPeerRole {
    Initiator,
    Responder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignaledPeerCloseReason {
    DuplicateConnection,
    RemoteClosed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignaledPeerCommand {
    CreatePeerConnection {
        connection_id: String,
        remote_peer_id: String,
        role: SignaledPeerRole,
    },
    SetRemoteDescription {
        connection_id: String,
        sdp_json: String,
    },
    AddIceCandidate {
        connection_id: String,
        candidate_json: String,
    },
    CloseConnection {
        connection_id: String,
        remote_peer_id: String,
        reason: SignaledPeerCloseReason,
    },
    SendDataChannelMessage {
        connection_id: String,
        protocol: String,
        payload: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignaledPeerEvent {
    StreamOpenRequest(SignaledStreamOpenRequest),
    StreamAccepted {
        stream_id: u64,
        manifest_json: String,
    },
    StreamEntry {
        stream_id: u64,
        entry_json: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignaledStreamOpenRequest {
    pub stream_id: u64,
    pub connection_id: String,
    pub remote_peer_id: String,
    pub protocol: String,
    pub request_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignaledStreamOpenResult {
    pub stream_id: u64,
    pub commands: Vec<SignaledPeerCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalEnvelope {
    pub from_peer_id: String,
    pub connection_id: String,
    pub kind: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignaledPeerError {
    MissingLocalPeerId,
    MissingDiscoveryUrl,
    MissingRemotePeerId,
    MissingConnectionId,
    MissingProtocol,
    DuplicateConnectionId(String),
    UnknownConnectionId(String),
    UnknownStreamId(u64),
    NoFramedHandler(String),
    NoStreamHandler(String),
    InvalidJson(String),
    InvalidStreamMessage(String),
    UnsupportedSignalKind(String),
}

impl fmt::Display for SignaledPeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignaledPeerError::MissingLocalPeerId => write!(f, "missing local peer id"),
            SignaledPeerError::MissingDiscoveryUrl => write!(f, "missing discovery url"),
            SignaledPeerError::MissingRemotePeerId => write!(f, "missing remote peer id"),
            SignaledPeerError::MissingConnectionId => write!(f, "missing connection id"),
            SignaledPeerError::MissingProtocol => write!(f, "missing protocol"),
            SignaledPeerError::DuplicateConnectionId(connection_id) => {
                write!(f, "duplicate connection id: {connection_id}")
            }
            SignaledPeerError::UnknownConnectionId(connection_id) => {
                write!(f, "unknown connection id: {connection_id}")
            }
            SignaledPeerError::UnknownStreamId(stream_id) => {
                write!(f, "unknown stream id: {stream_id}")
            }
            SignaledPeerError::NoFramedHandler(protocol) => {
                write!(f, "no framed handler registered for protocol: {protocol}")
            }
            SignaledPeerError::NoStreamHandler(protocol) => {
                write!(f, "no stream handler registered for protocol: {protocol}")
            }
            SignaledPeerError::InvalidJson(message) => write!(f, "invalid json: {message}"),
            SignaledPeerError::InvalidStreamMessage(message) => {
                write!(f, "invalid stream message: {message}")
            }
            SignaledPeerError::UnsupportedSignalKind(kind) => {
                write!(f, "unsupported signal kind: {kind}")
            }
        }
    }
}

impl Error for SignaledPeerError {}

pub struct SignaledPeerCore {
    local_peer_id: String,
    discovery_url: String,
    connections: HashMap<String, SignaledConnection>,
    active_by_remote: HashMap<String, String>,
    framed_handlers: HashMap<String, FramedHandler>,
    stream_protocols: HashSet<String>,
    pending_streams: HashMap<u64, PendingStream>,
    active_streams: HashMap<u64, ActiveStream>,
    next_stream_id: u64,
}

impl SignaledPeerCore {
    pub fn new(local_peer_id: String, discovery_url: String) -> Result<Self, SignaledPeerError> {
        if local_peer_id.is_empty() {
            return Err(SignaledPeerError::MissingLocalPeerId);
        }
        if discovery_url.is_empty() {
            return Err(SignaledPeerError::MissingDiscoveryUrl);
        }
        Ok(Self {
            local_peer_id,
            discovery_url,
            connections: HashMap::new(),
            active_by_remote: HashMap::new(),
            framed_handlers: HashMap::new(),
            stream_protocols: HashSet::new(),
            pending_streams: HashMap::new(),
            active_streams: HashMap::new(),
            next_stream_id: 1,
        })
    }

    pub fn local_peer_id(&self) -> &str {
        &self.local_peer_id
    }

    pub fn discovery_url(&self) -> &str {
        &self.discovery_url
    }

    pub fn request_framed(
        &self,
        connection_id: String,
        protocol: String,
        payload: Vec<u8>,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        validate_connection_and_protocol(&connection_id, &protocol)?;
        Ok(vec![SignaledPeerCommand::SendDataChannelMessage {
            connection_id,
            protocol,
            payload,
        }])
    }

    pub fn handle_framed<F>(
        &mut self,
        protocol: String,
        handler: F,
    ) -> Result<(), SignaledPeerError>
    where
        F: Fn(Vec<u8>) -> Option<Vec<u8>> + Send + Sync + 'static,
    {
        validate_protocol(&protocol)?;
        self.framed_handlers.insert(protocol, Arc::new(handler));
        Ok(())
    }

    pub fn receive_framed(
        &self,
        protocol: &str,
        payload: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, SignaledPeerError> {
        validate_protocol(protocol)?;
        let handler = self
            .framed_handlers
            .get(protocol)
            .ok_or_else(|| SignaledPeerError::NoFramedHandler(protocol.to_string()))?;
        Ok(handler(payload))
    }

    pub fn open_stream(
        &mut self,
        connection_id: String,
        remote_peer_id: String,
        protocol: String,
        request_json: String,
    ) -> Result<SignaledStreamOpenResult, SignaledPeerError> {
        validate_peer_and_connection(&remote_peer_id, &connection_id)?;
        validate_protocol(&protocol)?;
        let stream_id = self.next_stream_id();
        self.active_streams.insert(
            stream_id,
            ActiveStream {
                connection_id: connection_id.clone(),
                protocol: protocol.clone(),
            },
        );
        Ok(SignaledStreamOpenResult {
            stream_id,
            commands: vec![SignaledPeerCommand::SendDataChannelMessage {
                connection_id,
                protocol,
                payload: stream_envelope("request", &request_json)?,
            }],
        })
    }

    pub fn handle_stream(&mut self, protocol: String) -> Result<(), SignaledPeerError> {
        validate_protocol(&protocol)?;
        self.stream_protocols.insert(protocol);
        Ok(())
    }

    pub fn receive_stream_message(
        &mut self,
        connection_id: String,
        remote_peer_id: String,
        protocol: String,
        payload: Vec<u8>,
    ) -> Result<Vec<SignaledPeerEvent>, SignaledPeerError> {
        validate_peer_and_connection(&remote_peer_id, &connection_id)?;
        validate_protocol(&protocol)?;
        let message: serde_json::Value = serde_json::from_slice(&payload)
            .map_err(|err| SignaledPeerError::InvalidStreamMessage(err.to_string()))?;

        if let Some(request) = message.get("request") {
            if !self.stream_protocols.contains(&protocol) {
                return Err(SignaledPeerError::NoStreamHandler(protocol));
            }
            let stream_id = self.next_stream_id();
            let request_json = compact_json(request)?;
            self.pending_streams.insert(
                stream_id,
                PendingStream {
                    connection_id: connection_id.clone(),
                    protocol: protocol.clone(),
                },
            );
            return Ok(vec![SignaledPeerEvent::StreamOpenRequest(
                SignaledStreamOpenRequest {
                    stream_id,
                    connection_id,
                    remote_peer_id,
                    protocol,
                    request_json,
                },
            )]);
        }

        if let Some(accept) = message.get("accept") {
            let stream_id = self
                .active_stream_id(&connection_id, &protocol)
                .ok_or(SignaledPeerError::UnknownConnectionId(connection_id))?;
            return Ok(vec![SignaledPeerEvent::StreamAccepted {
                stream_id,
                manifest_json: compact_json(accept)?,
            }]);
        }

        if let Some(entry) = message.get("entry") {
            let stream_id = self
                .active_stream_id(&connection_id, &protocol)
                .ok_or(SignaledPeerError::UnknownConnectionId(connection_id))?;
            return Ok(vec![SignaledPeerEvent::StreamEntry {
                stream_id,
                entry_json: compact_json(entry)?,
            }]);
        }

        Err(SignaledPeerError::InvalidStreamMessage(
            "expected request, accept, or entry envelope".to_string(),
        ))
    }

    pub fn accept_stream_open(
        &mut self,
        stream_id: u64,
        manifest_json: String,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        let pending = self
            .pending_streams
            .remove(&stream_id)
            .ok_or(SignaledPeerError::UnknownStreamId(stream_id))?;
        self.active_streams.insert(
            stream_id,
            ActiveStream {
                connection_id: pending.connection_id.clone(),
                protocol: pending.protocol.clone(),
            },
        );
        Ok(vec![SignaledPeerCommand::SendDataChannelMessage {
            connection_id: pending.connection_id,
            protocol: pending.protocol,
            payload: stream_envelope("accept", &manifest_json)?,
        }])
    }

    pub fn push_stream_entry(
        &self,
        stream_id: u64,
        entry_json: String,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        let stream = self
            .active_streams
            .get(&stream_id)
            .ok_or(SignaledPeerError::UnknownStreamId(stream_id))?;
        Ok(vec![SignaledPeerCommand::SendDataChannelMessage {
            connection_id: stream.connection_id.clone(),
            protocol: stream.protocol.clone(),
            payload: stream_envelope("entry", &entry_json)?,
        }])
    }

    pub fn connect(
        &mut self,
        remote_peer_id: String,
        connection_id: String,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        validate_peer_and_connection(&remote_peer_id, &connection_id)?;
        if self.connections.contains_key(&connection_id) {
            return Err(SignaledPeerError::DuplicateConnectionId(connection_id));
        }
        if let Some(existing_id) = self.active_by_remote.get(&remote_peer_id) {
            return Err(SignaledPeerError::DuplicateConnectionId(
                existing_id.clone(),
            ));
        }

        self.connections.insert(
            connection_id.clone(),
            SignaledConnection {
                remote_peer_id: remote_peer_id.clone(),
                role: SignaledPeerRole::Initiator,
                remote_description_set: false,
                queued_candidates: Vec::new(),
            },
        );
        self.active_by_remote
            .insert(remote_peer_id.clone(), connection_id.clone());

        Ok(vec![SignaledPeerCommand::CreatePeerConnection {
            connection_id,
            remote_peer_id,
            role: SignaledPeerRole::Initiator,
        }])
    }

    pub fn handle_signal(
        &mut self,
        signal: SignalEnvelope,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        validate_peer_and_connection(&signal.from_peer_id, &signal.connection_id)?;
        match signal.kind.as_str() {
            "offer" => self.handle_offer(signal),
            "answer" => self.handle_answer(signal),
            "candidate" => self.handle_candidate(signal),
            "close" => {
                Ok(self
                    .close_connection(&signal.connection_id, SignaledPeerCloseReason::RemoteClosed))
            }
            other => Err(SignaledPeerError::UnsupportedSignalKind(other.to_string())),
        }
    }

    fn handle_offer(
        &mut self,
        signal: SignalEnvelope,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        let mut commands = Vec::new();
        if self.resolve_duplicate_offer(&signal, &mut commands) {
            return Ok(commands);
        }

        let created = self.ensure_responder_connection(&signal)?;
        if created {
            commands.push(SignaledPeerCommand::CreatePeerConnection {
                connection_id: signal.connection_id.clone(),
                remote_peer_id: signal.from_peer_id.clone(),
                role: SignaledPeerRole::Responder,
            });
        }

        let queued = {
            let connection = self
                .connections
                .get_mut(&signal.connection_id)
                .expect("connection inserted above");
            connection.remote_description_set = true;
            std::mem::take(&mut connection.queued_candidates)
        };

        commands.push(SignaledPeerCommand::SetRemoteDescription {
            connection_id: signal.connection_id.clone(),
            sdp_json: signal.payload_json,
        });
        commands.extend(queued.into_iter().map(|candidate_json| {
            SignaledPeerCommand::AddIceCandidate {
                connection_id: signal.connection_id.clone(),
                candidate_json,
            }
        }));
        Ok(commands)
    }

    fn handle_answer(
        &mut self,
        signal: SignalEnvelope,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        let connection = self
            .connections
            .get_mut(&signal.connection_id)
            .ok_or_else(|| SignaledPeerError::UnknownConnectionId(signal.connection_id.clone()))?;
        connection.remote_description_set = true;
        let queued = std::mem::take(&mut connection.queued_candidates);

        let mut commands = vec![SignaledPeerCommand::SetRemoteDescription {
            connection_id: signal.connection_id.clone(),
            sdp_json: signal.payload_json,
        }];
        commands.extend(queued.into_iter().map(|candidate_json| {
            SignaledPeerCommand::AddIceCandidate {
                connection_id: signal.connection_id.clone(),
                candidate_json,
            }
        }));
        Ok(commands)
    }

    fn handle_candidate(
        &mut self,
        signal: SignalEnvelope,
    ) -> Result<Vec<SignaledPeerCommand>, SignaledPeerError> {
        let mut commands = Vec::new();
        if self.resolve_duplicate_candidate(&signal, &mut commands) {
            return Ok(commands);
        }

        let created = self.ensure_responder_connection(&signal)?;
        if created {
            commands.push(SignaledPeerCommand::CreatePeerConnection {
                connection_id: signal.connection_id.clone(),
                remote_peer_id: signal.from_peer_id.clone(),
                role: SignaledPeerRole::Responder,
            });
        }

        let connection = self
            .connections
            .get_mut(&signal.connection_id)
            .expect("connection inserted above");
        if connection.remote_description_set {
            commands.push(SignaledPeerCommand::AddIceCandidate {
                connection_id: signal.connection_id,
                candidate_json: signal.payload_json,
            });
        } else {
            connection.queued_candidates.push(signal.payload_json);
        }
        Ok(commands)
    }

    fn ensure_responder_connection(
        &mut self,
        signal: &SignalEnvelope,
    ) -> Result<bool, SignaledPeerError> {
        if self.connections.contains_key(&signal.connection_id) {
            return Ok(false);
        }
        self.connections.insert(
            signal.connection_id.clone(),
            SignaledConnection {
                remote_peer_id: signal.from_peer_id.clone(),
                role: SignaledPeerRole::Responder,
                remote_description_set: false,
                queued_candidates: Vec::new(),
            },
        );
        self.active_by_remote
            .insert(signal.from_peer_id.clone(), signal.connection_id.clone());
        Ok(true)
    }

    fn resolve_duplicate_offer(
        &mut self,
        signal: &SignalEnvelope,
        commands: &mut Vec<SignaledPeerCommand>,
    ) -> bool {
        let Some(existing_id) = self.active_by_remote.get(&signal.from_peer_id).cloned() else {
            return false;
        };
        if existing_id == signal.connection_id {
            return false;
        }
        let existing_role = self
            .connections
            .get(&existing_id)
            .map(|connection| connection.role);
        let local_initiator_loses = existing_role == Some(SignaledPeerRole::Initiator)
            && self.local_peer_id > signal.from_peer_id;
        if !local_initiator_loses {
            commands.push(SignaledPeerCommand::CloseConnection {
                connection_id: signal.connection_id.clone(),
                remote_peer_id: signal.from_peer_id.clone(),
                reason: SignaledPeerCloseReason::DuplicateConnection,
            });
            return true;
        }
        commands.extend(
            self.close_connection(&existing_id, SignaledPeerCloseReason::DuplicateConnection),
        );
        false
    }

    fn resolve_duplicate_candidate(
        &mut self,
        signal: &SignalEnvelope,
        commands: &mut Vec<SignaledPeerCommand>,
    ) -> bool {
        let Some(existing_id) = self.active_by_remote.get(&signal.from_peer_id).cloned() else {
            return false;
        };
        if existing_id == signal.connection_id {
            return false;
        }
        commands.push(SignaledPeerCommand::CloseConnection {
            connection_id: signal.connection_id.clone(),
            remote_peer_id: signal.from_peer_id.clone(),
            reason: SignaledPeerCloseReason::DuplicateConnection,
        });
        true
    }

    fn close_connection(
        &mut self,
        connection_id: &str,
        reason: SignaledPeerCloseReason,
    ) -> Vec<SignaledPeerCommand> {
        let Some(connection) = self.connections.remove(connection_id) else {
            return Vec::new();
        };
        if self
            .active_by_remote
            .get(&connection.remote_peer_id)
            .is_some_and(|active_id| active_id == connection_id)
        {
            self.active_by_remote.remove(&connection.remote_peer_id);
        }
        vec![SignaledPeerCommand::CloseConnection {
            connection_id: connection_id.to_string(),
            remote_peer_id: connection.remote_peer_id,
            reason,
        }]
    }

    fn next_stream_id(&mut self) -> u64 {
        let stream_id = self.next_stream_id;
        self.next_stream_id += 1;
        stream_id
    }

    fn active_stream_id(&self, connection_id: &str, protocol: &str) -> Option<u64> {
        self.active_streams
            .iter()
            .find(|(_, stream)| {
                stream.connection_id == connection_id && stream.protocol == protocol
            })
            .map(|(stream_id, _)| *stream_id)
    }
}

#[derive(Debug, Clone)]
struct SignaledConnection {
    remote_peer_id: String,
    role: SignaledPeerRole,
    remote_description_set: bool,
    queued_candidates: Vec<String>,
}

type FramedHandler = Arc<dyn Fn(Vec<u8>) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Debug, Clone)]
struct PendingStream {
    connection_id: String,
    protocol: String,
}

#[derive(Debug, Clone)]
struct ActiveStream {
    connection_id: String,
    protocol: String,
}

fn validate_peer_and_connection(
    remote_peer_id: &str,
    connection_id: &str,
) -> Result<(), SignaledPeerError> {
    if remote_peer_id.is_empty() {
        return Err(SignaledPeerError::MissingRemotePeerId);
    }
    if connection_id.is_empty() {
        return Err(SignaledPeerError::MissingConnectionId);
    }
    Ok(())
}

fn validate_connection_and_protocol(
    connection_id: &str,
    protocol: &str,
) -> Result<(), SignaledPeerError> {
    if connection_id.is_empty() {
        return Err(SignaledPeerError::MissingConnectionId);
    }
    validate_protocol(protocol)
}

fn validate_protocol(protocol: &str) -> Result<(), SignaledPeerError> {
    if protocol.is_empty() {
        return Err(SignaledPeerError::MissingProtocol);
    }
    Ok(())
}

fn stream_envelope(kind: &str, payload_json: &str) -> Result<Vec<u8>, SignaledPeerError> {
    let payload: serde_json::Value = serde_json::from_str(payload_json)
        .map_err(|err| SignaledPeerError::InvalidJson(err.to_string()))?;
    let mut envelope = serde_json::Map::new();
    envelope.insert(kind.to_string(), payload);
    serde_json::to_vec(&serde_json::Value::Object(envelope))
        .map_err(|err| SignaledPeerError::InvalidJson(err.to_string()))
}

fn compact_json(value: &serde_json::Value) -> Result<String, SignaledPeerError> {
    serde_json::to_string(value).map_err(|err| SignaledPeerError::InvalidJson(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_dial_emits_peer_connection_command() {
        let mut peer = SignaledPeerCore::new("peer-a".into(), "http://discovery".into()).unwrap();
        let commands = peer.connect("peer-b".into(), "conn-1".into()).unwrap();

        assert_eq!(
            commands[0],
            SignaledPeerCommand::CreatePeerConnection {
                connection_id: "conn-1".into(),
                remote_peer_id: "peer-b".into(),
                role: SignaledPeerRole::Initiator,
            }
        );
    }

    #[test]
    fn inbound_offer_creates_responder_and_sets_remote_description() {
        let mut peer = SignaledPeerCore::new("peer-b".into(), "http://discovery".into()).unwrap();
        let commands = peer
            .handle_signal(SignalEnvelope {
                from_peer_id: "peer-a".into(),
                connection_id: "conn-1".into(),
                kind: "offer".into(),
                payload_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
            })
            .unwrap();

        assert!(
            commands.contains(&SignaledPeerCommand::CreatePeerConnection {
                connection_id: "conn-1".into(),
                remote_peer_id: "peer-a".into(),
                role: SignaledPeerRole::Responder,
            })
        );
        assert!(
            commands.contains(&SignaledPeerCommand::SetRemoteDescription {
                connection_id: "conn-1".into(),
                sdp_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
            })
        );
    }

    #[test]
    fn candidate_before_remote_description_is_queued_and_flushed_after_offer() {
        let mut peer = SignaledPeerCore::new("peer-b".into(), "http://discovery".into()).unwrap();
        let candidate = r#"{"candidate":"candidate:1"}"#;

        let candidate_commands = peer
            .handle_signal(SignalEnvelope {
                from_peer_id: "peer-a".into(),
                connection_id: "conn-1".into(),
                kind: "candidate".into(),
                payload_json: candidate.into(),
            })
            .unwrap();

        assert_eq!(
            candidate_commands,
            vec![SignaledPeerCommand::CreatePeerConnection {
                connection_id: "conn-1".into(),
                remote_peer_id: "peer-a".into(),
                role: SignaledPeerRole::Responder,
            }]
        );

        let offer_commands = peer
            .handle_signal(SignalEnvelope {
                from_peer_id: "peer-a".into(),
                connection_id: "conn-1".into(),
                kind: "offer".into(),
                payload_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
            })
            .unwrap();

        assert_eq!(
            offer_commands,
            vec![
                SignaledPeerCommand::SetRemoteDescription {
                    connection_id: "conn-1".into(),
                    sdp_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
                },
                SignaledPeerCommand::AddIceCandidate {
                    connection_id: "conn-1".into(),
                    candidate_json: candidate.into(),
                }
            ]
        );
    }

    #[test]
    fn simultaneous_dial_closes_duplicate_offer_when_local_peer_wins_tie_break() {
        let mut peer = SignaledPeerCore::new("peer-a".into(), "http://discovery".into()).unwrap();
        peer.connect("peer-b".into(), "local-conn".into()).unwrap();

        let commands = peer
            .handle_signal(SignalEnvelope {
                from_peer_id: "peer-b".into(),
                connection_id: "remote-conn".into(),
                kind: "offer".into(),
                payload_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
            })
            .unwrap();

        assert_eq!(
            commands,
            vec![SignaledPeerCommand::CloseConnection {
                connection_id: "remote-conn".into(),
                remote_peer_id: "peer-b".into(),
                reason: SignaledPeerCloseReason::DuplicateConnection,
            }]
        );
    }

    #[test]
    fn framed_router_invokes_registered_handler_and_sends_requests() {
        let mut peer = SignaledPeerCore::new("peer-a".into(), "http://discovery".into()).unwrap();
        peer.handle_framed("/auki/info/0.0.1".into(), |payload| {
            assert_eq!(payload, br#"{"request":true}"#.to_vec());
            Some(br#"{"ok":true}"#.to_vec())
        })
        .unwrap();

        let response = peer
            .receive_framed("/auki/info/0.0.1", br#"{"request":true}"#.to_vec())
            .unwrap();
        assert_eq!(response, Some(br#"{"ok":true}"#.to_vec()));

        let commands = peer
            .request_framed(
                "conn-1".into(),
                "/auki/info/0.0.1".into(),
                br#"{"ping":true}"#.to_vec(),
            )
            .unwrap();
        assert_eq!(
            commands,
            vec![SignaledPeerCommand::SendDataChannelMessage {
                connection_id: "conn-1".into(),
                protocol: "/auki/info/0.0.1".into(),
                payload: br#"{"ping":true}"#.to_vec(),
            }]
        );
    }

    #[test]
    fn stream_router_accepts_open_request_and_emits_entries() {
        let mut peer = SignaledPeerCore::new("peer-b".into(), "http://discovery".into()).unwrap();
        peer.handle_stream("/auki/stream/0.1.0".into()).unwrap();

        let events = peer
            .receive_stream_message(
                "conn-1".into(),
                "peer-a".into(),
                "/auki/stream/0.1.0".into(),
                br#"{"request":{"sensor_id":"camera"}}"#.to_vec(),
            )
            .unwrap();

        let SignaledPeerEvent::StreamOpenRequest(open) = &events[0] else {
            panic!("expected stream open request");
        };
        assert_eq!(open.stream_id, 1);
        assert_eq!(open.connection_id, "conn-1");
        assert_eq!(open.remote_peer_id, "peer-a");
        assert_eq!(open.protocol, "/auki/stream/0.1.0");
        assert_eq!(open.request_json, r#"{"sensor_id":"camera"}"#);

        let accept_commands = peer
            .accept_stream_open(
                open.stream_id,
                r#"{"sensor_id":"camera","sensor_hash":"sensor-hash","clock_id":"clock","clock_hash":"clock-hash","frame_id":"frame","frame_hash":"frame-hash"}"#.into(),
            )
            .unwrap();
        assert_eq!(
            command_payload_json(&accept_commands[0]),
            serde_json::json!({
                "accept": {
                    "sensor_id": "camera",
                    "sensor_hash": "sensor-hash",
                    "clock_id": "clock",
                    "clock_hash": "clock-hash",
                    "frame_id": "frame",
                    "frame_hash": "frame-hash"
                }
            })
        );

        let entry_commands = peer
            .push_stream_entry(
                open.stream_id,
                r#"{"timestamp_ns":1,"seq":0,"payload":[1,2,3]}"#.into(),
            )
            .unwrap();
        assert_eq!(
            command_payload_json(&entry_commands[0]),
            serde_json::json!({
                "entry": {
                    "timestamp_ns": 1,
                    "seq": 0,
                    "payload": [1, 2, 3]
                }
            })
        );
    }

    fn command_payload_json(command: &SignaledPeerCommand) -> serde_json::Value {
        let SignaledPeerCommand::SendDataChannelMessage { payload, .. } = command else {
            panic!("expected SendDataChannelMessage");
        };
        serde_json::from_slice(payload).unwrap()
    }
}
