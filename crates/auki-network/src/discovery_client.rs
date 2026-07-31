//! HTTP client for the Discovery service.
//!
//! Discovery is a bootstrap rendezvous directory: peers consult it on
//! first boot to resolve a domain's current Manager, then operate
//! peer-to-peer over libp2p from there. Discovery's record is a hint
//! — the cluster owns itself; Discovery's entry catches up on the next
//! Manager registration. What Discovery stores per domain is
//! deliberately minimal (current Manager's libp2p hint + aggregate
//! `peer_count` + timestamps); the peer list lives on the Manager and
//! is gossiped peer-to-peer, never on Discovery.
//!
//! ## Endpoints
//!
//! - `GET    /api/v1/domains/{id}/peer-manager` — resolve Manager hint
//!   for one domain (no global list).
//! - `PUT    /api/v1/domains/{id}/peer-manager` — upsert register /
//!   reclaim. When Discovery has `DDS_URL`, requires
//!   `Authorization: Bearer <domain-access-jwt>` with `domain_id`
//!   matching `{id}`; otherwise trust-on-first-claim.
//! - `POST   /api/v1/domains/{id}/peer-manager/heartbeat` — Manager
//!   liveness, pushed every 1s. Body `{ peer_count }`. Same auth rule
//!   as PUT.
//! - `DELETE /api/v1/domains/{id}/peer-manager` — graceful
//!   deregistration. Same auth rule as PUT.
//!
//! Attach Manager write credentials via
//! [`DiscoveryClient::with_authorization`].
//!
//! ## Wire format
//!
//! JSON throughout. Timestamps on the wire are RFC3339; the client
//! maps them to unix nanoseconds in [`ClusterEntry`]. `PeerId` rides
//! as its canonical libp2p string form (base58 / multihash);
//! `Multiaddr` rides as its canonical `/ip4/.../tcp/...` text form.
//! The client parses these at the boundary so consumers see typed
//! values. When a Manager address includes `/p2p-circuit`, the client
//! also publishes the relay base address as `relay_multiaddrs` so
//! browser peers can reserve and advertise through the same Domain
//! Relay.

use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Upper bound on any single Discovery HTTP round-trip. Discovery is
/// LAN-scale (its liveness sweep is 3s and Managers tick it at 1s);
/// a black-holed deployment must surface as a transport error within
/// the failover loop's cadence, not the OS TCP connect timeout.
/// Callers needing different policy supply their own client via
/// [`DiscoveryClient::with_http`].
pub const DISCOVERY_HTTP_TIMEOUT: Duration = Duration::from_secs(2);

// ─── Public types ──────────────────────────────────────────────────

/// One cluster's entry in Discovery's directory.
///
/// `manager_peer_id` + `manager_multiaddrs` are the hint a newcomer
/// uses to dial the current Manager over libp2p. `relay_multiaddrs`
/// are optional relay hints a browser can use when direct Manager
/// transport is not dialable. `peer_count` is the Manager's most-recent
/// self-reported size (aggregate; no identities). `created_ns` and
/// `last_liveness_check_ns` are unix nanoseconds mapped from the wire
/// `registered_at` / `last_seen` fields.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEntry {
    /// The cluster / domain name.
    pub name: String,
    /// Current Manager's libp2p peer-id.
    pub manager_peer_id: PeerId,
    /// Current Manager's dialable libp2p multiaddrs.
    pub manager_multiaddrs: Vec<Multiaddr>,
    /// Relay base multiaddrs associated with any relay-mediated Manager
    /// addresses, plus optional relay hints for browser and
    /// constrained-network peers.
    pub relay_multiaddrs: Vec<Multiaddr>,
    /// Manager's most-recent self-reported peer count (aggregate).
    pub peer_count: u32,
    /// Unix nanoseconds, Discovery's server clock at first registration.
    pub created_ns: i64,
    /// Unix nanoseconds, last heartbeat received.
    pub last_liveness_check_ns: i64,
}

