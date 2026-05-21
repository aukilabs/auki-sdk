//! UniFFI Swift bindings for `auki-network`.
//!
//! ## Scope (Stage 1)
//!
//! This crate mirrors the **root Discovery surface** of
//! [`auki-network-py`](../../../python/auki-network-py): `DiscoveryClient` plus the
//! `ClusterEntry` / `CreateClusterOutcome` value types. The stream
//! (audio) surface is Stage 2 and cluster lifecycle / peer enumeration
//! is a future `auki-domain-swift`, exactly mirroring the
//! `auki-network` / `auki-domain` split the Python bindings already
//! follow (see `src/sprint.md`).
//!
//! ## API shape
//!
//! Unlike `auki-network-py` (which is deliberately sync-shaped because
//! Python callers live in a GIL world), this crate exports **async**
//! methods. Swift consumers get real `async`/`await`, and on iOS the
//! calling thread (often the main thread) must never block on network
//! I/O. UniFFI drives the exported futures on a tokio runtime via
//! `#[uniffi::export(async_runtime = "tokio")]`; `reqwest` (the
//! Discovery HTTP transport) gets its reactor that way. This
//! async-vs-sync divergence from the `-py` precedent is intentional but
//! flagged for human confirmation in `parking_lot.md`.
//!
//! `PeerId` and `Multiaddr` cross the FFI as their canonical string
//! forms; the binding parses/validates at the seam so Swift never sees
//! a libp2p type. Errors are flattened into [`DiscoveryError`].

use auki_network_rs::discovery_client::{
    ClusterEntry as RustClusterEntry, CreateClusterOutcome as RustCreateClusterOutcome,
    DiscoveryClient as RustDiscoveryClient, DiscoveryError as RustDiscoveryError,
};
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::str::FromStr;
use std::sync::Arc;

uniffi::setup_scaffolding!();

// ─── Custom-type registrations ─────────────────────────────────────
//
// `PeerId` and `Multiaddr` are libp2p types defined in external crates
// (`libp2p-identity`, `multiaddr`); we can't annotate them directly. UniFFI's
// `custom_type!` with the `remote` keyword registers the conversion at the
// binding-crate level, anchored on this crate's `UniFfiTag` to satisfy the
// orphan rule. Every upstream `auki-network` method that takes or returns
// `PeerId` / `Multiaddr` is auto-exposed with `String` at the seam.
//
// `auki-domain-swift` (PR C) will pick up these registrations via its dep on
// this crate — no need to redeclare there.

// Cross-FFI representation: canonical libp2p peer-id string (`12D3KooW…`).
// Parse failures surface as a Rust `anyhow::Error` — UniFFI propagates the
// message to Swift as a thrown error on the affected method.
//
// The `remote` keyword is critical. Without it, the macro emits
// `impl<UT> FfiConverter<UT> for PeerId` which fails the orphan rule
// (PeerId and FfiConverter are both foreign). With `remote`, the impl
// becomes `impl FfiConverter<crate::UniFfiTag> for PeerId` — the
// binding crate's local UniFfiTag is the anchor.
uniffi::custom_type!(PeerId, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});

// ─── Value types ───────────────────────────────────────────────────

/// One cluster's entry in Discovery's directory. `manager_peer_id` is
/// the canonical libp2p peer-id string; `manager_multiaddrs` are
/// canonical `/ip4/.../tcp/...` strings.
///
/// `PartialEq`/`Eq` are derived for testing convenience and are
/// **string-byte equality across every field**. They are *not* a
/// reliable "same cluster" check — multiaddrs can differ in whitespace,
/// port-zero, percent-encoding, or protocol-stack order while still
/// denoting the same endpoint. For semantic equality compare `name`
/// (Discovery enforces a normalized character set) or `manager_peer_id`
/// (the libp2p peer-id is round-trip canonical).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct ClusterEntry {
    pub name: String,
    pub manager_peer_id: String,
    pub manager_multiaddrs: Vec<String>,
    pub peer_count: u32,
    pub created_ns: i64,
    pub last_liveness_check_ns: i64,
}

