//! HTTP client for the Discovery service.
//!
//! Discovery is a bootstrap rendezvous directory: peers consult it on
//! first boot to find existing clusters, then operate peer-to-peer
//! over libp2p from there. Discovery's record is a hint — the cluster
//! owns itself; Discovery's entry catches up on the next Manager
//! registration. What Discovery stores per cluster is deliberately
//! minimal (current Manager's libp2p hint + aggregate `peer_count` +
//! timestamps); the peer list lives on the Manager and is gossiped
//! peer-to-peer, never on Discovery.
//!
//! ## Endpoints
//!
//! - `GET    /clusters`                  — directory snapshot, sorted
//!   by `created_ns` desc (newest-created first).
//! - `POST   /clusters/{name}`           — create. Trust-on-first-claim;
//!   409 if the name is taken.
//! - `POST   /clusters/{name}/liveness`  — Manager liveness check,
//!   pushed every 1s. Body `{ peer_count }`. Resets Discovery's 3s
//!   sweep window for the cluster.
//! - `POST   /clusters/{name}/manager`   — rotate Manager hint (no
//!   crypto in v1).
//! - `DELETE /clusters/{name}`           — graceful deregistration.
//!
//! v1 skips all signature verification — endpoints accept claims by
//! shape. v2 hardening will reintroduce successor-token validation,
//! Discovery-issued challenge/response, and DELETE auth.
//!
//! ## Wire format
//!
//! JSON throughout. Timestamps in unix nanoseconds. `PeerId` rides as
//! its canonical libp2p string form (base58 / multihash); `Multiaddr`
//! rides as its canonical `/ip4/.../tcp/...` text form. The client
//! parses these at the boundary so consumers see typed values. When a
//! Manager address includes `/p2p-circuit`, the client also publishes
//! the relay base address as `relay_multiaddrs` so browser peers can
//! reserve and advertise through the same Domain Relay.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

// ─── Public types ──────────────────────────────────────────────────

/// One cluster's entry in Discovery's directory.
///
/// `manager_peer_id` + `manager_multiaddrs` are the hint a newcomer
/// uses to dial the current Manager over libp2p. `peer_count` is the
/// Manager's most-recent self-reported size (aggregate; no
/// identities). `created_ns` is Discovery's server-stamped creation
/// timestamp and the sort key for `list_clusters`.
/// `last_liveness_check_ns` is the unix-ns of the Manager's most recent
/// liveness check (`0` if the cluster was just created and the Manager
/// hasn't pushed one yet).
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEntry {
    /// The cluster's name. Discovery enforces `^[A-Za-z0-9_-]{1,64}$`.
    pub name: String,
    /// Current Manager's libp2p peer-id.
    pub manager_peer_id: PeerId,
    /// Current Manager's dialable libp2p multiaddrs.
    pub manager_multiaddrs: Vec<Multiaddr>,
    /// Relay base multiaddrs associated with any relay-mediated
    /// Manager addresses. Browsers use these to reserve and advertise
    /// their own circuit address through the same Domain Relay.
    pub relay_multiaddrs: Vec<Multiaddr>,
    /// Manager's most-recent self-reported peer count (aggregate).
    pub peer_count: u32,
    /// Unix nanoseconds, Discovery's server clock at cluster creation.
    /// Sort key for [`DiscoveryClient::list_clusters`] (desc).
    pub created_ns: i64,
    /// Unix nanoseconds, last liveness check received. Discovery stamps
    /// it at create time as well, so on a fresh cluster
    /// `last_liveness_check_ns == created_ns`; subsequent liveness
    /// checks advance it.
    pub last_liveness_check_ns: i64,
}

/// Outcome of [`DiscoveryClient::create_cluster`].
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateClusterOutcome {
    /// `201 Created` — the caller is the new cluster's initial Manager.
    Created(ClusterEntry),
    /// `409 Conflict` — the cluster name was already taken. Re-fetch
    /// the directory with [`DiscoveryClient::list_clusters`] and join
    /// the existing cluster.
    AlreadyExists,
}