/// Outcome of [`DiscoveryClient::create_cluster`] (compat alias for
/// [`DiscoveryClient::put_peer_manager`]).
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateClusterOutcome {
    /// Upsert succeeded — the caller's hint is now authoritative.
    Created(ClusterEntry),
    /// Legacy variant retained for API compatibility. The peer-manager
    /// directory upserts and never returns 409.
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
    /// Discovery returned a non-success status code.
    #[error("Discovery returned HTTP {status}: {body}")]
    Status {
        /// HTTP status code Discovery returned.
        status: u16,
        /// Response body as text (typically `{ "error": "…" }`).
        body: String,
    },
    /// Discovery's response carried a `peer_id` that didn't parse as a
    /// libp2p `PeerId`.
    #[error("invalid peer-id in Discovery response: {0}")]
    InvalidPeerId(String),
    /// Discovery's response carried a `multiaddrs` entry that didn't
    /// parse as a libp2p `Multiaddr`.
    #[error("invalid multiaddr in Discovery response: {0}")]
    InvalidMultiaddr(String),
    /// Discovery's response carried a timestamp that didn't parse.
    #[error("invalid timestamp in Discovery response: {0}")]
    InvalidTimestamp(String),
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
    /// Optional `Authorization` header for Manager write routes
    /// (`Bearer <jwt>`). `GET …/peer-manager` ignores this.
    authorization: Option<String>,
}

// ─── Sync methods (exported as the primary constructor + accessor) ──

#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl DiscoveryClient {
    /// Construct against `base_url` with a default `reqwest::Client`.
    /// Trailing `/` on `base_url` is stripped.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new(base_url: String) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(DISCOVERY_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client with static config");
        Arc::new(Self::with_http(base_url, http))
    }

    /// The base URL this client was constructed against (trailing `/`
    /// stripped).
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }
}

impl DiscoveryClient {
    /// Construct against `base_url` with a caller-provided
    /// `reqwest::Client`.
    pub fn with_http(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            authorization: None,
        }
    }

    /// Attach `Authorization` for PUT / heartbeat / delete.
    pub fn with_authorization(mut self, authorization: impl Into<String>) -> Self {
        self.authorization = normalize_authorization(authorization.into());
        self
    }

    /// Construct with default HTTP client + optional write auth.
    pub fn new_with_authorization(
        base_url: String,
        authorization: impl Into<String>,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(DISCOVERY_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client with static config");
        Arc::new(Self::with_http(base_url, http).with_authorization(authorization))
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.authorization {
            Some(value) => req.header(reqwest::header::AUTHORIZATION, value.as_str()),
            None => req,
        }
    }
}

fn normalize_authorization(authorization: String) -> Option<String> {
    let s = authorization.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((scheme, rest)) = s.split_once(char::is_whitespace) {
        if scheme.eq_ignore_ascii_case("bearer") {
            let token = rest.trim();
            return if token.is_empty() {
                None
            } else {
                Some(format!("Bearer {token}"))
            };
        }
    }
    Some(format!("Bearer {s}"))
}

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b'/')
    .add(b'?')
    .add(b'#')
    .add(b'[')
    .add(b']')
    .add(b'@')
    .add(b'!')
    .add(b'$')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b';')
    .add(b'=');

fn peer_manager_url(base_url: &str, domain_id: &str) -> String {
    let encoded = utf8_percent_encode(domain_id, PATH_SEGMENT_ENCODE_SET);
    format!("{base_url}/api/v1/domains/{encoded}/peer-manager")
}

fn peer_manager_heartbeat_url(base_url: &str, domain_id: &str) -> String {
    format!("{}/heartbeat", peer_manager_url(base_url, domain_id))
}

// ─── Async methods (exported with tokio runtime) ────────────────────