impl From<RustClusterEntry> for ClusterEntry {
    fn from(e: RustClusterEntry) -> Self {
        Self {
            name: e.name,
            manager_peer_id: e.manager_peer_id.to_string(),
            manager_multiaddrs: e.manager_multiaddrs.iter().map(|m| m.to_string()).collect(),
            peer_count: e.peer_count,
            created_ns: e.created_ns,
            last_liveness_check_ns: e.last_liveness_check_ns,
        }
    }
}

/// Outcome of [`DiscoveryClient::create_cluster`]. `already_exists` is
/// `true` when the name was taken (HTTP 409) — the caller should list
/// and join the existing cluster. `entry` is populated only when the
/// caller won the create race.
///
/// `PartialEq`/`Eq` inherit string-byte equality from [`ClusterEntry`];
/// see that type's docs for the equality caveat.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct CreateClusterOutcome {
    pub already_exists: bool,
    pub entry: Option<ClusterEntry>,
}

// ─── Error mapping ─────────────────────────────────────────────────

/// Failure modes for Discovery calls. Flattened 1:1 from
/// `auki_network::discovery_client::DiscoveryError` so Swift can branch
/// on the case without seeing reqwest/libp2p types.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("Discovery transport: {message}")]
    Transport { message: String },
    #[error("Discovery HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("invalid peer-id: {value}")]
    InvalidPeerId { value: String },
    #[error("invalid multiaddr: {value}")]
    InvalidMultiaddr { value: String },
}

impl From<RustDiscoveryError> for DiscoveryError {
    fn from(e: RustDiscoveryError) -> Self {
        match e {
            RustDiscoveryError::Transport(err) => DiscoveryError::Transport {
                message: err.to_string(),
            },
            RustDiscoveryError::Status { status, body } => DiscoveryError::Status { status, body },
            RustDiscoveryError::InvalidPeerId(s) => DiscoveryError::InvalidPeerId { value: s },
            RustDiscoveryError::InvalidMultiaddr(s) => {
                DiscoveryError::InvalidMultiaddr { value: s }
            }
        }
    }
}

fn parse_peer_id(s: &str) -> Result<PeerId, DiscoveryError> {
    PeerId::from_str(s).map_err(|e| DiscoveryError::InvalidPeerId {
        value: format!("{s:?}: {e}"),
    })
}

fn parse_multiaddrs(ss: &[String]) -> Result<Vec<Multiaddr>, DiscoveryError> {
    ss.iter()
        .map(|s| {
            Multiaddr::from_str(s).map_err(|e| DiscoveryError::InvalidMultiaddr {
                value: format!("{s:?}: {e}"),
            })
        })
        .collect()
}

// ─── DiscoveryClient ───────────────────────────────────────────────

/// HTTP client for a Discovery service instance.
#[derive(uniffi::Object)]
pub struct DiscoveryClient {
    inner: RustDiscoveryClient,
}

#[uniffi::export]
impl DiscoveryClient {
    /// Construct against `base_url`, e.g. `"http://192.168.9.130:8080"`.
    /// Trailing `/` is stripped.
    #[uniffi::constructor]
    pub fn new(base_url: String) -> Arc<Self> {
        Arc::new(Self {
            inner: RustDiscoveryClient::new(base_url),
        })
    }