/// Failure modes for [`DiscoveryClient`] calls.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// HTTP transport or connection failure.
    #[error("Discovery transport: {0}")]
    Transport(#[from] reqwest::Error),
    /// Discovery returned a non-success status code (other than 409
    /// for `create_cluster`, which is mapped to
    /// [`CreateClusterOutcome::AlreadyExists`]).
    #[error("Discovery returned HTTP {status}: {body}")]
    Status {
        /// HTTP status code Discovery returned.
        status: u16,
        /// Response body as text (typically `{ "error": "…" }`).
        body: String,
    },
    /// Discovery's response carried a `manager_peer_id` that didn't
    /// parse as a libp2p `PeerId`.
    #[error("invalid peer-id in Discovery response: {0}")]
    InvalidPeerId(String),
    /// Discovery's response carried a `manager_multiaddrs` entry that
    /// didn't parse as a libp2p `Multiaddr`.
    #[error("invalid multiaddr in Discovery response: {0}")]
    InvalidMultiaddr(String),
}

// ─── Client ────────────────────────────────────────────────────────

/// HTTP client for a Discovery service instance.
///
/// Construct with a base URL like `http://192.168.9.130:8080`. Methods
/// are async; the caller drives a tokio runtime.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
#[derive(Debug, Clone)]
pub struct DiscoveryClient {
    base_url: String,
    http: reqwest::Client,
}

// ─── Sync methods (exported as the primary constructor + accessor) ──

#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl DiscoveryClient {
    /// Construct against `base_url` with a default `reqwest::Client`.
    /// Trailing `/` on `base_url` is stripped.
    ///
    /// Returns `Arc<Self>` to satisfy the UniFFI 0.31 object-constructor
    /// contract. Rust callers that need a plain `DiscoveryClient` should
    /// use [`Self::with_http`] instead.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new(base_url: String) -> Arc<Self> {
        Arc::new(Self::with_http(base_url, reqwest::Client::new()))
    }

    /// The base URL this client was constructed against (trailing `/`
    /// stripped).
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }
}

impl DiscoveryClient {
    /// Construct against `base_url` with a caller-provided
    /// `reqwest::Client`. Use this to set custom timeouts, proxies,
    /// custom TLS roots, etc.
    ///
    /// Returns `Self` (not `Arc<Self>`) — use this from Rust callers
    /// that store or clone the client directly.
    pub fn with_http(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }
}

// ─── Async methods (exported with tokio runtime) ────────────────────

#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl DiscoveryClient {
    /// Snapshot of Discovery's directory, sorted by `created_ns` desc
    /// (newest cluster first).
    pub async fn list_clusters(&self) -> Result<Vec<ClusterEntry>, DiscoveryError> {
        let url = format!("{}/clusters", self.base_url);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(DiscoveryError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let list: WireListResponse = resp.json().await?;
        list.clusters.into_iter().map(parse_wire_entry).collect()
    }

    /// Atomically create a new cluster. The caller becomes its initial
    /// Manager. Returns [`CreateClusterOutcome::AlreadyExists`] (not
    /// an error) when the name is taken — the caller can list and
    /// join the existing cluster.
    pub async fn create_cluster(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
    ) -> Result<CreateClusterOutcome, DiscoveryError> {
        let url = format!("{}/clusters/{}", self.base_url, name);
        let body = WireManagerRequest::from_multiaddrs(manager_peer_id, &manager_multiaddrs);
        let resp = self.http.post(&url).json(&body).send().await?;
        match resp.status() {
            StatusCode::CREATED => {
                let wire: WireClusterEntry = resp.json().await?;
                Ok(CreateClusterOutcome::Created(parse_wire_entry(wire)?))
            }
            StatusCode::CONFLICT => Ok(CreateClusterOutcome::AlreadyExists),
            status => Err(DiscoveryError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            }),
        }
    }

    /// Manager push: report aggregate `peer_count`. Discovery refreshes
    /// `last_liveness_check_ns` and resets the 3s sweep window
    /// (`LIVENESS_REQUIREMENT_NS`). Returns the updated `ClusterEntry`.
    pub async fn liveness_check(
        &self,
        name: String,
        peer_count: u32,
    ) -> Result<ClusterEntry, DiscoveryError> {
        let url = format!("{}/clusters/{}/liveness", self.base_url, name);
        let body = WireLivenessCheckRequest { peer_count };
        let resp = self.http.post(&url).json(&body).send().await?;
        ok_or_status(resp).await
    }

    /// Rotate the Manager hint. Called by a newly-elected Manager after
    /// a successor election to update Discovery's directory entry.
    /// Returns the updated `ClusterEntry`.
    pub async fn rotate_manager(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        let url = format!("{}/clusters/{}/manager", self.base_url, name);
        let body = WireManagerRequest::from_multiaddrs(manager_peer_id, &manager_multiaddrs);
        let resp = self.http.post(&url).json(&body).send().await?;
        ok_or_status(resp).await
    }

    /// Graceful deregistration. Called by the last cluster member on
    /// clean exit. v1 = trust-the-claim; any HTTP caller succeeds.
    pub async fn deregister(&self, name: String) -> Result<(), DiscoveryError> {
        let url = format!("{}/clusters/{}", self.base_url, name);
        let resp = self.http.delete(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(DiscoveryError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            })
        }
    }
}