#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl DiscoveryClient {
    /// Upsert the peer-manager hint for `domain_id`.
    pub async fn put_peer_manager(
        &self,
        domain_id: String,
        peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        self.put_peer_manager_with_relay_multiaddrs(domain_id, peer_id, multiaddrs, Vec::new())
            .await
    }

    /// Upsert the peer-manager hint with explicit relay multiaddrs.
    pub async fn put_peer_manager_with_relay_multiaddrs(
        &self,
        domain_id: String,
        peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        let url = peer_manager_url(&self.base_url, &domain_id);
        let body = WirePeerManagerPutRequest::from_multiaddrs_and_relay_multiaddrs(
            peer_id,
            &multiaddrs,
            &relay_multiaddrs,
        );
        let resp = self
            .apply_auth(self.http.put(&url).json(&body))
            .send()
            .await?;
        ok_or_status(resp).await
    }

    /// Resolve the peer-manager hint for one domain.
    pub async fn get_peer_manager(&self, domain_id: String) -> Result<ClusterEntry, DiscoveryError> {
        let url = peer_manager_url(&self.base_url, &domain_id);
        let resp = self.http.get(&url).send().await?;
        ok_or_status(resp).await
    }

    /// Manager push: report aggregate `peer_count`. Discovery refreshes
    /// `last_seen` and resets the 3s sweep window. Returns the updated
    /// [`ClusterEntry`].
    pub async fn heartbeat(
        &self,
        domain_id: String,
        peer_count: u32,
    ) -> Result<ClusterEntry, DiscoveryError> {
        let url = peer_manager_heartbeat_url(&self.base_url, &domain_id);
        let body = WireHeartbeatRequest { peer_count };
        let resp = self
            .apply_auth(self.http.post(&url).json(&body))
            .send()
            .await?;
        ok_or_status(resp).await
    }

    /// Graceful deregistration for one domain.
    pub async fn deregister(&self, domain_id: String) -> Result<(), DiscoveryError> {
        let url = peer_manager_url(&self.base_url, &domain_id);
        let resp = self.apply_auth(self.http.delete(&url)).send().await?;
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

    /// Compat alias — always returns [`CreateClusterOutcome::Created`].
    pub async fn create_cluster(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
    ) -> Result<CreateClusterOutcome, DiscoveryError> {
        let entry = self
            .put_peer_manager(name, manager_peer_id, manager_multiaddrs)
            .await?;
        Ok(CreateClusterOutcome::Created(entry))
    }

    /// Compat alias — always returns [`CreateClusterOutcome::Created`].
    pub async fn create_cluster_with_relay_multiaddrs(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
    ) -> Result<CreateClusterOutcome, DiscoveryError> {
        let entry = self
            .put_peer_manager_with_relay_multiaddrs(
                name,
                manager_peer_id,
                manager_multiaddrs,
                relay_multiaddrs,
            )
            .await?;
        Ok(CreateClusterOutcome::Created(entry))
    }

    /// Compat alias for [`Self::heartbeat`].
    pub async fn liveness_check(
        &self,
        name: String,
        peer_count: u32,
    ) -> Result<ClusterEntry, DiscoveryError> {
        self.heartbeat(name, peer_count).await
    }

    /// Compat alias for [`Self::put_peer_manager`].
    pub async fn rotate_manager(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        self.put_peer_manager(name, manager_peer_id, manager_multiaddrs)
            .await
    }

    /// Compat alias for [`Self::put_peer_manager_with_relay_multiaddrs`].
    pub async fn rotate_manager_with_relay_multiaddrs(
        &self,
        name: String,
        manager_peer_id: PeerId,
        manager_multiaddrs: Vec<Multiaddr>,
        relay_multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        self.put_peer_manager_with_relay_multiaddrs(
            name,
            manager_peer_id,
            manager_multiaddrs,
            relay_multiaddrs,
        )
        .await
    }
}

// ─── Wire shapes (internal) ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WirePeerManagerEntry {
    domain_id: String,
    peer_id: String,
    multiaddrs: Vec<String>,
    #[serde(default)]
    relay_multiaddrs: Vec<String>,
    peer_count: u32,
    registered_at: String,
    last_seen: String,
}

#[derive(Debug, Serialize)]
struct WirePeerManagerPutRequest {
    peer_id: String,
    multiaddrs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    relay_multiaddrs: Vec<String>,
}

impl WirePeerManagerPutRequest {
    fn from_multiaddrs_and_relay_multiaddrs(
        peer_id: PeerId,
        multiaddrs: &[Multiaddr],
        relay_multiaddrs: &[Multiaddr],
    ) -> Self {
        let mut relay_multiaddrs = relay_multiaddrs
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>();
        for relay in relay_multiaddrs_from_manager_multiaddrs(multiaddrs) {
            if !relay_multiaddrs.contains(&relay) {
                relay_multiaddrs.push(relay);
            }
        }
        Self {
            peer_id: peer_id.to_string(),
            multiaddrs: multiaddrs.iter().map(|m| m.to_string()).collect(),
            relay_multiaddrs,
        }
    }
}

#[derive(Debug, Serialize)]
struct WireHeartbeatRequest {
    peer_count: u32,
}

fn parse_wire_entry(w: WirePeerManagerEntry) -> Result<ClusterEntry, DiscoveryError> {
    let manager_peer_id = PeerId::from_str(&w.peer_id)
        .map_err(|e| DiscoveryError::InvalidPeerId(format!("{}: {}", w.peer_id, e)))?;
    let manager_multiaddrs = w
        .multiaddrs
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
        name: w.domain_id,
        manager_peer_id,
        manager_multiaddrs,
        relay_multiaddrs,
        peer_count: w.peer_count,
        created_ns: parse_wire_timestamp_to_ns(&w.registered_at)?,
        last_liveness_check_ns: parse_wire_timestamp_to_ns(&w.last_seen)?,
    })
}

