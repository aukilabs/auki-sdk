# SDK Swift Binding Expansion Design

Status: draft design for user review (revision 2).

Date: May 20, 2026.

## Goal

Expand the existing Swift binding family under `bindings/swift/` so that `aukilabs/iosapp` can prove end-to-end use of both the **identity** and **networking** SDK modules: load a Keychain-persisted wallet seed, derive the local libp2p `PeerId`, list clusters from Discovery, join a cluster, and observe peer membership-change events as they arrive. This is the SDK-side prerequisite for Spec 2 (iosapp wiring + proof-of-load UI), which depends entirely on the surfaces this spec defines.

The Swift bindings already exist for the Discovery HTTP client (`bindings/swift/auki-network-swift`, PRs #151 / #152, on `develop` at `aa97045`). This design adds the rest of the surface needed for the proof.

**Approach: minimal binding-crate wrapping.** The UniFFI proc-macros live on the upstream Rust types directly (`auki-identity::Wallet`, `auki-network::NetworkRuntime`, `auki-domain::ClusterManager`, etc.), gated behind a new optional `swift-bindings` cargo feature on each upstream crate. The Swift binding crates under `bindings/swift/` become thin scaffolding hosts: a `setup_scaffolding!()` call, custom-type registrations for the libp2p types (`PeerId` / `Multiaddr` → `String`), a few `pub use` re-exports, the per-component doc files, and the `build-xcframework.sh` build script. No `pub struct FooSwift { inner: Foo }` wrappers, no hand-mapped UniFFI Records that duplicate upstream pure-data structs, no per-method shim methods. UniFFI introspects the upstream types and generates the Swift surface from them directly.

## Scope

In:

- New `swift-bindings` cargo feature on each of `crates/auki-identity`, `crates/auki-network`, `crates/auki-domain`. When the feature is off (the default), the crate compiles exactly as today — no UniFFI dep pulled in. When on, the public types relevant to the Swift binding gain `#[derive(uniffi::Object|Record|Enum)]` and the relevant methods gain `#[uniffi::export]`. Annotations gated by `#[cfg(feature = "swift-bindings")]` so they vanish when the feature is off.
- New `bindings/swift/auki-identity-swift` crate (thin scaffolding host).
- Expansion of `bindings/swift/auki-network-swift`: enable the upstream `swift-bindings` feature, add the rest of the scaffolding (NetworkRuntime, PeerLivenessEvent callback, full stream surface — Stage 2 folded in). Existing Discovery surface stays operational.
- New `bindings/swift/auki-domain-swift` crate (thin scaffolding host).
- Per-crate `build-xcframework.sh`; workspace `Cargo.toml` members; updated `bindings/swift/{README,changelog,parking_lot}.md`; new per-crate doc files following the auki-sdk per-component convention.

Out:

- iosapp-side work. The Keychain helper, `Bridge/AukiBridge.swift`, `scripts/sync-sdk.sh`, the proof-of-load UI, `DiscoveryGate`, `ContentView` replacement — all live in Spec 2.
- CI for any of the binding crates or the new feature compilations. Same deferral pattern as Stage 1's PR #151.
- A published SwiftPM-style distribution. Consumption stays build-from-source via the iosapp-side sync script (decided in Spec 2's brainstorm).
- `auki-identity` surfaces beyond `Wallet` and `PeerIdentity` annotation. `Signature`, `verify`, `CreationCert`, `issue_creation_cert`, `derive_child` stay un-exported at v0.
- Changing the public Rust API of any upstream crate. The `swift-bindings` feature is **additive only** — it adds derives and annotations; it does not move methods, change signatures, or break existing consumers. If a method's signature is incompatible with UniFFI export (e.g. takes a borrowed `&PeerId`, returns a non-FFI-safe type), the binding crate may add a small wrapper around it rather than mutate the upstream signature.

## Architecture

Three upstream Rust crates each gain a `swift-bindings` feature. Three binding crates under `bindings/swift/` consume those features and host the UniFFI scaffolding:

```text
crates/
├── auki-identity/        ← +features=["swift-bindings"]; +derive annotations on Wallet, PeerIdentity
├── auki-network/         ← +features=["swift-bindings"]; +annotations on NetworkRuntime, stream types, etc.
└── auki-domain/          ← +features=["swift-bindings"]; +annotations on ClusterManager, ClusterTarget, etc.

bindings/swift/
├── README.md             ← updated index (three crates)
├── changelog.md
├── parking_lot.md
├── auki-identity-swift/  ← NEW. Thin scaffolding host (~30 LOC of Rust).
├── auki-network-swift/   ← EXPANDED. Enables the new feature; adds custom types
│                           and scaffolding for the network + stream surfaces.
└── auki-domain-swift/    ← NEW. Thin scaffolding host (~50 LOC of Rust).
```

Dependency graph (binding-crate Cargo deps; the `swift-bindings` feature on each is the carrier of the UniFFI exports):

```text
auki-identity-swift  →  ../../crates/auki-identity { features=["swift-bindings"] }
auki-network-swift   →  ../../crates/auki-network  { features=["swift-bindings","swarm","discovery_client"] }
                      +  ../auki-identity-swift             (re-exports the PeerIdentity custom-type registration)
auki-domain-swift    →  ../../crates/auki-domain   { features=["swift-bindings"] }
                      +  ../auki-network-swift              (re-exports stream types, NetworkRuntime)
                      +  ../auki-identity-swift             (PeerIdentity reachability)
```

The binding crates `cdylib`/`staticlib` outputs each include the scaffolding metadata for whatever upstream types are linked with `swift-bindings` enabled. Each binding crate's `cargo build` triggers compilation of the upstream crate with `swift-bindings` on, picking up the annotations.

## How "minimal wrapping" works in practice

Two UniFFI features carry the weight:

**1. Proc-macros on upstream types, gated by feature.** The upstream crate's source gets blocks like:

```rust
// crates/auki-identity/src/lib.rs
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct Wallet { signing_key: SigningKey }

#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl Wallet {
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn from_seed(seed: &[u8; 32]) -> Self { /* ... */ }
    // ... etc.
}
```

When the feature is off (the default), `cfg_attr` strips the annotations and the code compiles unchanged for Python sidecars / Rust daemons. When on, UniFFI generates scaffolding metadata for the type and methods.

The binding crate's `Cargo.toml` enables the feature on its path-dep:

```toml
# bindings/swift/auki-identity-swift/Cargo.toml
[dependencies]
auki-identity = { path = "../../../crates/auki-identity", features = ["swift-bindings"] }
```

**2. Custom types for libp2p seam.** `PeerId` and `Multiaddr` are external types (from `libp2p-identity` and `multiaddr` crates) that we cannot annotate. The binding crate declares them as UniFFI custom types that lower to `String`:

```rust
// bindings/swift/auki-network-swift/src/lib.rs
uniffi::custom_type!(PeerId, String, {
    try_lift: |s: String| s.parse::<PeerId>().map_err(|e| anyhow!("invalid peer-id: {e}")),
    lower: |p: PeerId| p.to_string(),
});

uniffi::custom_type!(Multiaddr, String, { /* similar */ });
```

After these declarations, every upstream method that takes or returns `PeerId` / `Multiaddr` (in any of `auki-network`, `auki-domain`, etc.) is auto-exposed across the FFI with String at the seam, no per-method wrapping. **This is where minimal wrapping pays off most** — one declaration replaces dozens of hand-written conversion shims.

The binding crate re-exports the custom-type registration so downstream binding crates (e.g. `auki-domain-swift` depending on `auki-network-swift`) inherit it without redeclaring.

**3. Prost payloads stay opaque `Vec<u8>`.** UniFFI cannot introspect prost-generated structs and we deliberately don't want it to (per Q1's Stage 1 decision: prost payloads cross as bytes; Swift decodes via swift-protobuf). The relevant fields on `StreamEntry.payload`, `AudioFrame.data`, etc. already are `bytes` in the wire format; the upstream Rust types expose them as `Vec<u8>`. UniFFI auto-exposes `Vec<u8>` to Swift `Data`. Zero extra Rust code.

**4. Async, callback interfaces, errors.** Standard UniFFI proc-macros work the same way on upstream types as on binding-crate types:

- `#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]` on existing `pub async fn` methods.
- Trait-object parameters that we want as Swift callback interfaces (e.g. the `StreamProvider` callable, the `MembershipListener` notifier — *if* upstream uses traits at those seams) are declared with `#[cfg_attr(feature = "swift-bindings", uniffi::export(callback_interface))]` on the trait.
- Existing error enums (`DiscoveryError`, `BootstrapError`, etc.) get `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]`. Variants whose fields aren't FFI-friendly (e.g. `Transport(reqwest::Error)`) get flattened — this is the only place hand-mapping survives, and only at the error-enum layer.

Where the upstream type does **not** match what we want to surface (e.g. a `pub async fn that returns Result<NetworkRuntime, _>` where `NetworkRuntime` holds `Swarm<Behaviour>` non-FFI-friendly internals), we keep a small upstream-side shim: a parallel `pub async fn spawn_for_swift(...)` that takes the FFI-friendly inputs and returns the FFI-friendly outputs. These shims are **in the upstream crate**, behind the same `swift-bindings` feature flag, **not** in the binding crate. The binding crate stays a pure scaffolding host. Expected count: small — probably 2–4 such shims total across all three crates.

## Per-crate plan

### `crates/auki-identity` + `bindings/swift/auki-identity-swift`

Upstream additions (behind `swift-bindings`):

- `#[derive(uniffi::Object)]` on `Wallet`. Constructors `from_seed` and `generate` annotated `#[uniffi::constructor]`. Methods `seed()` and `wallet_id()` annotated `#[uniffi::export]`.
- `#[derive(uniffi::Object)]` on `PeerIdentity` (from `auki-network`, actually — but `PeerIdentity::from_wallet(Wallet)` lives in `auki-network::lib.rs`, so this annotation goes there). Method `peer_id()` annotated. Constructor `from_wallet(wallet: Arc<Wallet>)` annotated.

The binding crate is essentially:

```rust
// bindings/swift/auki-identity-swift/src/lib.rs
uniffi::setup_scaffolding!();
```

Plus the per-component doc files, `build-xcframework.sh`, `Cargo.toml` enabling the feature.

Out of upstream-annotation scope (kept unexposed): `derive_child`, `Signature`, `verify`, `PublicKey` (external use), `CreationCert`, `issue_creation_cert`.

### `crates/auki-network` + `bindings/swift/auki-network-swift`

Upstream additions (behind `swift-bindings`):

- `#[derive(uniffi::custom_type)]` registrations are declared in the binding crate, not upstream — but the upstream `lib.rs` re-exports `PeerId`, `Multiaddr` so they're reachable.
- `#[derive(uniffi::Object)]` on `NetworkRuntime` (the runtime handle in `network_runtime.rs`). Methods: `local_peer_id()`, `connected_peers()`, `set_allowed_peers()`, `request_participant_info()`, `shutdown()` get `#[uniffi::export]` (with `async_runtime = "tokio"` where async).
- `NetworkRuntime::spawn(...)` cannot be auto-exposed as-is — it returns `(Self, mpsc::Receiver<JoinEvent>, mpsc::Receiver<PeerLivenessEvent>, ...) ` (8 channels). Need a small upstream shim: `spawn_for_swift(identity, listen_multiaddrs, allowed_peers, peer_liveness_listener: Arc<dyn PeerLivenessListener>, ...) -> Result<Arc<Self>, SpawnError>` that wires each channel to its corresponding callback interface and returns just the runtime handle. **This is the largest piece of upstream Rust this spec adds.**
- `PeerLivenessEvent` enum: `#[derive(uniffi::Enum)]`. Already a clean data enum.
- `AllowedPeer` record (already exists): `#[derive(uniffi::Record)]`.
- `PeerLivenessListener` trait (NEW upstream): callback interface, `#[uniffi::export(callback_interface)]`. The shim wires the upstream `mpsc::Receiver<PeerLivenessEvent>` to this listener via a spawned tokio task.
- Existing `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`: gain the feature-gated UniFFI annotations and **replace** the equivalent hand-wrapped Stage 1 binding-crate types. The current `bindings/swift/auki-network-swift/src/lib.rs` becomes mostly empty — the hand-written `pub struct DiscoveryClient { inner: ... }` and the `From<RustClusterEntry> for ClusterEntry` impls go away.
- Stream surface (`stream_protocol.rs`, `stream_runtime.rs`): all the public types (`StreamRequest`, `StreamManifest`, `StreamItem`, `StreamEntry`, `StreamSubscription`, payload-frame types like `AudioFrame`/`JpegFrame`/`PointCloudFrame`, `DeclineReason`, `EndReason`, `StreamDecision`) annotated. The payload-frame structs are prost-derived; their `bytes` fields auto-expose as `Vec<u8>`. The `StreamProvider` and `*Source` (`SourceStream<T>`) traits get callback-interface annotations. The `open_stream` method on `NetworkRuntime` is auto-exposed (returns `StreamSubscription<T>` — generics over `T` may need a per-payload-type concretization, e.g. `open_audio_stream` / `open_pointcloud_stream` shims since UniFFI cannot generate generic FFI surfaces; small upstream additions).

Binding crate (`bindings/swift/auki-network-swift/src/lib.rs`): scaffolding + custom-type registrations for `PeerId` and `Multiaddr` + a `pub use auki_network::*` (or specific re-exports) so the UniFFI metadata is bundled. Stage 1's existing hand-written wrappers are deleted as the upstream annotations take over.

### `crates/auki-domain` + `bindings/swift/auki-domain-swift`

Upstream additions (behind `swift-bindings`):

- `#[derive(uniffi::Object)]` on `ClusterManager` and `ClusterTarget`. The four `ClusterTarget` constructors (`create`, `join`, `join_or_create`, `most_recent_or_create`) annotated `#[uniffi::constructor]`.
- All `pub fn` / `pub async fn` methods on `ClusterManager` annotated `#[uniffi::export]`: `bootstrap`, `list_clusters`, `create_cluster`, `cluster_name`, `local_peer_id`, `local_multiaddrs`, `manager_peer_id`, `is_manager`, `peer_count`, `participant_info`, `membership`, `admit_peer`, `fetch_*_catalog`, `fetch_*_entry`, `fetch_participant_info`, `open_stream`, `set_*_provider`, `set_registry_app_root`, `shutdown`.
- Records: `ClusterMembership`, `ClusterMember`, `DiscoveryClusterEntry`, `SensorsResponse`, `ResourcesResponse`, `SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `RegistryKind`, `DaemonInfo`, `SensorEntry`, `SensorsRequest`, etc. — `#[derive(uniffi::Record)]` or `#[derive(uniffi::Enum)]`.
- Errors: `BootstrapError`, `CreateClusterError`, `AdmitError`, `DiscoveryClientError`, `FetchSensorsCatalogError`, `FetchResourcesCatalogError`, `FetchRegistryEntryError`, `FetchParticipantInfoError`, `JoinClusterError` — `#[derive(uniffi::Error)]`. Where variants wrap non-FFI types, flatten with `#[uniffi(flat_error)]` or by replacing the wrapped variant's payload with a `message: String` field.
- Callback interfaces: `SensorCatalogProvider`, `ResourceCatalogProvider`, `MembershipListener` (NEW upstream trait — needs upstream design for how membership changes are observed; currently `ClusterMembership` is a snapshot, no event stream). The membership-listener may need a small upstream shim that converts an internal mpsc into trait calls, same pattern as `PeerLivenessListener` on `NetworkRuntime`.
- `bootstrap` constructor likely needs the `peer_liveness_listener` and `membership_listener` parameters; if the underlying `auki_domain::ClusterManager::bootstrap` doesn't already accept them, this is another small upstream shim.

Binding crate (`bindings/swift/auki-domain-swift/src/lib.rs`): scaffolding + re-exports. Depends on `auki-network-swift` to pick up the custom-type registrations for `PeerId`/`Multiaddr` and the stream-type metadata.

## Cross-cutting UniFFI patterns

**Async runtime ownership.** Per-crate `OnceLock<Runtime>` lives in each binding crate's `lib.rs` (not in the upstream crate — upstream stays runtime-agnostic). UniFFI's `async_runtime = "tokio"` annotation on the upstream methods picks up the runtime configured by the binding crate's setup. Three runtimes at process-time, same trade-off as the original design.

**Callback interfaces** for `PeerLivenessListener`, `MembershipListener`, `SensorCatalogProvider`, `ResourceCatalogProvider`, `StreamProvider`, and the per-payload `*Source` traits. UniFFI 0.31's `#[uniffi::export(callback_interface)]` declares the trait as a Swift-implementable protocol. Swift implements; Rust spawns a tokio task that drains the internal `mpsc::Receiver` and invokes the trait method.

**Custom types for libp2p seam.** Two `uniffi::custom_type!` declarations per binding crate that touches `auki-network` exports (just `auki-network-swift`; downstream binding crates re-export the registrations). Once declared, every upstream method that uses `PeerId`/`Multiaddr` is auto-exposed.

**Prost payloads as opaque bytes.** Already byte-shaped at the upstream type level (`bytes` proto field → `Vec<u8>` Rust field). UniFFI auto-exposes `Vec<u8>` → Swift `Data`. Zero binding-crate code; swift-protobuf decodes payloads against the committed `crates/auki-datatypes/proto/*.proto`.

**Errors.** `#[derive(uniffi::Error)]` on the upstream error enum. Variants whose payloads aren't FFI-safe (`reqwest::Error`, `libp2p_stream::OpenStreamError`) replaced or `#[uniffi(flat_error)]`-treated so they cross as a Display'd message string.

## iOS cross-compile risks

Unchanged from the previous design pass — the dep tree pulled when `swarm` is enabled is the same regardless of where the UniFFI annotations live:

- **`SystemConfiguration.framework` link.** Likely needed when the binding crate's xcframework lands in iosapp; the consuming Xcode target adds the framework as a linked library. Document in the `auki-network-swift` sprint notes.
- **QUIC/UDP on iOS.** Foreground-only; iosapp's Q2 background-execution decision handles the lifecycle.
- **`mac_address` crate.** `auki-network`'s `app_instance` feature stays off in the binding crate's feature set.
- **`ring` cross-compile.** Proven non-issue at Stage 1.
- **`libp2p-stream` pinned to `=0.4.0-alpha`** — keep the upstream pin.

## Conventions

Each new crate (`auki-identity-swift`, `auki-domain-swift`) carries the standard auki-sdk per-component files. The expanded crate (`auki-network-swift`) updates its existing files in place.

- `README.md`, `parking_lot.md`, `changelog.md`, `src/readme.md`, `src/sprint.md`.

Workspace `Cargo.toml` gains three new `members` entries adjacent to the existing `bindings/swift/auki-network-swift`. `bindings/swift/README.md` updates its table. `bindings/swift/parking_lot.md` updates per-package summaries. `bindings/changelog.md` and root `changelog.md` get one-liners per PR.

The three upstream crates (`auki-identity`, `auki-network`, `auki-domain`) get the `swift-bindings` feature added to their `Cargo.toml`, with `uniffi` as an optional dep. Their `parking_lot.md` / `changelog.md` record the feature addition. The upstream `src/sprint.md` files briefly note "swift binding annotations live here behind the `swift-bindings` feature."

Each binding crate gets its own `build-xcframework.sh`, copied from the existing `auki-network-swift/build-xcframework.sh` template (already validated).

Resolved parking-lot items get deleted entirely per the auki-sdk CLAUDE.md rule (one append-only exception); resolution is recorded in the changelog only.

## Testing

Host-only at Spec 1 stage, mirroring the Stage 1 precedent. Two test layers:

**Upstream crates with `swift-bindings` on.** Each upstream crate (`auki-identity`, `auki-network`, `auki-domain`) gains a test that confirms `cargo test --features swift-bindings -p <crate>` compiles and the existing test suite still passes. This validates the proc-macros don't break behavior. Most upstream tests are already feature-agnostic; the new tests are mostly "feature compiles" smoke tests.

**Binding crates.** `cargo test -p auki-identity-swift` / `-p auki-network-swift` / `-p auki-domain-swift` — verifies the scaffolding compiles and the custom-type registrations parse round-trip (PeerId String ↔ PeerId roundtrip, Multiaddr similar). The existing 4 Stage 1 tests in `auki-network-swift` migrate to use the upstream-annotated types instead of the hand-wrapped ones.

`build-xcframework.sh` against the three iOS targets per crate as the integration validation. Inspect the generated Swift surface (correct async signatures, correct types) per crate.

Cross-language wire-vector tests (Swift swift-protobuf decoding bytes encoded by Rust prost) are deferred to a follow-up that introduces the harness.

## Implementation staging

Three PRs against `auki-sdk` `develop`, each landing as one self-contained unit (upstream feature addition + binding crate change together so reviewers see the full picture for each layer):

1. **PR A — `auki-identity-swift` + upstream features on identity + network's PeerIdentity.** `PeerIdentity` and its `PEER_DERIVATION_LABEL` live in `crates/auki-network/src/lib.rs`, not in `crates/auki-identity`. PR A therefore touches *both* upstream crates: it adds the `swift-bindings` feature to `crates/auki-identity` (annotating `Wallet`) AND to `crates/auki-network` (annotating *only* `PeerIdentity` + adding the `PeerId` custom-type registration's upstream reachability — `NetworkRuntime` and the stream surface land in PR B, gated by the same feature). This makes PR A self-contained: it delivers a usable identity-only Swift binding ("load PeerIdentity from a Wallet seed") without waiting for B. Smallest of the three PRs; establishes the feature-flag-and-annotation pattern. The `swift-bindings` feature on `auki-network` lights up incrementally across PR A and PR B.

