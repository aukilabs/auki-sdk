# SDK Swift Binding Expansion Design

Status: draft design for user review.

Date: May 20, 2026.

## Goal

Expand the existing Swift binding family under `bindings/swift/` so that `aukilabs/iosapp` can prove end-to-end use of both the **identity** and **networking** SDK modules: load a Keychain-persisted wallet seed, derive the local libp2p `PeerId`, list clusters from Discovery, join a cluster, and observe peer membership-change events as they arrive. This is the SDK-side prerequisite for Spec 2 (iosapp wiring + proof-of-load UI), which depends entirely on the surfaces this spec defines.

The Swift bindings already exist for the Discovery HTTP client (`bindings/swift/auki-network-swift`, PRs #151 / #152, on `develop` at `aa97045`). This design adds the rest of the surface needed for the proof.

## Scope

In:

- New `bindings/swift/auki-identity-swift` crate exposing `Wallet` and `PeerIdentity`.
- Expansion of `bindings/swift/auki-network-swift` to add `NetworkRuntime` spawn, the `PeerLivenessEvent` callback stream, and the full stream surface (`StreamRequest`, `StreamManifest`, `StreamItem`, `StreamEntry`, `StreamSubscription`, payload-frame types, `StreamDecision` and `StreamProvider` callbacks, decline / end reasons). This folds in what was previously framed as "Stage 2."
- New `bindings/swift/auki-domain-swift` crate exposing the full `ClusterManager` surface at parity with `bindings/python/auki-domain-py`.
- Per-crate `build-xcframework.sh` build scripts; workspace `Cargo.toml` members; updated `bindings/swift/{README,changelog,parking_lot}.md`; new per-crate doc files following the auki-sdk per-component convention.

Out:

- iosapp-side work. The Keychain helper that implements iOS's analogue of `auki-identity::load_or_mint_seed`, `Bridge/AukiBridge.swift`, the `scripts/sync-sdk.sh` build orchestrator, the proof-of-load UI, the `DiscoveryGate` view — all live in Spec 2 (iosapp wiring), which is blocked by this spec.
- CI for any of the binding crates. Same deferral pattern as Stage 1's PR #151 — host `cargo test -p <crate>` is the gate during development; CI catches up later.
- A published SwiftPM-style distribution. Consumption is build-from-source via the iosapp-side sync script (decided in Spec 2's brainstorm).
- `auki-identity` surfaces beyond `Wallet` and `PeerIdentity` — no `Signature`, `verify`, `CreationCert`, `issue_creation_cert`, or `derive_child` exposed; not needed for the proof, expand later when a concrete feature wants them.

## Architecture

Three new or expanded binding crates under `bindings/swift/`, each independently owning its UniFFI scaffolding and an `OnceLock<Runtime>` tokio runtime, mirroring the per-component convention established for `bindings/python/`:

```text
bindings/swift/
├── README.md              ← updated index (three crates)
├── changelog.md           ← propagation entry per change
├── parking_lot.md         ← updated per-package summaries
├── auki-identity-swift/   ← NEW. Wallet + PeerIdentity.
├── auki-network-swift/    ← EXPANDED. Existing Discovery surface preserved.
└── auki-domain-swift/     ← NEW. Full ClusterManager parity with -py.
```

Dependency graph (all path-deps on the upstream Rust crates renamed via Cargo's `package =`, exactly as the existing `auki-network-swift` does for `auki-network`):

```text
auki-identity-swift  →  ../../crates/auki-identity
auki-network-swift   →  ../../crates/auki-network  (features: discovery_client + swarm)
                      + auki-identity-swift              (uses PeerIdentity in NetworkRuntime ctors)
auki-domain-swift    →  ../../crates/auki-domain
                      + auki-network-swift               (uses StreamSubscription in open_stream)
                      + auki-identity-swift              (uses PeerIdentity in bootstrap)
```

Each crate is staged in its own PR for review tractability:

1. `auki-identity-swift` (small, foundational, low-risk) lands first.
2. `auki-network-swift` expansion lands second. This is where the libp2p iOS cross-compile sharp edges surface or do not.
3. `auki-domain-swift` lands third, building on the prior two.

## `auki-identity-swift`

Minimum surface to derive a stable `PeerId` from a Keychain-managed seed:

```rust
#[derive(uniffi::Object)]
pub struct Wallet { inner: auki_identity::Wallet }

#[uniffi::export]
impl Wallet {
    /// Construct from a 32-byte ed25519 seed. iosapp's Keychain helper
    /// supplies the seed bytes; the `load_or_mint` policy stays Swift-side
    /// per the resolved Q6 in `aukilabs/iosapp`'s parking lot (Keychain,
    /// not filesystem — `auki_identity::load_or_mint_seed` is a fs helper).
    #[uniffi::constructor]
    pub fn from_seed(seed: Vec<u8>) -> Result<Arc<Wallet>, IdentityError>;

    /// Mint a fresh wallet with a CSPRNG-generated seed. Used by the
    /// Keychain helper's "mint" branch on first launch.
    #[uniffi::constructor]
    pub fn generate() -> Arc<Wallet>;

    /// Expose the 32-byte seed so the consumer can persist it to the
    /// Keychain (or wherever). Sensitive — treat as secret key material.
    pub fn seed(&self) -> Vec<u8>;

    /// Stable wallet identifier (`auki_identity::WalletId.0`); useful for
    /// logging without leaking the seed.
    pub fn wallet_id(&self) -> String;
}

#[derive(uniffi::Object)]
pub struct PeerIdentity { inner: auki_network::PeerIdentity }

#[uniffi::export]
impl PeerIdentity {
    /// Derive the libp2p peer identity from a wallet. Equivalent to the
    /// Rust `PeerIdentity::from_wallet(w)`, which under the hood is
    /// `from_seed(&w.derive_child("peer/v1").seed())`. `derive_child`
    /// stays unexposed at v0 — the binding only offers the canonical
    /// `peer/v1` derivation.
    #[uniffi::constructor]
    pub fn from_wallet(wallet: Arc<Wallet>) -> Arc<PeerIdentity>;

    /// Canonical libp2p peer-id string (`12D3KooW…`).
    pub fn peer_id(&self) -> String;
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("seed must be 32 bytes, got {actual}")]
    InvalidSeedLength { actual: u32 },
}
```

Out of this binding's scope at this stage: `Wallet::derive_child` (callers go through `PeerIdentity::from_wallet`), `Signature` / `verify` / `PublicKey`, `CreationCert` / `issue_creation_cert`. Expand when a concrete feature needs them.

Conventions: standard auki-sdk per-crate files (`README.md`, `parking_lot.md`, `changelog.md`, `src/readme.md`, `src/sprint.md`); workspace `members` and `bindings/swift/{README,parking_lot}.md` updated; `build-xcframework.sh` follows the existing `auki-network-swift` template.

## `auki-network-swift` expansion

The existing Discovery surface (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`) is **unchanged**. The expansion adds the libp2p runtime, the peer-event callback stream, and the full stream surface.

### NetworkRuntime + PeerLivenessEvent

```rust
#[derive(uniffi::Object)]
pub struct NetworkRuntime { /* owns the inner runtime + a handle */ }

#[uniffi::export(async_runtime = "tokio")]
impl NetworkRuntime {
    /// Build a libp2p Swarm rooted at `identity` and spawn the runtime.
    /// `allowed_peers` is the initial cluster allow-list (the trust
    /// boundary at the NetworkBehaviour layer); subsequent updates use
    /// `set_allowed_peers`. Every `PeerLivenessEvent` the runtime emits
    /// is fanned to `listener` on a tokio task.
    #[uniffi::constructor]
    pub async fn spawn(
        identity: Arc<PeerIdentity>,
        listen_multiaddrs: Vec<String>,
        allowed_peers: Vec<AllowedPeer>,
        listener: Arc<dyn PeerLivenessListener>,
    ) -> Result<Arc<NetworkRuntime>, SpawnError>;

    /// Canonical local peer-id string. Round-trips with PeerIdentity.peer_id().
    pub fn local_peer_id(&self) -> String;

    /// Sync snapshot of currently-connected peers.
    pub fn connected_peers(&self) -> Vec<String>;

    /// Replace the cluster allow-list. Returns a report of what changed.
    pub async fn set_allowed_peers(
        &self,
        peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError>;

    /// Round-trip request for a peer's `ParticipantInfo` JSON via
    /// `/auki/info/0.0.1`. JSON crosses the FFI as a String; Swift
    /// decodes via `JSONDecoder` (no UniFFI Record needed for the
    /// 11-field ParticipantInfo shape).
    pub async fn request_participant_info(
        &self,
        peer_id: String,
    ) -> Result<String, RequestInfoError>;

    /// Graceful shutdown: cleanly closes the swarm and stops the
    /// background task. Drop is the hard-stop fallback.
    pub fn shutdown(&self);
}

#[derive(uniffi::Record)]
pub struct AllowedPeer {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct UpdateReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// Async callback interface — Swift implements; Rust calls from the
/// event-drain task. Use this pattern (not a pull iterator) because the
/// natural SwiftUI consumption is `@Observable` reactive state updated
/// from these callbacks.
#[uniffi::export(callback_interface, async_runtime = "tokio")]
pub trait PeerLivenessListener: Send + Sync {
    fn on_event(&self, event: PeerLivenessEvent);
}

#[derive(uniffi::Enum)]
pub enum PeerLivenessEvent {
    Connected             { peer_id: String },
    Disconnected          { peer_id: String },
    HeartbeatReceived     { peer_id: String },
    HeartbeatStreamClosed { peer_id: String },
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("no tokio runtime context available")]
    NoTokioRuntime,
    #[error("invalid multiaddr: {value}")]
    InvalidMultiaddr { value: String },
    #[error("swarm bootstrap failed: {message}")]
    Swarm { message: String },
}

#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum RequestInfoError {
    #[error("open stream: {message}")] OpenStream { message: String },
    #[error("protocol: {message}")]    Protocol { message: String },
    #[error("timeout after {timeout_ms} ms")] Timeout { timeout_ms: u32 },
}
```

### Stream surface (Stage 2 folded in)

Mirrors the curated stream surface in `bindings/python/auki-network-py`. Each prost message type from `auki-datatypes` (`StreamRequest`, `StreamManifest`, `StreamItem`, `StreamEntry`, `DeclineReason`, `EndReason`, `StreamDecision`, `AudioFrame`, `JpegFrame`, `PointCloudFrame`, `JointEncodersFrame`) gets a UniFFI Record or Enum (hand-mapped). The wire-payload `bytes` fields (the audio PCM, the JPEG body, the CDR point-cloud bytes) cross the FFI as `Vec<u8>` and are decoded Swift-side via **swift-protobuf** against the committed `crates/auki-datatypes/proto/*.proto` — the same `.proto` `prost-build` consumes for the Rust side. No Swift-side `.proto` authoring; wire compatibility is structural.

Consumer side:

```rust
#[derive(uniffi::Object)]
pub struct StreamSubscription { /* wraps the inner futures::Stream */ }

#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscription {
    pub fn manifest(&self) -> StreamManifest;

    /// Async pull: returns the next entry, or `Ok(None)` on clean end.
    /// `Ok(None)` is sticky — subsequent calls after the first `Ok(None)`
    /// keep returning `Ok(None)`. Mid-stream errors (protocol error,
    /// connection loss) are returned as a terminal `Result::Err`; after
    /// an error, subsequent calls also return `Ok(None)`.
    pub async fn next(&self) -> Result<Option<StreamEntry>, StreamError>;

    /// Explicit cancellation. Drops the underlying receiver; the
    /// producer sees the substream close. Drop is the implicit
    /// equivalent.
    pub fn cancel(&self);
}
```

Producer side uses a callback interface symmetric to the Python `_build_stream_provider` PyCapsule bridge:

```rust
#[uniffi::export(callback_interface, async_runtime = "tokio")]
pub trait StreamProvider: Send + Sync {
    async fn dispatch(
        &self,
        requester_peer_id: String,
        request: StreamRequest,
    ) -> StreamDispatch;
}

#[derive(uniffi::Enum)]
pub enum StreamDispatch {
    AcceptCamera         { manifest: StreamManifest, source: Arc<dyn FrameSource> },
    AcceptPointCloud     { manifest: StreamManifest, source: Arc<dyn PointCloudSource> },
    AcceptJointEncoders  { manifest: StreamManifest, source: Arc<dyn JointEncodersSource> },
    AcceptAudio          { manifest: StreamManifest, source: Arc<dyn AudioSource> },
    AcceptDetection      { manifest: StreamManifest, source: Arc<dyn DetectionSource> },
    Decline              { reason: DeclineReason },
}
```

Each `*Source` is an async callback interface emitting `StreamItem<T>` via an `async fn next() -> Option<StreamItem>` method; clean end is `None`, error end is an exception/error variant. Swift-side producers implement these to publish a sensor.

This is the largest surface in the expansion. It is intentionally curated to the consumer/producer ergonomic the Python binding already proves — not every internal `auki-network::stream_protocol` type is exposed.

## `auki-domain-swift`

Full `ClusterManager` parity with `bindings/python/auki-domain-py`. The surface is large; this section describes the shape, not every signature.

```rust
#[derive(uniffi::Object)]
pub struct ClusterTarget { /* opaque; built via ::create / ::join / ::join_or_create / ::most_recent_or_create */ }

#[uniffi::export]
impl ClusterTarget {
    #[uniffi::constructor] pub fn create(name: String) -> Arc<ClusterTarget>;
    #[uniffi::constructor] pub fn join(name: String) -> Arc<ClusterTarget>;
    #[uniffi::constructor] pub fn join_or_create(name: String) -> Arc<ClusterTarget>;
    #[uniffi::constructor] pub fn most_recent_or_create(fallback_name: String) -> Arc<ClusterTarget>;
}

#[derive(uniffi::Object)]
pub struct ClusterManager { /* owns the inner ClusterManager */ }

#[uniffi::export(async_runtime = "tokio")]
impl ClusterManager {
    /// The main entry point. Looks up Discovery, dials the Manager (if
    /// joining) or claims the name (if creating), receives the
    /// ClusterDoc, spawns the NetworkRuntime with the right allow-list,
    /// and returns a running handle. If `membership_listener` is set, it
    /// receives membership-change events for the lifetime of this
    /// handle.
    #[uniffi::constructor]
    pub async fn bootstrap(
        target: Arc<ClusterTarget>,
        identity: Arc<PeerIdentity>,
        discovery_url: String,
        listen_multiaddrs: Vec<String>,
        membership_listener: Option<Arc<dyn MembershipListener>>,
    ) -> Result<Arc<ClusterManager>, BootstrapError>;

    /// Discovery-only directory listing. Useful for "let the user pick
    /// a cluster" UIs that don't want to bootstrap immediately.
    pub async fn list_clusters(
        discovery_url: String,
    ) -> Result<Vec<DiscoveryClusterEntry>, DiscoveryClientError>;

    /// Explicit create (vs. join). Same return-shape as bootstrap.
    pub async fn create_cluster(/* target, identity, discovery_url, listen_multiaddrs, listener */)
        -> Result<Arc<ClusterManager>, CreateClusterError>;

    // Introspection — all sync, snapshot semantics.
    pub fn cluster_name(&self) -> String;
    pub fn local_peer_id(&self) -> String;
    pub fn local_multiaddrs(&self) -> Vec<String>;
    pub fn manager_peer_id(&self) -> String;
    pub fn is_manager(&self) -> bool;
    pub fn peer_count(&self) -> u32;
    pub fn membership(&self) -> ClusterMembership;
    pub fn participant_info(&self) -> String;   // JSON

    // Member management
    pub async fn admit_peer(
        &self,
        peer_id: String,
        multiaddrs: Vec<String>,
    ) -> Result<(), AdmitError>;

    // Peer-side fetches
    pub async fn fetch_participant_info(&self, peer_id: String) -> Result<String, FetchParticipantInfoError>;
    pub async fn fetch_sensors_catalog(&self) -> Result<SensorsResponse, FetchSensorsCatalogError>;
    pub async fn fetch_resources_catalog(&self) -> Result<ResourcesResponse, FetchResourcesCatalogError>;
    pub async fn fetch_sensor_entry(&self, peer_id: String, id: String, hash: String) -> Result<SensorRegistryEntry, FetchRegistryEntryError>;
    pub async fn fetch_clock_entry(&self,  peer_id: String, id: String, hash: String) -> Result<ClockRegistryEntry,  FetchRegistryEntryError>;
    pub async fn fetch_frame_entry(&self,  peer_id: String, id: String, hash: String) -> Result<FrameRegistryEntry,  FetchRegistryEntryError>;

    // Producer-side provider hooks (callback interfaces — see below).
    pub fn set_sensor_catalog_provider(&self,   provider: Arc<dyn SensorCatalogProvider>);
    pub fn set_resource_catalog_provider(&self, provider: Arc<dyn ResourceCatalogProvider>);
    pub fn set_registry_app_root(&self, app_root: String);

    // Streams — delegates to auki-network-swift's StreamSubscription.
    pub async fn open_stream(
        &self,
        peer_id: String,
        request: StreamRequest,
    ) -> Result<Arc<StreamSubscription>, OpenStreamError>;

    pub fn shutdown(&self);
}

/// Membership-change callback interface — Swift implements, Rust calls
/// when the cluster membership changes. Distinct from
/// PeerLivenessListener (which is libp2p-level connectivity); this is
/// cluster-membership-level (who is logically in the cluster).
#[uniffi::export(callback_interface)]
pub trait MembershipListener: Send + Sync {
    fn on_member_joined(&self, peer_id: String);
    fn on_member_left(&self, peer_id: String);
    fn on_manager_changed(&self, new_manager_peer_id: String);
}

#[uniffi::export(callback_interface, async_runtime = "tokio")]
pub trait SensorCatalogProvider: Send + Sync {
    async fn provide(&self) -> SensorsResponse;
}

#[uniffi::export(callback_interface, async_runtime = "tokio")]
pub trait ResourceCatalogProvider: Send + Sync {
    async fn provide(&self) -> ResourcesResponse;
}
```

Supporting Records / Enums hand-mapped from the upstream Rust types: `ClusterMembership` (`ClusterMember` list — peer_id + multiaddrs + joined_at_ns + role), `DiscoveryClusterEntry` (same as the existing `ClusterEntry` in `auki-network-swift` but re-exported), `SensorsResponse` / `ResourcesResponse` (proto-derived, prost-payload bytes opaque), `SensorRegistryEntry` / `ClockRegistryEntry` / `FrameRegistryEntry` (canonical JSON-friendly Records). Errors (`BootstrapError`, `CreateClusterError`, `AdmitError`, `DiscoveryClientError`, `FetchSensorsCatalogError`, `FetchResourcesCatalogError`, `FetchRegistryEntryError`, `FetchParticipantInfoError`, `OpenStreamError`) flattened to UniFFI enums with the same hide-reqwest/hide-libp2p discipline the existing `DiscoveryError` follows.

## Cross-cutting UniFFI patterns

**Async runtime ownership.** Each crate declares its own process-wide `OnceLock<Runtime>` for tokio, lazily initialized on first use, exactly like `bindings/python/auki-network-py`'s `cluster_tokio_runtime()`. UniFFI's `async_runtime = "tokio"` consumes that runtime when polling exported futures. Three runtimes coexist at process-time; this is accepted overhead for now and flagged as a future-defer parking-lot item in each crate ("share a single runtime via an internal common crate if profiling shows worker-thread pressure"). On iPhone 15 Pro / 6-core silicon, three default multi-thread runtimes mean ~18 worker threads at the runtime layer — measurable but not pathological.

**Callback interfaces.** UniFFI 0.31 supports `#[uniffi::export(callback_interface)]` plus async callback methods. Used for: `PeerLivenessListener`, `MembershipListener`, `SensorCatalogProvider`, `ResourceCatalogProvider`, `StreamProvider`, and the per-payload `*Source` traits used by `StreamDispatch::Accept*`. Swift implements the protocol; Rust spawns a tokio task that drains the inner `mpsc::Receiver` (for the event streams) or invokes the trait method directly (for one-shot providers), and forwards into Swift. Cancellation: the consumer drops the returned object; UniFFI's Drop fires, the runtime task is aborted, the underlying receiver/stream is dropped, the upstream notices the close.

**Prost payloads as opaque bytes.** Every prost message field whose Rust shape is `bytes` (`AudioFrame.data`, `JpegFrame.bytes`, `PointCloudFrame.bytes`, `StreamEntry.payload`) crosses the FFI as `Vec<u8>` / `Data` and is encoded/decoded Swift-side via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`. The Rust binding does not re-state any prost type as a UniFFI Record; that path was rejected in the Q1 thin-binding decision (Stage 1) because it duplicates schema authoring and forces per-field marshalling on the audio hot path. The binding crates that touch stream types add a `swift-protobuf-vectors-locked.txt`-style test in a follow-up (mirrors the existing `bindings/python/auki-datatypes-py/tests/test_locked_vectors.py` cross-language wire compat check); not in Spec 1.

**`PeerId` and `Multiaddr` at the FFI seam.** Both cross as canonical strings (`"12D3KooW…"`, `"/ip4/.../tcp/4001"`). The binding parses them at the boundary into `libp2p_identity::PeerId` / `multiaddr::Multiaddr`; parse failures map to the relevant error enum's `InvalidPeerId` / `InvalidMultiaddr` variant. iosapp never sees a libp2p type. This matches the existing `auki-network-swift` Stage 1 discipline.

## iOS cross-compile risks

Stage 1's `auki-network-swift` build was clean against rustc 1.94 + Xcode 26.3 because it only pulled `auki-network` with the `discovery_client` feature (HTTP + reqwest + rustls + `ring 0.17`). The expansion pulls the `swarm` feature, which brings the full libp2p stack into the iOS cross-compile.

Predicted hazards:

- **`SystemConfiguration.framework` link.** Some libp2p transport components reference `_kSCNetworkInterfaceType*` symbols indirectly (via `if-watch` and similar address-enumeration libraries). The static lib will link, but the consuming Xcode target may need to add `SystemConfiguration.framework` to its linked frameworks. Mitigation: document this in the new `auki-network-swift` Stage 2 sprint notes; Spec 2's iosapp wiring picks it up if it surfaces.
- **QUIC/UDP on iOS.** Works in the foreground; iOS suspends UDP sockets when the app backgrounds. This is the cluster-side manifestation of the iosapp Q2 background-execution decision. No binding-side mitigation needed; the runtime simply observes connection loss and emits `Disconnected` events.
- **`mac_address` crate.** Used by `auki-network`'s `app_instance` feature for per-machine identifier derivation. The expansion **does not enable `app_instance`** — that feature uses platform-specific syscalls (`getifaddrs` / `GetAdaptersAddresses`) and is unnecessary for the proof. Keep the feature off.
- **`ring` cross-compile.** Proven non-issue at Stage 1; remains non-issue with the swarm feature added (libp2p's noise transport uses the same `ring` path).
- **`libp2p-stream` pinned to `=0.4.0-alpha`** in `auki-network`. Pre-1.0, paired with libp2p 0.56. Keep the same pin in the binding crate.

Validation: the same `build-xcframework.sh` template Stage 1 uses, run against all three Apple targets after each PR lands. If `SystemConfiguration` is needed at link-time, the binding crate's README documents it for the iosapp consumer.

## Conventions

Each **new** crate (`auki-identity-swift`, `auki-domain-swift`) carries the standard auki-sdk per-component files; the **expanded** crate (`auki-network-swift`) updates its existing ones in place. Files in both cases:

- `README.md` — aspirational spec for that crate.
- `parking_lot.md` — open questions specific to the crate (e.g., the per-crate "single shared tokio runtime?" future-defer item).
- `changelog.md` — append-only, propagates up.
- `src/readme.md` — honest implementation status.
- `src/sprint.md` — current work, next steps, out of scope.

Workspace `Cargo.toml` gets three new `members` entries (adjacent to the existing `bindings/swift/auki-network-swift` line). `bindings/swift/README.md` updates its table to list all three crates. `bindings/swift/parking_lot.md` updates the per-package summary. `bindings/changelog.md` and root `changelog.md` get one-liners per PR.

Each crate gets its own `build-xcframework.sh`, copied from the existing `auki-network-swift/build-xcframework.sh` template. The iosapp-side sync script (Spec 2) invokes them in dependency order.

Resolved parking-lot items get **deleted entirely** per the auki-sdk CLAUDE.md rule (one append-only exception); resolution is recorded in the changelog only.

## Testing

Host-only at Spec 1 stage, mirroring the Stage 1 precedent:

- `cargo test -p auki-identity-swift` — error mapping (`InvalidSeedLength`), seed round-trip, `PeerIdentity::from_wallet` determinism (same seed → same peer-id).
- `cargo test -p auki-network-swift` — preserves the existing 4 Stage 1 tests; adds: `PeerLivenessEvent` enum round-trip, `AllowedPeer` Record round-trip, `SpawnError` variants. Integration test for actually spawning a runtime is deferred to a CI/two-host harness.
- `cargo test -p auki-domain-swift` — error mapping for each new error enum, Record round-trips for `ClusterMembership` / `DiscoveryClusterEntry` / `SensorsResponse` / `ResourcesResponse`.

Each crate runs `build-xcframework.sh` against the three iOS targets as the integration validation; the output XCFramework + Swift glue are inspected for surface correctness (correct slice arches, async signatures present, no stray `bytes`-to-non-bytes type misses).

Cross-language wire-vector tests (verifying Swift swift-protobuf decodes the same bytes the Rust prost encodes) are deferred — added in a follow-up that introduces the harness, same pattern as the `bindings/python/auki-datatypes-py/tests/test_locked_vectors.py` precedent.

## Implementation staging

This spec breaks naturally into three PRs against `auki-sdk` `develop`:

1. **PR A** — `auki-identity-swift` (new crate). Small, foundational. Includes per-crate doc files, workspace members update, `bindings/swift/{README,parking_lot}.md` update, `build-xcframework.sh`. Validates the iOS XCFramework flow for a fresh crate from scratch. Acts as the template the other two follow.
2. **PR B** — `auki-network-swift` expansion. Adds the `swarm` feature dependency, `NetworkRuntime` + `PeerLivenessEvent` + the full stream surface, depends on `auki-identity-swift` (PR A's branch). This PR is where the iOS libp2p cross-compile sharp edges surface. Risk-bear it on its own so the cause is unambiguous if something breaks.
3. **PR C** — `auki-domain-swift` (new crate). Depends on PRs A and B. Full `ClusterManager` parity. Largest PR; the design above is the contract it implements.

Each PR follows the standard hierarchical changelog/parking_lot propagation (leaf → per-crate → `bindings/swift/` → `bindings/` → root) and the CLAUDE.md "resolved items deleted" rule.

## Open items

These belong in the new crates' parking lots (filed during PR A):

- **Single shared tokio runtime across the three binding crates.** Three `OnceLock<Runtime>` instances mean three worker pools. Acceptable for now; revisit if profiling on an iPhone 15 Pro shows thread-pressure pain. The fix is a small internal `auki-bindings-swift-common` crate; defer until measurable need.
- **Where generated Swift + XCFramework artifacts live.** This is the same item Stage 1 already parked. The Spec 2 decision (build-from-source via iosapp's sync script, no published SwiftPM package) settles it operationally for now; the question of an eventual SwiftPM-published artifact when consumers outside iosapp arrive is still open.
- **`with_http` and other Rust escape-hatches not exposed at v0.** Mirrors the existing Stage 1 parking-lot item. Recheck per binding crate; expose when a real deployment needs the knob (proxies, custom TLS roots, custom timeouts).
- **Async API shape vs. `-py`'s sync shape.** Already flagged for human confirmation in the Stage 1 parking lot. Same decision propagates to the new crates: async is correct for iOS; sync façade is rejected.
- **Cross-language wire-vector test harness for the stream surface.** Mirrors the Python locked-vectors test; needs a small Swift test setup. Deferred.

## Out of scope (explicit)

- All of Spec 2 (iosapp wiring): the Keychain helper, `Bridge/AukiBridge.swift`, `scripts/sync-sdk.sh`, the proof-of-load UI, `DiscoveryGate`, `ContentView` replacement. Spec 2 is blocked by this spec.
- CI for any binding crate. Same deferral pattern as the existing `bindings/swift/auki-network-swift` (no CI yet).
- A published SwiftPM-format package. Distribution is build-from-source via the iosapp sync script (Spec 2).
- Sign-and-publish / TestFlight implications for iosapp. Spec 2 + iosapp's existing Q7 resolution own that.
- `auki-identity` surfaces beyond `Wallet` / `PeerIdentity`. Beyond `PeerLivenessListener`, `MembershipListener`, `SensorCatalogProvider`, `ResourceCatalogProvider`, `StreamProvider` — no other callback interfaces are exposed at v0.
- Cluster lifecycle features that the Python equivalent does not yet ship (e.g., Manager handoff coordination, successor token verification — those are upstream Rust changes, not binding work).