fn parse_wire_timestamp_to_ns(s: &str) -> Result<i64, DiscoveryError> {
    if let Ok(ns) = s.parse::<i64>() {
        return Ok(ns);
    }
    parse_rfc3339_to_ns(s)
}

fn parse_rfc3339_to_ns(s: &str) -> Result<i64, DiscoveryError> {
    let s = s.trim();
    if !s.ends_with('Z') {
        return Err(DiscoveryError::InvalidTimestamp(s.to_string()));
    }
    let s = &s[..s.len() - 1];
    let (date, time) = s.split_once('T').ok_or_else(|| {
        DiscoveryError::InvalidTimestamp(format!("expected RFC3339 date-time, got {s:?}"))
    })?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(date.to_string()))?;
    let month: i64 = date_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(date.to_string()))?;
    let day: i64 = date_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(date.to_string()))?;

    let (time_main, frac_ns) = if let Some((main, frac)) = time.split_once('.') {
        let frac_digits = frac.len().min(9);
        let mut frac_str = frac[..frac_digits].to_string();
        while frac_str.len() < 9 {
            frac_str.push('0');
        }
        let frac_ns: i64 = frac_str
            .parse()
            .map_err(|_| DiscoveryError::InvalidTimestamp(time.to_string()))?;
        (main, frac_ns)
    } else {
        (time, 0)
    };

    let mut time_parts = time_main.split(':');
    let hour: i64 = time_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(time.to_string()))?;
    let minute: i64 = time_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(time.to_string()))?;
    let second: i64 = time_parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| DiscoveryError::InvalidTimestamp(time.to_string()))?;

    let days = unix_days_from_civil(year, month, day)?;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Ok(secs * 1_000_000_000 + frac_ns)
}

