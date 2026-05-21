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
use auki_network_rs::HeartbeatTimestampSource;
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

// Cross-FFI representation: canonical `/ip4/.../tcp/...` multiaddr
// string. Parse failures surface as Rust `anyhow::Error`. `remote`
// keyword for the same reason as the `PeerId` registration above.
uniffi::custom_type!(Multiaddr, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});

// ─── Liveness events ───────────────────────────────────────────────
//
// `auki_network::PeerLivenessEvent` has 5 variants; two carry rich
// `Heartbeat*Observation` structs that aren't useful at v0 (iosapp's
// proof-bar UI only needs connect/disconnect/heartbeat-stream-closed).
// Translate to a 3-variant Swift-facing enum here and skip the two
// heartbeat-detail variants in the binding adapter — Rust callers
// continue to see the full upstream enum.

/// Peer connection-level liveness events surfaced to Swift consumers.
/// Each variant carries the affected peer-id as a canonical string.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum SwiftPeerLivenessEvent {
    /// A known peer connected at the libp2p connection layer.
    Connected { peer_id: String },
    /// A known peer disconnected at the libp2p connection layer.
    Disconnected { peer_id: String },
    /// A heartbeat substream closed or could not be opened. Useful for
    /// observers that want to distinguish transport-level disconnects
    /// from heartbeat-protocol failures.
    HeartbeatStreamClosed { peer_id: String },
}

impl SwiftPeerLivenessEvent {
    /// Translate an upstream `PeerLivenessEvent` into the Swift-facing
    /// 3-variant subset. The two heartbeat-detail upstream variants
    /// (`HeartbeatReceived`, `HeartbeatNtpSampleObserved`) get folded
    /// into `HeartbeatStreamClosed` as a placeholder; production callers
    /// of this function should pre-filter via `is_v0_forwardable` so
    /// those variants never reach this function.
    pub fn from_upstream(e: &auki_network_rs::PeerLivenessEvent) -> Self {
        use auki_network_rs::PeerLivenessEvent;
        match e {
            PeerLivenessEvent::Connected { peer_id } => Self::Connected {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::Disconnected { peer_id } => Self::Disconnected {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::HeartbeatStreamClosed { peer_id } => Self::HeartbeatStreamClosed {
                peer_id: peer_id.to_string(),
            },
            PeerLivenessEvent::HeartbeatReceived { peer_id, .. }
            | PeerLivenessEvent::HeartbeatNtpSampleObserved { peer_id, .. } => {
                Self::HeartbeatStreamClosed {
                    peer_id: peer_id.to_string(),
                }
            }
        }
    }

    /// True for upstream variants that should be forwarded to Swift
    /// listeners at v0 (filters out the two heartbeat-detail variants).
    pub fn is_v0_forwardable(upstream: &auki_network_rs::PeerLivenessEvent) -> bool {
        use auki_network_rs::PeerLivenessEvent;
        matches!(
            upstream,
            PeerLivenessEvent::Connected { .. }
                | PeerLivenessEvent::Disconnected { .. }
                | PeerLivenessEvent::HeartbeatStreamClosed { .. }
        )
    }
}

// ─── Peer liveness listener (Swift callback interface) ─────────────

/// Swift consumers implement this trait to receive peer liveness
/// events from the network runtime. Wrapped in `Arc<dyn ...>` and
/// passed into [`spawn_for_swift`]; the runtime's drain task calls
/// `on_event` for each forwardable upstream event.
///
/// `Send + Sync` per UniFFI callback-interface contract — Swift
/// implementations must be safe to call from a Rust tokio worker
/// thread. Swift compiler enforces this when adopting the protocol.
#[uniffi::export(callback_interface)]
pub trait PeerLivenessListener: Send + Sync {
    /// Invoked once per peer liveness event observed by the runtime.
    /// The drain task in `spawn_for_swift` runs on a tokio worker; long
    /// blocking work here will stall delivery of subsequent events.
    fn on_event(&self, event: SwiftPeerLivenessEvent);
}

// ─── Heartbeat timestamp provider (Swift callback interface) ───────

/// Swift consumers implement this trait to supply the heartbeat-source
/// timestamp readings and clock identity the runtime needs. Wrapped in
/// `Arc<dyn ...>`; the adapter [`heartbeat_source_from_provider`]
/// converts it into the upstream `HeartbeatTimestampSource` shape.
///
/// `clock_id` and `clock_hash` are read once at runtime spawn (they're
/// stable for the lifetime of the runtime). `now_ns` is invoked on
/// every outbound heartbeat frame; `domain_clock_bytes` is invoked the
/// same way and returns the JSON-encoded
/// `auki_network::heartbeat_protocol::HeartbeatDomainClock` or `None`.
///
/// Note: `HeartbeatDomainClock` uses JSON encoding (the heartbeat wire
/// format is length-prefixed JSON), so `domain_clock_bytes` must
/// carry a valid JSON object matching that struct's serde shape, or
/// `None` to signal "no domain clock to advertise".
#[uniffi::export(callback_interface)]
pub trait HeartbeatTimestampProvider: Send + Sync {
    /// Clock Registry id for the heartbeat `sent_at_clock_ns` values.
    /// Read once at spawn.
    fn clock_id(&self) -> String;
    /// Content-addressed hash of `clock_id`'s Clock Registry entry.
    /// Read once at spawn.
    fn clock_hash(&self) -> String;
    /// Current reading of `clock_id` in nanoseconds. Called per
    /// outbound heartbeat frame; must be fast (<1 ms).
    fn now_ns(&self) -> i64;
    /// JSON-encoded `auki_network::heartbeat_protocol::HeartbeatDomainClock`
    /// describing the domain clock this peer is currently advertising,
    /// or `None`. Called per outbound heartbeat frame.
    fn domain_clock_bytes(&self) -> Option<Vec<u8>>;
}

/// Adapter: build an upstream `HeartbeatTimestampSource` from a Swift
/// `HeartbeatTimestampProvider`. The closures wrap the trait-object
/// method calls; `domain_clock_bytes` results are decoded as
/// JSON-encoded `HeartbeatDomainClock` values (decode failure → `None`,
/// treated as "no domain clock to advertise").
pub(crate) fn heartbeat_source_from_provider(
    provider: Arc<dyn HeartbeatTimestampProvider>,
) -> HeartbeatTimestampSource {
    let clock_id = provider.clock_id();
    let clock_hash = provider.clock_hash();
    let p_for_now = provider.clone();
    let p_for_dc = provider.clone();
    HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: Arc::new(move || p_for_now.now_ns()),
        domain_clock: Arc::new(move || {
            p_for_dc.domain_clock_bytes().and_then(|bytes| {
                serde_json::from_slice::<auki_network_rs::heartbeat_protocol::HeartbeatDomainClock>(
                    &bytes,
                )
                .ok()
            })
        }),
    }
}

// ─── spawn_for_swift orchestrator ──────────────────────────────────

/// Errors from [`spawn_for_swift`].
///
/// `swift-bindings`: UniFFI Error. Flattens swarm-build failures to a
/// `message: String` since the underlying types are libp2p-specific.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum SpawnSwiftError {
    /// `auki_network::swarm::build_swarm` failed (invalid listen
    /// multiaddr, transport bind failure, etc.).
    #[error("swarm build: {message}")]
    SwarmBuild { message: String },
    /// `NetworkRuntime::spawn` failed — currently only one variant
    /// (`NoTokioRuntime`), but propagated as a message string for
    /// consistency.
    #[error("runtime spawn: {message}")]
    RuntimeSpawn { message: String },
}