// ─── Wire shapes (internal) ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireClusterEntry {
    name: String,
    manager_peer_id: String,
    manager_multiaddrs: Vec<String>,
    #[serde(default)]
    relay_multiaddrs: Vec<String>,
    peer_count: u32,
    created_ns: i64,
    last_liveness_check_ns: i64,
}

#[derive(Debug, Deserialize)]
struct WireListResponse {
    clusters: Vec<WireClusterEntry>,
}

#[derive(Debug, Serialize)]
struct WireManagerRequest {
    manager_peer_id: String,
    manager_multiaddrs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    relay_multiaddrs: Vec<String>,
}

impl WireManagerRequest {
    fn from_multiaddrs(manager_peer_id: PeerId, manager_multiaddrs: &[Multiaddr]) -> Self {
        Self {
            manager_peer_id: manager_peer_id.to_string(),
            manager_multiaddrs: manager_multiaddrs.iter().map(|m| m.to_string()).collect(),
            relay_multiaddrs: relay_multiaddrs_from_manager_multiaddrs(manager_multiaddrs),
        }
    }
}

#[derive(Debug, Serialize)]
struct WireLivenessCheckRequest {
    peer_count: u32,
}

fn parse_wire_entry(w: WireClusterEntry) -> Result<ClusterEntry, DiscoveryError> {
    let manager_peer_id = PeerId::from_str(&w.manager_peer_id)
        .map_err(|e| DiscoveryError::InvalidPeerId(format!("{}: {}", w.manager_peer_id, e)))?;
    let manager_multiaddrs = w
        .manager_multiaddrs
        .into_iter()
        .map(|s| {
            Multiaddr::from_str(&s)
                .map_err(|e| DiscoveryError::InvalidMultiaddr(format!("{}: {}", s, e)))
        })
        .collect::<Result<_, _>>()?;
    let relay_multiaddrs = w
        .relay_multiaddrs
        .into_iter()
        .map(|s| {
            Multiaddr::from_str(&s)
                .map_err(|e| DiscoveryError::InvalidMultiaddr(format!("{}: {}", s, e)))
        })
        .collect::<Result<_, _>>()?;
    Ok(ClusterEntry {
        name: w.name,
        manager_peer_id,
        manager_multiaddrs,
        relay_multiaddrs,
        peer_count: w.peer_count,
        created_ns: w.created_ns,
        last_liveness_check_ns: w.last_liveness_check_ns,
    })
}

fn relay_multiaddrs_from_manager_multiaddrs(manager_multiaddrs: &[Multiaddr]) -> Vec<String> {
    let mut relay_multiaddrs = Vec::new();
    for addr in manager_multiaddrs {
        let Some(relay) = relay_base_multiaddr_from_circuit(addr) else {
            continue;
        };
        let relay = relay.to_string();
        if !relay_multiaddrs.contains(&relay) {
            relay_multiaddrs.push(relay);
        }
    }
    relay_multiaddrs
}

