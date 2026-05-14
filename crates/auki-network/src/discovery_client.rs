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
//! parses these at the boundary so consumers see typed values.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEntry {
    /// The cluster's name. Discovery enforces `^[A-Za-z0-9_-]{1,64}$`.
    pub name: String,
    /// Current Manager's libp2p peer-id.
    pub manager_peer_id: PeerId,
    /// Current Manager's dialable libp2p multiaddrs.
    pub manager_multiaddrs: Vec<Multiaddr>,
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
#[derive(Debug)]
pub enum CreateClusterOutcome {
    /// `201 Created` — the caller is the new cluster's initial Manager.
    Created(ClusterEntry),
    /// `409 Conflict` — the cluster name was already taken. Re-fetch
    /// the directory with [`DiscoveryClient::list_clusters`] and join
    /// the existing cluster.
    AlreadyExists,
}

/// Failure modes for [`DiscoveryClient`] calls.
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
#[derive(Debug, Clone)]
pub struct DiscoveryClient {
    base_url: String,
    http: reqwest::Client,
}

impl DiscoveryClient {
    /// Construct against `base_url` with a default `reqwest::Client`.
    /// Trailing `/` on `base_url` is stripped.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_http(base_url, reqwest::Client::new())
    }

    /// Construct against `base_url` with a caller-provided
    /// `reqwest::Client`. Use this to set custom timeouts, proxies,
    /// custom TLS roots, etc.
    pub fn with_http(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// The base URL this client was constructed against (trailing `/`
    /// stripped).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

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
        name: &str,
        manager_peer_id: &PeerId,
        manager_multiaddrs: &[Multiaddr],
    ) -> Result<CreateClusterOutcome, DiscoveryError> {
        let url = format!("{}/clusters/{}", self.base_url, name);
        let body = WireManagerRequest {
            manager_peer_id: manager_peer_id.to_string(),
            manager_multiaddrs: manager_multiaddrs.iter().map(|m| m.to_string()).collect(),
        };
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
        name: &str,
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
        name: &str,
        manager_peer_id: &PeerId,
        manager_multiaddrs: &[Multiaddr],
    ) -> Result<ClusterEntry, DiscoveryError> {
        let url = format!("{}/clusters/{}/manager", self.base_url, name);
        let body = WireManagerRequest {
            manager_peer_id: manager_peer_id.to_string(),
            manager_multiaddrs: manager_multiaddrs.iter().map(|m| m.to_string()).collect(),
        };
        let resp = self.http.post(&url).json(&body).send().await?;
        ok_or_status(resp).await
    }

    /// Graceful deregistration. Called by the last cluster member on
    /// clean exit. v1 = trust-the-claim; any HTTP caller succeeds.
    pub async fn deregister(&self, name: &str) -> Result<(), DiscoveryError> {
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

    // ── Infrastructure nodes (/nodes) ─────────────────────────────────────────

    /// List infrastructure nodes registered with Discovery.
    ///
    /// Pass `node_type = Some("relay")` to filter by type. `None` returns all
    /// node types. Results are sorted by `created_ns` descending.
    ///
    /// Nodes (relay servers, reconstruction servers, domain servers, ...) differ
    /// from clusters: they have no Manager, no membership document, and no join
    /// protocol — they are simply stable advertisements of a peer's public
    /// multiaddrs so ad-hoc peers can find and use them.
    pub async fn list_nodes(
        &self,
        node_type: Option<&str>,
    ) -> Result<Vec<NodeEntry>, DiscoveryError> {
        let url = match node_type {
            Some(t) => format!("{}/nodes?type={}", self.base_url, t),
            None => format!("{}/nodes", self.base_url),
        };
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(DiscoveryError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let wire: WireListNodesResponse = resp.json().await?;
        Ok(wire.nodes)
    }

    /// Register this peer as an infrastructure node with Discovery.
    ///
    /// Called at daemon boot. Returns 409 (mapped to `DiscoveryError::Status`)
    /// if the same `peer_id` is already registered — callers should
    /// `deregister_node` then retry, or log a warning and continue.
    pub async fn register_node(
        &self,
        peer_id: &PeerId,
        node_type: &str,
        multiaddrs: &[Multiaddr],
    ) -> Result<NodeEntry, DiscoveryError> {
        let url = format!("{}/nodes", self.base_url);
        let body = WireRegisterNodeRequest {
            peer_id: peer_id.to_string(),
            node_type: node_type.to_string(),
            multiaddrs: multiaddrs.iter().map(|a| a.to_string()).collect(),
        };
        let resp = self.http.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(DiscoveryError::Status {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.json::<NodeEntry>().await?)
    }

    /// Send a keep-alive heartbeat for an infrastructure node.
    ///
    /// Call every ~3 seconds. Discovery sweeps entries that have not
    /// heartbeated in 10 seconds.
    pub async fn heartbeat_node(&self, peer_id: &PeerId) -> Result<(), DiscoveryError> {
        let url = format!("{}/nodes/{}/heartbeat", self.base_url, peer_id);
        let resp = self.http.post(&url).send().await?;
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

    /// Deregister an infrastructure node from Discovery.
    ///
    /// Call on clean shutdown. Discovery will sweep the entry after ~10 seconds
    /// of missed heartbeats regardless, but calling this accelerates cleanup.
    pub async fn deregister_node(&self, peer_id: &PeerId) -> Result<(), DiscoveryError> {
        let url = format!("{}/nodes/{}", self.base_url, peer_id);
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
}

#[derive(Debug, Serialize)]
struct WireLivenessCheckRequest {
    peer_count: u32,
}

// ── /nodes wire shapes ────────────────────────────────────────────────────────

/// An infrastructure node registered with Discovery.
///
/// Unlike clusters (which have a Manager + membership document), nodes are
/// stateless advertisements — `peer_id + node_type + multiaddrs`. Used by
/// relay servers, reconstruction servers, domain servers, and any other
/// stable infrastructure that ad-hoc peers need to find at boot time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEntry {
    pub peer_id: String,
    pub node_type: String,
    pub multiaddrs: Vec<String>,
    pub created_ns: i64,
    pub last_heartbeat_ns: i64,
}

#[derive(Debug, Deserialize)]
struct WireListNodesResponse {
    nodes: Vec<NodeEntry>,
}

#[derive(Debug, Serialize)]
struct WireRegisterNodeRequest {
    peer_id: String,
    node_type: String,
    multiaddrs: Vec<String>,
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
    Ok(ClusterEntry {
        name: w.name,
        manager_peer_id,
        manager_multiaddrs,
        peer_count: w.peer_count,
        created_ns: w.created_ns,
        last_liveness_check_ns: w.last_liveness_check_ns,
    })
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
        WireClusterEntry {
            name: "foo".to_string(),
            manager_peer_id: "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw".to_string(),
            manager_multiaddrs: vec!["/ip4/192.168.9.10/tcp/4001".to_string()],
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
    fn client_trims_trailing_slash_from_base_url() {
        let a = DiscoveryClient::new("http://example.com:8080");
        let b = DiscoveryClient::new("http://example.com:8080/");
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
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"manager_peer_id\":"), "{json}");
        assert!(json.contains("\"manager_multiaddrs\":"), "{json}");
    }

    /// Same for the liveness-check body.
    #[test]
    fn wire_liveness_check_request_field_name_is_locked() {
        let req = WireLivenessCheckRequest { peer_count: 7 };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"peer_count":7}"#);
    }
}