/// Swift entry point for spawning a `NetworkRuntime`. Builds the libp2p
/// swarm internally, wires the `PeerLivenessListener` to the
/// `PeerLivenessEvent` channel via a drain task, drops the other 8
/// receivers (cluster-orchestration concerns reach for them via
/// `auki-domain-swift::ClusterManager` in PR C).
///
/// At this task's checkpoint, the stream provider is hard-coded to
/// `decline_all_streams()` — every inbound stream request is declined.
/// Task 13 widens this signature to accept a Swift-implemented
/// `SwiftStreamProvider`.
///
/// ## UniFFI callback-interface note
///
/// UniFFI 0.31 `Lift`-implements `Box<dyn Trait>` for callback
/// interfaces — not `Arc<dyn Trait>`. The `Box` is immediately promoted
/// to `Arc` inside the function body so that `drain_liveness_events` can
/// hold a `'static` reference across the tokio task boundary.
/// `heartbeat_source_from_provider` likewise needs `Arc`, so both are
/// promoted at entry. This is an internal implementation detail — Swift
/// callers see their protocol conformances passed by value, same as any
/// other UniFFI callback interface.
#[uniffi::export(async_runtime = "tokio")]
pub async fn spawn_for_swift(
    identity: Arc<auki_network_rs::PeerIdentity>,
    listen_multiaddrs: Vec<Multiaddr>,
    allowed_peers: Vec<auki_network_rs::AllowedPeer>,
    peer_liveness_listener: Box<dyn PeerLivenessListener>,
    heartbeat_timestamps: Box<dyn HeartbeatTimestampProvider>,
) -> Result<Arc<auki_network_rs::NetworkRuntime>, SpawnSwiftError> {
    // Promote callback-interface boxes to Arc so they can be shared
    // across the tokio spawn boundary and the heartbeat adapter.
    let peer_liveness_listener: Arc<dyn PeerLivenessListener> =
        Arc::from(peer_liveness_listener);
    let heartbeat_timestamps: Arc<dyn HeartbeatTimestampProvider> =
        Arc::from(heartbeat_timestamps);

    // 1. Build the swarm. Upstream API: `build_swarm(&PeerIdentity, SwarmConfig)`.
    let swarm = auki_network_rs::swarm::build_swarm(
        identity.as_ref(),
        auki_network_rs::swarm::SwarmConfig {
            listen_addresses: listen_multiaddrs,
            agent_version: format!(
                "auki-network-swift/{}",
                env!("CARGO_PKG_VERSION")
            ),
            enable_relay_server: false,
        },
    )
    .map_err(|e| SpawnSwiftError::SwarmBuild {
        message: e.to_string(),
    })?;

    // 2. Build the heartbeat source from the Swift provider.
    let heartbeat_source = heartbeat_source_from_provider(heartbeat_timestamps);

    // 3. Install decline-all stream provider (Task 13 widens this).
    let stream_provider = auki_network_rs::stream_runtime::decline_all_streams();

    // 4. Spawn the runtime. The 9-element tuple destructure: (Self,
    //    join_rx, liveness_rx, membership_rx, info_rx, resources_rx,
    //    sensors_rx, registry_rx, diagnostic_rx). At v0 we only wire
    //    liveness_rx to the Swift listener; the others are dropped
    //    (their senders' errors are swallowed by run_task).
    let (rt, _join_rx, liveness_rx, _membership_rx, _info_rx, _resources_rx, _sensors_rx, _registry_rx, _diagnostic_rx) =
        auki_network_rs::NetworkRuntime::spawn(
            swarm,
            allowed_peers,
            stream_provider,
            heartbeat_source,
        )
        .map_err(|e| SpawnSwiftError::RuntimeSpawn {
            message: e.to_string(),
        })?;

    // 5. Drain task: pump liveness events to the Swift listener.
    tokio::spawn(drain_liveness_events(liveness_rx, peer_liveness_listener));

    Ok(Arc::new(rt))
}