    /// The base URL this client was constructed against.
    pub fn base_url(&self) -> String {
        self.inner.base_url().to_string()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl DiscoveryClient {
    /// Snapshot of Discovery's directory, sorted by `created_ns` desc.
    pub async fn list_clusters(&self) -> Result<Vec<ClusterEntry>, DiscoveryError> {
        Ok(self
            .inner
            .list_clusters()
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Atomically create a cluster; the caller becomes its initial
    /// Manager. A taken name yields `already_exists = true` (not an
    /// error).
    pub async fn create_cluster(
        &self,
        name: String,
        manager_peer_id: String,
        manager_multiaddrs: Vec<String>,
    ) -> Result<CreateClusterOutcome, DiscoveryError> {
        let peer_id = parse_peer_id(&manager_peer_id)?;
        let addrs = parse_multiaddrs(&manager_multiaddrs)?;
        Ok(
            match self.inner.create_cluster(&name, &peer_id, &addrs).await? {
                RustCreateClusterOutcome::Created(e) => CreateClusterOutcome {
                    already_exists: false,
                    entry: Some(e.into()),
                },
                RustCreateClusterOutcome::AlreadyExists => CreateClusterOutcome {
                    already_exists: true,
                    entry: None,
                },
            },
        )
    }

    /// Manager liveness push; resets Discovery's sweep window. Returns
    /// the updated entry.
    pub async fn liveness_check(
        &self,
        name: String,
        peer_count: u32,
    ) -> Result<ClusterEntry, DiscoveryError> {
        Ok(self.inner.liveness_check(&name, peer_count).await?.into())
    }

    /// Publish a new Manager hint after a successor election.
    pub async fn rotate_manager(
        &self,
        name: String,
        manager_peer_id: String,
        manager_multiaddrs: Vec<String>,
    ) -> Result<ClusterEntry, DiscoveryError> {
        let peer_id = parse_peer_id(&manager_peer_id)?;
        let addrs = parse_multiaddrs(&manager_multiaddrs)?;
        Ok(self
            .inner
            .rotate_manager(&name, &peer_id, &addrs)
            .await?
            .into())
    }

    /// Graceful deregistration.
    pub async fn deregister(&self, name: String) -> Result<(), DiscoveryError> {
        self.inner.deregister(&name).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic PeerId from a fixed ed25519 key — avoids needing
    /// libp2p-identity's `rand` feature in the shipped lib.
    fn test_peer_id() -> PeerId {
        libp2p_identity::Keypair::ed25519_from_bytes([7u8; 32])
            .expect("valid ed25519 seed")
            .public()
            .to_peer_id()
    }

    #[test]
    fn maps_status_error() {
        let mapped: DiscoveryError = RustDiscoveryError::Status {
            status: 503,
            body: "down".into(),
        }
        .into();
        match mapped {
            DiscoveryError::Status { status, body } => {
                assert_eq!(status, 503);
                assert_eq!(body, "down");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn maps_invalid_peer_id_and_multiaddr() {
        let p: DiscoveryError = RustDiscoveryError::InvalidPeerId("x".into()).into();
        assert!(matches!(p, DiscoveryError::InvalidPeerId { .. }));
        let m: DiscoveryError = RustDiscoveryError::InvalidMultiaddr("y".into()).into();
        assert!(matches!(m, DiscoveryError::InvalidMultiaddr { .. }));
    }

    #[test]
    fn cluster_entry_conversion_stringifies_libp2p_types() {
        let pid = test_peer_id();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let rust = RustClusterEntry {
            name: "demo".into(),
            manager_peer_id: pid,
            manager_multiaddrs: vec![addr.clone()],
            peer_count: 3,
            created_ns: 111,
            last_liveness_check_ns: 222,
        };
        let ffi: ClusterEntry = rust.into();
        assert_eq!(ffi.name, "demo");
        assert_eq!(ffi.manager_peer_id, pid.to_string());
        assert_eq!(ffi.manager_multiaddrs, vec![addr.to_string()]);
        assert_eq!(ffi.peer_count, 3);
        assert_eq!(ffi.created_ns, 111);
        assert_eq!(ffi.last_liveness_check_ns, 222);
    }

    #[test]
    fn rejects_malformed_peer_id_and_multiaddr() {
        assert!(parse_peer_id("not-a-peer-id").is_err());
        assert!(parse_multiaddrs(&["definitely/not/an/addr".to_string()]).is_err());
        // A well-formed peer-id round-trips.
        let pid = test_peer_id().to_string();
        assert_eq!(parse_peer_id(&pid).unwrap().to_string(), pid);
    }

    /// `PeerId` round-trips through its UniFFI custom-type registration:
    /// canonical string in → `PeerId` → canonical string out (identical).
    #[test]
    fn peer_id_custom_type_round_trips() {
        let pid = test_peer_id();
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }
}
