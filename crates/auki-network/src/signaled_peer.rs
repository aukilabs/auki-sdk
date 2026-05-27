use std::{collections::HashMap, error::Error, fmt};

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
    DuplicateConnectionId(String),
    UnknownConnectionId(String),
    UnsupportedSignalKind(String),
}

impl fmt::Display for SignaledPeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignaledPeerError::MissingLocalPeerId => write!(f, "missing local peer id"),
            SignaledPeerError::MissingDiscoveryUrl => write!(f, "missing discovery url"),
            SignaledPeerError::MissingRemotePeerId => write!(f, "missing remote peer id"),
            SignaledPeerError::MissingConnectionId => write!(f, "missing connection id"),
            SignaledPeerError::DuplicateConnectionId(connection_id) => {
                write!(f, "duplicate connection id: {connection_id}")
            }
            SignaledPeerError::UnknownConnectionId(connection_id) => {
                write!(f, "unknown connection id: {connection_id}")
            }
            SignaledPeerError::UnsupportedSignalKind(kind) => {
                write!(f, "unsupported signal kind: {kind}")
            }
        }
    }
}

impl Error for SignaledPeerError {}

#[derive(Debug, Clone)]
pub struct SignaledPeerCore {
    local_peer_id: String,
    discovery_url: String,
    connections: HashMap<String, SignaledConnection>,
    active_by_remote: HashMap<String, String>,
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
        })
    }

    pub fn local_peer_id(&self) -> &str {
        &self.local_peer_id
    }

    pub fn discovery_url(&self) -> &str {
        &self.discovery_url
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
}

#[derive(Debug, Clone)]
struct SignaledConnection {
    remote_peer_id: String,
    role: SignaledPeerRole,
    remote_description_set: bool,
    queued_candidates: Vec<String>,
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
}
