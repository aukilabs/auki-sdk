//! Domain facade over the SDK-owned signaled WebRTC peer core.

use std::collections::HashSet;

/// Errors from the Domain signaled peer facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignaledDomainPeerError {
    /// The cluster name was empty.
    MissingClusterName,
    /// The underlying network signaled peer rejected the input or operation.
    Network(String),
    /// JSON input was invalid for the requested Domain surface.
    InvalidJson(String),
}

impl std::fmt::Display for SignaledDomainPeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingClusterName => write!(f, "missing cluster name"),
            Self::Network(message) => write!(f, "network error: {message}"),
            Self::InvalidJson(message) => write!(f, "invalid json: {message}"),
        }
    }
}

impl std::error::Error for SignaledDomainPeerError {}

/// Minimal Domain-level facade for Discovery-signaled WebRTC peers.
pub struct SignaledDomainPeer {
    cluster_name: String,
    network: auki_network::SignaledPeerCore,
    sensor_catalog_json: String,
    resource_catalog_json: String,
    registry_entries_json: String,
    finished_streams: HashSet<u64>,
}

impl SignaledDomainPeer {
    /// Construct a signaled Domain peer around a local peer id and Discovery URL.
    pub fn new(
        local_peer_id: String,
        discovery_url: String,
        cluster_name: String,
    ) -> Result<Self, SignaledDomainPeerError> {
        if cluster_name.is_empty() {
            return Err(SignaledDomainPeerError::MissingClusterName);
        }
        let mut network = auki_network::SignaledPeerCore::new(local_peer_id, discovery_url)
            .map_err(network_error)?;
        network
            .handle_stream("/auki/stream/0.1.0".to_string())
            .map_err(network_error)?;
        Ok(Self {
            cluster_name,
            network,
            sensor_catalog_json: r#"{"sensors":[]}"#.to_string(),
            resource_catalog_json: r#"{"resources":[]}"#.to_string(),
            registry_entries_json: r#"{"entries":[]}"#.to_string(),
            finished_streams: HashSet::new(),
        })
    }

    /// Return the local peer id.
    pub fn local_peer_id(&self) -> &str {
        self.network.local_peer_id()
    }

    /// Return the cluster name this peer joins or creates.
    pub fn cluster_name(&self) -> &str {
        &self.cluster_name
    }

    /// Return the advertised signaled WebRTC multiaddrs for this peer.
    pub fn multiaddrs(&self) -> Result<Vec<String>, SignaledDomainPeerError> {
        Ok(vec![
            auki_network::format_signaled_address(
                self.network.discovery_url(),
                self.network.local_peer_id(),
            )
            .map_err(|err| SignaledDomainPeerError::Network(err.to_string()))?,
        ])
    }

    /// Replace the static sensor catalog JSON.
    pub fn set_static_sensor_catalog_json(
        &mut self,
        catalog_json: String,
    ) -> Result<(), SignaledDomainPeerError> {
        self.sensor_catalog_json =
            super::validate_sensor_catalog_json(&catalog_json).map_err(data_error)?;
        Ok(())
    }

    /// Replace the static resource catalog JSON.
    pub fn set_static_resource_catalog_json(
        &mut self,
        catalog_json: String,
    ) -> Result<(), SignaledDomainPeerError> {
        self.resource_catalog_json =
            super::validate_resource_catalog_json(&catalog_json).map_err(data_error)?;
        Ok(())
    }

    /// Replace the static registry entries JSON.
    pub fn set_static_registry_entries_json(
        &mut self,
        entries_json: String,
    ) -> Result<(), SignaledDomainPeerError> {
        let value: serde_json::Value = serde_json::from_str(&entries_json)
            .map_err(|err| SignaledDomainPeerError::InvalidJson(err.to_string()))?;
        self.registry_entries_json = serde_json::to_string(&value)
            .map_err(|err| SignaledDomainPeerError::InvalidJson(err.to_string()))?;
        Ok(())
    }

    /// Return the static sensor catalog JSON.
    pub fn sensor_catalog_json(&self) -> &str {
        &self.sensor_catalog_json
    }

    /// Return the static resource catalog JSON.
    pub fn resource_catalog_json(&self) -> &str {
        &self.resource_catalog_json
    }

    /// Return the static registry entries JSON.
    pub fn registry_entries_json(&self) -> &str {
        &self.registry_entries_json
    }

    /// Accept a pending stream-open request.
    pub fn accept_stream_open(
        &mut self,
        stream_id: u64,
        manifest_json: String,
    ) -> Result<(), SignaledDomainPeerError> {
        self.network
            .accept_stream_open(stream_id, manifest_json)
            .map_err(network_error)?;
        Ok(())
    }

    /// Decline a pending stream-open request.
    pub fn decline_stream_open(
        &mut self,
        stream_id: u64,
        _reason: String,
    ) -> Result<(), SignaledDomainPeerError> {
        self.finished_streams.insert(stream_id);
        Ok(())
    }

    /// Push one stream entry onto an accepted stream.
    pub fn push_stream_entry(
        &self,
        stream_id: u64,
        entry_json: String,
    ) -> Result<(), SignaledDomainPeerError> {
        self.network
            .push_stream_entry(stream_id, entry_json)
            .map_err(network_error)?;
        Ok(())
    }

    /// Finish a stream locally.
    pub fn finish_stream(&mut self, stream_id: u64) {
        self.finished_streams.insert(stream_id);
    }
}

fn network_error(error: auki_network::SignaledPeerError) -> SignaledDomainPeerError {
    SignaledDomainPeerError::Network(error.to_string())
}

fn data_error(error: super::DomainDataError) -> SignaledDomainPeerError {
    SignaledDomainPeerError::InvalidJson(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_signaled_multiaddr_and_static_catalogs() {
        let mut peer = SignaledDomainPeer::new(
            "peer-a".to_string(),
            "http://discovery.local/".to_string(),
            "cluster-a".to_string(),
        )
        .unwrap();

        assert_eq!(peer.local_peer_id(), "peer-a");
        assert_eq!(peer.cluster_name(), "cluster-a");
        assert_eq!(peer.sensor_catalog_json(), r#"{"sensors":[]}"#);
        assert_eq!(peer.resource_catalog_json(), r#"{"resources":[]}"#);
        assert_eq!(peer.registry_entries_json(), r#"{"entries":[]}"#);
        assert_eq!(
            auki_network::parse_signaled_address(&peer.multiaddrs().unwrap()[0])
                .unwrap()
                .discovery_url,
            "http://discovery.local"
        );

        peer.set_static_sensor_catalog_json(
            r#"{"sensors":[{"sensor_id":"camera","sensor_hash":"hash","kind":"camera"}]}"#
                .to_string(),
        )
        .unwrap();
        assert!(peer.sensor_catalog_json().contains("camera"));
    }
}