2. **PR B — `auki-network-swift` expansion + upstream feature.** Adds `swift-bindings` feature to `crates/auki-network`. Annotates `NetworkRuntime` + adds the `spawn_for_swift` shim. Annotates the stream-surface types. Annotates the existing Discovery surface (replaces Stage 1's hand-written shim). Adds custom-type registrations in the binding crate. **This is where the iOS libp2p cross-compile sharp edges land.** Largest PR.

3. **PR C — `auki-domain-swift` + upstream feature.** Adds `swift-bindings` feature to `crates/auki-domain`. Annotates `ClusterManager` + `ClusterTarget` + all the Record / Enum types + the provider/listener callback interfaces. Adds the membership-listener shim if needed. Depends on PRs A and B. Final piece.

Each PR follows the standard hierarchical changelog/parking_lot propagation. Each upstream `swift-bindings` feature addition is itself a small, audit-friendly change: a feature flag declaration, optional dependency on `uniffi`, and `cfg_attr` annotations on existing types — no behavioral change for non-Swift consumers.

## Open items

Filed in the new crates' parking lots during the relevant PR:

- **Single shared tokio runtime.** Same as before — three `OnceLock<Runtime>` is acceptable; consolidate later if profiling shows pain.
- **Where generated Swift + XCFramework artifacts live.** Stage 1's parking lot already holds this; Spec 2 settles it operationally for now via build-from-source.
- **Naming of the `swift-bindings` feature.** Confirm it doesn't collide with existing features on the three upstream crates (`auki-network` has `swarm`, `app_instance`, `discovery_client`; `auki-identity` may have similar). Should be conflict-free.
- **Shims that landed upstream gated by `swift-bindings`.** Per crate, list which shims exist (e.g. `NetworkRuntime::spawn_for_swift`, `ClusterManager::bootstrap_for_swift` if needed). Audit periodically — when upstream APIs evolve to be FFI-natively shaped, the shims can be removed.
- **Cross-language wire-vector test harness** for the stream surface. Deferred.
- **Async API shape vs. `-py`'s sync shape.** Already flagged for human confirmation in the existing Stage 1 parking lot; carries over.
- **`uniffi::custom_type!` reachability across binding crates.** Confirm `auki-network-swift`'s `PeerId`/`Multiaddr` registrations are inherited by `auki-domain-swift` without re-declaration. If not, declare them in both (with a comment cross-referencing) or factor into a tiny internal common crate (would re-open the "shared common crate?" question).
- **`open_stream` generic concretization.** Upstream `open_stream<T: prost::Message>` cannot be UniFFI-exported as-is (no generic FFI). Add per-payload-type concrete methods upstream (`open_audio_stream`, `open_pointcloud_stream`, etc.) under the `swift-bindings` feature; the alternative — exposing a generic `Vec<u8>` payload — loses type info Swift wants. Verify the upstream Rust supports this pattern cleanly before locking it in.

## Out of scope (explicit)

- All of Spec 2 (iosapp wiring): Keychain helper, `Bridge/AukiBridge.swift`, `scripts/sync-sdk.sh`, the proof-of-load UI, `DiscoveryGate`, `ContentView` replacement.
- CI for any binding crate, or for the new feature-on builds in upstream crates.
- A published SwiftPM-format package. Distribution stays build-from-source via the iosapp sync script (Spec 2).
- Sign-and-publish / TestFlight implications for iosapp. Spec 2 + iosapp's Q7 resolution own that.
- `auki-identity` surfaces beyond `Wallet` / `PeerIdentity`. `Signature`/`verify`/`CreationCert`/etc. stay unexposed.
- Restructuring upstream APIs to be more FFI-friendly. The `swift-bindings` feature is additive only; the rare necessary shim is upstream but in addition to, not in place of, existing methods.
- Cluster lifecycle features the Python equivalent does not yet ship.
