//! Runtime transport-path status helpers.

use libp2p::{Multiaddr, core::ConnectedPoint};
use multiaddr::Protocol;
use serde_json::{Map, Value};

/// Local role on an established libp2p connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AukiConnectionDirection {
    /// Local peer dialed the remote.
    Dialer,
    /// Local peer accepted an inbound connection.
    Listener,
}

/// Coarse transport family observed for an established connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AukiTransportProtocol {
    /// TCP transport.
    Tcp,
    /// QUIC transport.
    Quic,
    /// WebSocket transport.
    WebSocket,
    /// WebRTC Direct transport.
    WebRtcDirect,
    /// Browser WebRTC transport.
    WebRtc,
    /// WebTransport transport.
    WebTransport,
    /// Transport could not be classified from the endpoint address.
    Unknown,
}

/// Runtime-observed transport path for one retained connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiConnectionPath {
    /// Local connection role.
    pub direction: AukiConnectionDirection,
    /// Coarse transport family.
    pub transport: AukiTransportProtocol,
    /// Whether the path includes a Circuit Relay hop.
    pub relay_involved: bool,
    /// Local connection address for inbound connections.
    pub local_address: Option<Multiaddr>,
    /// Remote or dialed address for the connection.
    pub remote_address: Multiaddr,
}

impl AukiConnectionDirection {
    /// Stable status string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dialer => "dialer",
            Self::Listener => "listener",
        }
    }
}

impl AukiTransportProtocol {
    /// Stable status string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Quic => "quic",
            Self::WebSocket => "websocket",
            Self::WebRtcDirect => "webrtc_direct",
            Self::WebRtc => "webrtc",
            Self::WebTransport => "webtransport",
            Self::Unknown => "unknown",
        }
    }
}

impl AukiConnectionPath {
    /// Classify a libp2p endpoint into demo/status-friendly transport state.
    pub fn from_endpoint(endpoint: &ConnectedPoint) -> Self {
        match endpoint {
            ConnectedPoint::Dialer { address, .. } => Self {
                direction: AukiConnectionDirection::Dialer,
                transport: classify_transport(address, None),
                relay_involved: has_relay_hop(address),
                local_address: None,
                remote_address: address.clone(),
            },
            ConnectedPoint::Listener {
                local_addr,
                send_back_addr,
            } => Self {
                direction: AukiConnectionDirection::Listener,
                transport: classify_transport(send_back_addr, Some(local_addr)),
                relay_involved: has_relay_hop(send_back_addr) || has_relay_hop(local_addr),
                local_address: Some(local_addr.clone()),
                remote_address: send_back_addr.clone(),
            },
        }
    }

    /// Project the path into status JSON. Address fields are optional so
    /// privacy redaction can keep transport/relay diagnostics visible without
    /// leaking endpoint addresses.
    pub fn to_status_value(&self, include_addresses: bool) -> Value {
        let mut object = Map::new();
        object.insert(
            "direction".to_owned(),
            Value::String(self.direction.as_str().to_owned()),
        );
        object.insert(
            "transport".to_owned(),
            Value::String(self.transport.as_str().to_owned()),
        );
        object.insert(
            "relay_involved".to_owned(),
            Value::Bool(self.relay_involved),
        );
        if include_addresses {
            object.insert(
                "remote_address".to_owned(),
                Value::String(self.remote_address.to_string()),
            );
            if let Some(local_address) = &self.local_address {
                object.insert(
                    "local_address".to_owned(),
                    Value::String(local_address.to_string()),
                );
            }
        }
        Value::Object(object)
    }
}

fn classify_transport(primary: &Multiaddr, secondary: Option<&Multiaddr>) -> AukiTransportProtocol {
    transport_from_address(primary)
        .or_else(|| secondary.and_then(transport_from_address))
        .unwrap_or(AukiTransportProtocol::Unknown)
}

fn transport_from_address(address: &Multiaddr) -> Option<AukiTransportProtocol> {
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebRTCDirect))
    {
        return Some(AukiTransportProtocol::WebRtcDirect);
    }
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebRTC))
    {
        return Some(AukiTransportProtocol::WebRtc);
    }
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::WebTransport))
    {
        return Some(AukiTransportProtocol::WebTransport);
    }
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Ws(_) | Protocol::Wss(_)))
    {
        return Some(AukiTransportProtocol::WebSocket);
    }
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Quic | Protocol::QuicV1))
    {
        return Some(AukiTransportProtocol::Quic);
    }
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::Tcp(_)))
    {
        return Some(AukiTransportProtocol::Tcp);
    }
    None
}

fn has_relay_hop(address: &Multiaddr) -> bool {
    address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_direct_dialer_endpoint() {
        let endpoint = ConnectedPoint::Dialer {
            address: "/ip4/127.0.0.1/udp/4001/quic-v1".parse().unwrap(),
            role_override: libp2p::core::Endpoint::Dialer,
            port_use: libp2p::core::transport::PortUse::New,
        };

        let path = AukiConnectionPath::from_endpoint(&endpoint);

        assert_eq!(path.direction, AukiConnectionDirection::Dialer);
        assert_eq!(path.transport, AukiTransportProtocol::Quic);
        assert!(!path.relay_involved);
        assert!(path.local_address.is_none());
    }

    #[test]
    fn classifies_relayed_websocket_dialer_endpoint() {
        let endpoint = ConnectedPoint::Dialer {
            address: "/ip4/127.0.0.1/tcp/4001/ws/p2p/12D3KooWEBeYWDsJk8FH87dWMPxZzMZ8pvG8hy9EpcSxHfXQBPzV/p2p-circuit/p2p/12D3KooWMwg8PCFBbgbuwmKoUwWVVMHNLyrXWMA6Jukm5wpk59h4"
                .parse()
                .unwrap(),
            role_override: libp2p::core::Endpoint::Dialer,
            port_use: libp2p::core::transport::PortUse::New,
        };

        let path = AukiConnectionPath::from_endpoint(&endpoint);
        let value = path.to_status_value(true);

        assert_eq!(path.transport, AukiTransportProtocol::WebSocket);
        assert!(path.relay_involved);
        assert_eq!(
            value.get("relay_involved").and_then(Value::as_bool),
            Some(true)
        );
        assert!(value.get("remote_address").is_some());
    }

    #[test]
    fn status_value_can_redact_addresses_without_hiding_relay_state() {
        let endpoint = ConnectedPoint::Listener {
            local_addr: "/ip4/127.0.0.1/tcp/4001/ws".parse().unwrap(),
            send_back_addr: "/ip4/127.0.0.1/tcp/5001/ws".parse().unwrap(),
        };
        let path = AukiConnectionPath::from_endpoint(&endpoint);

        let value = path.to_status_value(false);

        assert_eq!(
            value.get("transport").and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            value.get("relay_involved").and_then(Value::as_bool),
            Some(false)
        );
        assert!(value.get("remote_address").is_none());
        assert!(value.get("local_address").is_none());
    }
}