fn relay_base_multiaddr_from_circuit(addr: &Multiaddr) -> Option<Multiaddr> {
    let mut base = Multiaddr::empty();
    let mut saw_relay_peer_id = false;
    for protocol in addr.iter() {
        if matches!(protocol, multiaddr::Protocol::P2pCircuit) {
            return saw_relay_peer_id.then_some(base);
        }
        if matches!(protocol, multiaddr::Protocol::P2p(_)) {
            saw_relay_peer_id = true;
        }
        base.push(protocol);
    }
    None
}

async fn ok_or_status(resp: reqwest::Response) -> Result<ClusterEntry, DiscoveryError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(DiscoveryError::Status {
            status: status.as_u16(),
            body: resp.text().await.unwrap_or_default(),
        });
    }
    let wire: WireClusterEntry = resp.json().await?;
    parse_wire_entry(wire)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire_entry() -> WireClusterEntry {
        let relay_peer_id = crate::PeerIdentity::from_seed(&[201u8; 32]).peer_id();
        WireClusterEntry {
            name: "foo".to_string(),
            manager_peer_id: "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw".to_string(),
            manager_multiaddrs: vec!["/ip4/192.168.9.10/tcp/4001".to_string()],
            relay_multiaddrs: vec![format!("/ip4/192.168.9.10/tcp/4002/ws/p2p/{relay_peer_id}")],
            peer_count: 3,
            created_ns: 1_715_423_400_000_000_000,
            last_liveness_check_ns: 1_715_423_500_000_000_000,
        }
    }

    #[test]
    fn parse_wire_entry_round_trips_via_typed_form() {
        let w = sample_wire_entry();
        let entry = parse_wire_entry(w.clone()).expect("valid wire entry parses");
        assert_eq!(entry.name, w.name);
        assert_eq!(entry.manager_peer_id.to_string(), w.manager_peer_id);
        assert_eq!(
            entry
                .manager_multiaddrs
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>(),
            w.manager_multiaddrs
        );
        assert_eq!(
            entry
                .relay_multiaddrs
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>(),
            w.relay_multiaddrs
        );
        assert_eq!(entry.peer_count, w.peer_count);
        assert_eq!(entry.created_ns, w.created_ns);
        assert_eq!(entry.last_liveness_check_ns, w.last_liveness_check_ns);
    }

    #[test]
    fn parse_wire_entry_rejects_garbage_peer_id() {
        let mut w = sample_wire_entry();
        w.manager_peer_id = "not-a-peer-id".to_string();
        let err = parse_wire_entry(w).expect_err("garbage peer-id is rejected");
        assert!(
            matches!(err, DiscoveryError::InvalidPeerId(_)),
            "expected InvalidPeerId, got {err:?}"
        );
    }

    #[test]
    fn parse_wire_entry_rejects_garbage_multiaddr() {
        let mut w = sample_wire_entry();
        w.manager_multiaddrs = vec!["not-a-multiaddr".to_string()];
        let err = parse_wire_entry(w).expect_err("garbage multiaddr is rejected");
        assert!(
            matches!(err, DiscoveryError::InvalidMultiaddr(_)),
            "expected InvalidMultiaddr, got {err:?}"
        );
    }

    #[test]
    fn parse_wire_entry_accepts_legacy_rows_without_relay_multiaddrs() {
        let json = serde_json::json!({
            "name": "foo",
            "manager_peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
            "manager_multiaddrs": ["/ip4/192.168.9.10/tcp/4001"],
            "peer_count": 3,
            "created_ns": 1_715_423_400_000_000_000i64,
            "last_liveness_check_ns": 1_715_423_500_000_000_000i64,
        });
        let wire: WireClusterEntry = serde_json::from_value(json).unwrap();
        let entry = parse_wire_entry(wire).unwrap();

        assert!(entry.relay_multiaddrs.is_empty());
    }

    #[test]
    fn client_trims_trailing_slash_from_base_url() {
        let a = DiscoveryClient::new("http://example.com:8080".to_string());
        let b = DiscoveryClient::new("http://example.com:8080/".to_string());
        assert_eq!(a.base_url(), b.base_url());
        assert_eq!(a.base_url(), "http://example.com:8080");
    }

    /// Pins the wire shape Discovery serializes against. A field rename
    /// on either side trips this test.
    #[test]
    fn wire_entry_field_names_are_locked() {
        let json = serde_json::to_string(&sample_wire_entry()).unwrap();
        for key in [
            "\"name\":",
            "\"manager_peer_id\":",
            "\"manager_multiaddrs\":",
            "\"relay_multiaddrs\":",
            "\"peer_count\":",
            "\"created_ns\":",
            "\"last_liveness_check_ns\":",
        ] {
            assert!(json.contains(key), "missing wire key {key:?} in {json}");
        }
    }

    /// Pins the wire shape the client SENDS on `create_cluster` /
    /// `rotate_manager` against Discovery's `CreateClusterRequest` /
    /// `RotateManagerRequest`. A field rename on either side trips
    /// this test.
    #[test]
    fn wire_manager_request_field_names_are_locked() {
        let req = WireManagerRequest {
            manager_peer_id: "12D3KooW…".to_string(),
            manager_multiaddrs: vec!["/ip4/1.2.3.4/tcp/1".to_string()],
            relay_multiaddrs: vec!["/ip4/1.2.3.4/tcp/2/ws/p2p/12D3KooWRelay".to_string()],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"manager_peer_id\":"), "{json}");
        assert!(json.contains("\"manager_multiaddrs\":"), "{json}");
        assert!(json.contains("\"relay_multiaddrs\":"), "{json}");
    }

    #[test]
    fn wire_manager_request_derives_relay_base_from_circuit_manager_addr() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[202u8; 32]).peer_id();
        let relay_peer_id = crate::PeerIdentity::from_seed(&[203u8; 32]).peer_id();
        let native: Multiaddr = "/ip4/192.168.9.130/tcp/4001".parse().unwrap();
        let relay_base: Multiaddr = format!("/ip4/192.168.9.130/tcp/4002/ws/p2p/{relay_peer_id}")
            .parse()
            .unwrap();
        let circuit = relay_base
            .clone()
            .with(multiaddr::Protocol::P2pCircuit)
            .with(multiaddr::Protocol::P2p(manager_peer_id));

        let req = WireManagerRequest::from_multiaddrs(manager_peer_id, &[native, circuit]);

        assert_eq!(
            req.relay_multiaddrs,
            vec![relay_base.to_string()],
            "Discovery should receive relay base addresses separately from Manager circuit addresses"
        );
    }

    #[test]
    fn wire_manager_request_omits_relay_multiaddrs_without_circuit_addr() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[204u8; 32]).peer_id();
        let native: Multiaddr = "/ip4/192.168.9.130/tcp/4001".parse().unwrap();

        let req = WireManagerRequest::from_multiaddrs(manager_peer_id, &[native]);
        let json = serde_json::to_string(&req).unwrap();

        assert!(!json.contains("relay_multiaddrs"), "{json}");
    }

    #[test]
    fn wire_manager_request_omits_relay_multiaddrs_when_circuit_lacks_relay_peer_id() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[205u8; 32]).peer_id();
        let malformed_circuit: Multiaddr =
            format!("/ip4/192.168.9.130/tcp/4002/ws/p2p-circuit/p2p/{manager_peer_id}")
                .parse()
                .unwrap();

        let req = WireManagerRequest::from_multiaddrs(manager_peer_id, &[malformed_circuit]);
        let json = serde_json::to_string(&req).unwrap();

        assert!(!json.contains("relay_multiaddrs"), "{json}");
    }

    /// Same for the liveness-check body.
    #[test]
    fn wire_liveness_check_request_field_name_is_locked() {
        let req = WireLivenessCheckRequest { peer_count: 7 };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"peer_count":7}"#);
    }
}