/// Drains the upstream `PeerLivenessEvent` receiver, forwarding each
/// `is_v0_forwardable` event to the Swift `PeerLivenessListener`.
async fn drain_liveness_events(
    mut rx: tokio::sync::mpsc::Receiver<auki_network_rs::PeerLivenessEvent>,
    listener: Arc<dyn PeerLivenessListener>,
) {
    while let Some(event) = rx.recv().await {
        if SwiftPeerLivenessEvent::is_v0_forwardable(&event) {
            listener.on_event(SwiftPeerLivenessEvent::from_upstream(&event));
        }
        // Else drop the heartbeat-detail variants per v0 design.
    }
}

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

    /// `Multiaddr` round-trips through its UniFFI custom-type registration.
    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let s = addr.to_string();
        let back: Multiaddr = s.parse().expect("canonical multiaddr parses");
        assert_eq!(back, addr);
    }

    /// `AllowedPeer` is constructible from canonical PeerId + multiaddr
    /// strings via UniFFI's auto-derived constructor. Exercises the
    /// custom-type lowering chain (String → PeerId → Vec<Multiaddr>).
    #[test]
    fn allowed_peer_constructs_with_string_inputs() {
        let pid = test_peer_id();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let ap = auki_network_rs::AllowedPeer {
            peer_id: pid,
            multiaddrs: vec![addr.clone()],
        };
        assert_eq!(ap.peer_id, pid);
        assert_eq!(ap.multiaddrs, vec![addr]);
    }

    /// `SpawnError` is a Display'd error; UniFFI flattens it as a tagged enum.
    #[test]
    fn spawn_error_is_display_clean() {
        use auki_network_rs::SpawnError;
        let e = SpawnError::NoTokioRuntime;
        assert!(!e.to_string().is_empty());
    }

    /// `UpdateError` round-trips through Display; UniFFI surfaces it as a
    /// tagged enum (no opaque payloads after annotation).
    #[test]
    fn update_error_is_display_clean() {
        use auki_network_rs::UpdateError;
        let e = UpdateError::RuntimeUnavailable;
        assert!(!e.to_string().is_empty());
    }

    /// `SwiftPeerLivenessEvent::from_upstream` translates each upstream
    /// variant to the right Swift variant.
    #[test]
    fn swift_peer_liveness_event_translation() {
        use auki_network_rs::PeerLivenessEvent;

        let pid = test_peer_id();
        let connected = PeerLivenessEvent::Connected { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&connected);
        assert!(matches!(s, SwiftPeerLivenessEvent::Connected { .. }));

        let disconnected = PeerLivenessEvent::Disconnected { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&disconnected);
        assert!(matches!(s, SwiftPeerLivenessEvent::Disconnected { .. }));

        let heartbeat_closed = PeerLivenessEvent::HeartbeatStreamClosed { peer_id: pid };
        let s = SwiftPeerLivenessEvent::from_upstream(&heartbeat_closed);
        assert!(matches!(s, SwiftPeerLivenessEvent::HeartbeatStreamClosed { .. }));
    }

    /// Smoke test: a no-op `PeerLivenessListener` impl compiles and can be
    /// stored as `Arc<dyn PeerLivenessListener>`. Real wire-up tested in
    /// the Task 10 spawn_for_swift smoke test.
    #[test]
    fn peer_liveness_listener_is_object_safe() {
        struct NoopListener;
        impl PeerLivenessListener for NoopListener {
            fn on_event(&self, _event: SwiftPeerLivenessEvent) {}
        }
        let listener: Arc<dyn PeerLivenessListener> = Arc::new(NoopListener);
        // Use it once so the binding isn't dead code.
        listener.on_event(SwiftPeerLivenessEvent::HeartbeatStreamClosed {
            peer_id: "irrelevant".to_string(),
        });
    }

    /// `NetworkRuntime` exposes its annotated method set. We can't spawn one
    /// here (needs a real tokio runtime + swarm), but we can confirm the
    /// types compile.
    #[test]
    fn network_runtime_is_uniffi_object() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<auki_network_rs::NetworkRuntime>();
    }

    /// Smoke test: `spawn_for_swift` constructs a runtime against a no-op
    /// listener + a wall-clock heartbeat provider, then shuts it down
    /// cleanly. Requires a real tokio runtime.
    #[tokio::test]
    async fn spawn_for_swift_smoke() {
        struct NoopListener;
        impl PeerLivenessListener for NoopListener {
            fn on_event(&self, _event: SwiftPeerLivenessEvent) {}
        }

        struct WallClockProvider;
        impl HeartbeatTimestampProvider for WallClockProvider {
            fn clock_id(&self) -> String {
                "smoke-clock".to_string()
            }
            fn clock_hash(&self) -> String {
                "smoke-hash".to_string()
            }
            fn now_ns(&self) -> i64 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0)
            }
            fn domain_clock_bytes(&self) -> Option<Vec<u8>> {
                None
            }
        }

        let wallet = auki_identity::Wallet::from_seed(vec![1u8; 32]).expect("32-byte seed");
        let identity =
            std::sync::Arc::new(auki_network_rs::PeerIdentity::from_wallet(wallet));

        // UniFFI callback interfaces cross the FFI as `Box<dyn Trait>`;
        // pass them as `Box` here to match the exported function signature.
        let listener: Box<dyn PeerLivenessListener> = Box::new(NoopListener);
        let heartbeat: Box<dyn HeartbeatTimestampProvider> = Box::new(WallClockProvider);

        let rt = spawn_for_swift(
            identity,
            vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            vec![],
            listener,
            heartbeat,
        )
        .await
        .expect("spawn succeeds in test runtime");

        let pid = rt.local_peer_id_string();
        assert!(pid.starts_with("12D3KooW"), "expected canonical PeerId");

        assert!(rt.connected_peer_id_strings().is_empty());

        rt.shutdown();
    }

    /// Smoke test: a `HeartbeatTimestampProvider` impl can be converted
    /// into an upstream `HeartbeatTimestampSource` via the adapter.
    #[test]
    fn heartbeat_timestamp_provider_adapter() {
        struct WallClockProvider;
        impl HeartbeatTimestampProvider for WallClockProvider {
            fn clock_id(&self) -> String {
                "test-clock".to_string()
            }
            fn clock_hash(&self) -> String {
                "test-hash".to_string()
            }
            fn now_ns(&self) -> i64 {
                42
            }
            fn domain_clock_bytes(&self) -> Option<Vec<u8>> {
                None
            }
        }
        let provider: Arc<dyn HeartbeatTimestampProvider> = Arc::new(WallClockProvider);
        let src = heartbeat_source_from_provider(provider);
        assert_eq!(src.clock_id, "test-clock");
        assert_eq!(src.clock_hash, "test-hash");
        assert_eq!((src.now_ns)(), 42);
        assert!((src.domain_clock)().is_none());
    }
}