fn unix_days_from_civil(year: i64, month: i64, day: i64) -> Result<i64, DiscoveryError> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(DiscoveryError::InvalidTimestamp(format!(
            "{year:04}-{month:02}-{day:02}"
        )));
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as i64;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
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
    let wire: WirePeerManagerEntry = resp.json().await?;
    parse_wire_entry(wire)
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire_entry() -> WirePeerManagerEntry {
        let relay_peer_id = crate::PeerIdentity::from_seed(&[201u8; 32]).peer_id();
        WirePeerManagerEntry {
            domain_id: "foo".to_string(),
            peer_id: "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw".to_string(),
            multiaddrs: vec!["/ip4/192.168.9.10/tcp/4001".to_string()],
            relay_multiaddrs: vec![format!("/ip4/192.168.9.10/tcp/4002/ws/p2p/{relay_peer_id}")],
            peer_count: 3,
            registered_at: "2026-07-31T04:00:00Z".to_string(),
            last_seen: "2026-07-31T04:00:01Z".to_string(),
        }
    }

    #[test]
    fn parse_wire_entry_round_trips_via_typed_form() {
        let w = sample_wire_entry();
        let entry = parse_wire_entry(w.clone()).expect("valid wire entry parses");
        assert_eq!(entry.name, w.domain_id);
        assert_eq!(entry.manager_peer_id.to_string(), w.peer_id);
        assert_eq!(
            entry
                .manager_multiaddrs
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>(),
            w.multiaddrs
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
        assert!(entry.created_ns > 0);
        assert!(entry.last_liveness_check_ns > entry.created_ns);
    }

    #[test]
    fn parse_wire_entry_preserves_relay_multiaddrs() {
        let json = r#"{
            "domain_id": "foo",
            "peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
            "multiaddrs": ["/ip4/192.168.9.10/tcp/4001"],
            "relay_multiaddrs": ["/ip4/192.168.9.20/tcp/4002/ws/p2p/12D3KooWESKUn3Fh3xTMq1KzoxbQQ6PypHodP1JAb4p7qkxJxJ7n"],
            "peer_count": 3,
            "registered_at": "2026-07-31T04:00:00Z",
            "last_seen": "2026-07-31T04:00:01Z"
        }"#;
        let wire: WirePeerManagerEntry = serde_json::from_str(json).unwrap();
        let entry = parse_wire_entry(wire).expect("valid wire entry parses");

        assert_eq!(
            entry
                .relay_multiaddrs
                .iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>(),
            vec![
                "/ip4/192.168.9.20/tcp/4002/ws/p2p/12D3KooWESKUn3Fh3xTMq1KzoxbQQ6PypHodP1JAb4p7qkxJxJ7n"
            ]
        );
    }

    #[test]
    fn parse_wire_entry_defaults_missing_relay_multiaddrs_to_empty() {
        let json = r#"{
            "domain_id": "foo",
            "peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
            "multiaddrs": ["/ip4/192.168.9.10/tcp/4001"],
            "peer_count": 3,
            "registered_at": "2026-07-31T04:00:00Z",
            "last_seen": "2026-07-31T04:00:01Z"
        }"#;
        let wire: WirePeerManagerEntry = serde_json::from_str(json).unwrap();
        let entry = parse_wire_entry(wire).expect("legacy wire entry parses");

        assert!(entry.relay_multiaddrs.is_empty());
    }

    #[test]
    fn parse_wire_entry_rejects_garbage_peer_id() {
        let mut w = sample_wire_entry();
        w.peer_id = "not-a-peer-id".to_string();
        let err = parse_wire_entry(w).expect_err("garbage peer-id is rejected");
        assert!(
            matches!(err, DiscoveryError::InvalidPeerId(_)),
            "expected InvalidPeerId, got {err:?}"
        );
    }

    #[test]
    fn parse_wire_entry_rejects_garbage_multiaddr() {
        let mut w = sample_wire_entry();
        w.multiaddrs = vec!["not-a-multiaddr".to_string()];
        let err = parse_wire_entry(w).expect_err("garbage multiaddr is rejected");
        assert!(
            matches!(err, DiscoveryError::InvalidMultiaddr(_)),
            "expected InvalidMultiaddr, got {err:?}"
        );
    }

    #[test]
    fn parse_wire_entry_accepts_legacy_ns_timestamps() {
        let json = serde_json::json!({
            "domain_id": "foo",
            "peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
            "multiaddrs": ["/ip4/192.168.9.10/tcp/4001"],
            "peer_count": 3,
            "registered_at": "1715423400000000000",
            "last_seen": "1715423500000000000",
        });
        let wire: WirePeerManagerEntry = serde_json::from_value(json).unwrap();
        let entry = parse_wire_entry(wire).unwrap();

        assert_eq!(entry.created_ns, 1_715_423_400_000_000_000);
        assert_eq!(entry.last_liveness_check_ns, 1_715_423_500_000_000_000);
    }

    #[test]
    fn client_trims_trailing_slash_from_base_url() {
        let a = DiscoveryClient::new("http://example.com:8080".to_string());
        let b = DiscoveryClient::new("http://example.com:8080/".to_string());
        assert_eq!(a.base_url(), b.base_url());
        assert_eq!(a.base_url(), "http://example.com:8080");
    }

    #[test]
    fn with_authorization_normalizes_bearer() {
        let c = DiscoveryClient::with_http(
            "http://example.com",
            reqwest::Client::new(),
        )
        .with_authorization("a.b.c");
        assert_eq!(c.authorization.as_deref(), Some("Bearer a.b.c"));
        let c2 = DiscoveryClient::with_http(
            "http://example.com",
            reqwest::Client::new(),
        )
        .with_authorization("Bearer tok");
        assert_eq!(c2.authorization.as_deref(), Some("Bearer tok"));
        let c3 = DiscoveryClient::with_http(
            "http://example.com",
            reqwest::Client::new(),
        )
        .with_authorization("  ");
        assert!(c3.authorization.is_none());
    }

    #[test]
    fn wire_entry_field_names_are_locked() {
        let json = serde_json::to_string(&sample_wire_entry()).unwrap();
        for key in [
            "\"domain_id\":",
            "\"peer_id\":",
            "\"multiaddrs\":",
            "\"relay_multiaddrs\":",
            "\"peer_count\":",
            "\"registered_at\":",
            "\"last_seen\":",
        ] {
            assert!(json.contains(key), "missing wire key {key:?} in {json}");
        }
    }

    #[test]
    fn wire_put_request_field_names_are_locked() {
        let req = WirePeerManagerPutRequest {
            peer_id: "12D3KooW…".to_string(),
            multiaddrs: vec!["/ip4/1.2.3.4/tcp/1".to_string()],
            relay_multiaddrs: Vec::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"peer_id\":"), "{json}");
        assert!(json.contains("\"multiaddrs\":"), "{json}");
        assert!(!json.contains("relay_multiaddrs"), "{json}");
    }

    #[test]
    fn wire_put_request_includes_relay_multiaddrs_when_present() {
        let req = WirePeerManagerPutRequest {
            peer_id: "12D3KooW…".to_string(),
            multiaddrs: vec!["/ip4/1.2.3.4/tcp/1".to_string()],
            relay_multiaddrs: vec![
                "/ip4/1.2.3.5/tcp/2/ws/p2p/12D3KooWESKUn3Fh3xTMq1KzoxbQQ6PypHodP1JAb4p7qkxJxJ7n"
                    .to_string(),
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"relay_multiaddrs\":"), "{json}");
    }

    #[test]
    fn wire_put_request_derives_relay_base_from_circuit_manager_addr() {
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

        let req = WirePeerManagerPutRequest::from_multiaddrs_and_relay_multiaddrs(
            manager_peer_id,
            &[native, circuit],
            &[],
        );

        assert_eq!(
            req.relay_multiaddrs,
            vec![relay_base.to_string()],
            "Discovery should receive relay base addresses separately from Manager circuit addresses"
        );
    }

    #[test]
    fn wire_put_request_combines_explicit_and_derived_relay_multiaddrs() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[206u8; 32]).peer_id();
        let relay_peer_id = crate::PeerIdentity::from_seed(&[207u8; 32]).peer_id();
        let explicit_relay: Multiaddr =
            "/ip4/192.168.9.131/tcp/4002/ws/p2p/12D3KooWESKUn3Fh3xTMq1KzoxbQQ6PypHodP1JAb4p7qkxJxJ7n"
                .parse()
                .unwrap();
        let derived_relay: Multiaddr =
            format!("/ip4/192.168.9.130/tcp/4002/ws/p2p/{relay_peer_id}")
                .parse()
                .unwrap();
        let circuit = derived_relay
            .clone()
            .with(multiaddr::Protocol::P2pCircuit)
            .with(multiaddr::Protocol::P2p(manager_peer_id));

        let req = WirePeerManagerPutRequest::from_multiaddrs_and_relay_multiaddrs(
            manager_peer_id,
            &[circuit],
            &[explicit_relay.clone()],
        );

        assert_eq!(
            req.relay_multiaddrs,
            vec![explicit_relay.to_string(), derived_relay.to_string()]
        );
    }

    #[test]
    fn wire_put_request_omits_relay_multiaddrs_without_circuit_addr() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[204u8; 32]).peer_id();
        let native: Multiaddr = "/ip4/192.168.9.130/tcp/4001".parse().unwrap();

        let req =
            WirePeerManagerPutRequest::from_multiaddrs_and_relay_multiaddrs(manager_peer_id, &[native], &[]);
        let json = serde_json::to_string(&req).unwrap();

        assert!(!json.contains("relay_multiaddrs"), "{json}");
    }

    #[test]
    fn wire_put_request_omits_relay_multiaddrs_when_circuit_lacks_relay_peer_id() {
        let manager_peer_id = crate::PeerIdentity::from_seed(&[205u8; 32]).peer_id();
        let malformed_circuit: Multiaddr =
            format!("/ip4/192.168.9.130/tcp/4002/ws/p2p-circuit/p2p/{manager_peer_id}")
                .parse()
                .unwrap();

        let req = WirePeerManagerPutRequest::from_multiaddrs_and_relay_multiaddrs(
            manager_peer_id,
            &[malformed_circuit],
            &[],
        );
        let json = serde_json::to_string(&req).unwrap();

        assert!(!json.contains("relay_multiaddrs"), "{json}");
    }

    #[test]
    fn wire_heartbeat_request_field_name_is_locked() {
        let req = WireHeartbeatRequest { peer_count: 7 };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"peer_count":7}"#);
    }

    #[test]
    fn peer_manager_url_percent_encodes_slashes() {
        let url = peer_manager_url("http://example.com", "wallet/cluster");
        assert_eq!(
            url,
            "http://example.com/api/v1/domains/wallet%2Fcluster/peer-manager"
        );
    }

    #[test]
    fn discovery_http_timeout_is_2s() {
        assert_eq!(DISCOVERY_HTTP_TIMEOUT, Duration::from_secs(2));
    }
}
