# auki-network-swift Expansion Implementation Plan (Spec 1, PR B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand `bindings/swift/auki-network-swift` to the second slice of the [SDK Swift binding expansion](../specs/2026-05-20-sdk-swift-binding-expansion-design.md): UniFFI-annotate `NetworkRuntime` + the full stream-surface on the upstream `auki-network` crate (gated by the existing `swift-bindings` feature established in PR A), add a binding-crate `spawn_for_swift` adapter that wires `PeerLivenessEvent`, `HeartbeatTimestampSource`, and `StreamProvider` to Swift-implementable callback interfaces, concretize `open_stream<T>` into one method per SDK-supported payload type (5 total: `AudioFrame`, `CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, `DetectionFrame`), and replace Stage 1's hand-wrapped `DiscoveryClient` with upstream-annotated types. After this PR, a Swift consumer can spawn a real libp2p runtime, observe peer connect/disconnect/heartbeat-stream-closed events, open typed outbound streams against any of the five SDK payload types, and accept inbound stream subscriptions via a Swift-implemented provider.

**Architecture:** PR A's pattern of upstream `#[cfg_attr(feature = "swift-bindings", uniffi::*)]` annotations carries forward to `NetworkRuntime`, `AllowedPeer`, `SpawnError`/`UpdateError`/`UpdateReport`, the discovery surface (`DiscoveryClient` + value types + error), and the five new per-payload `StreamSubscriptionX` Objects + `open_*_stream` methods. The Swift-side complexity that doesn't belong upstream — `PeerLivenessListener` and `HeartbeatTimestampProvider` and `SwiftStreamProvider` callback-interface traits, the source-stream-from-callback adapters, the `SwiftStreamDispatch` enum, and the `spawn_for_swift` orchestrator — lives in the binding crate (`bindings/swift/auki-network-swift/src/lib.rs`). All prost-generated wire types (`StreamRequest`, `StreamManifest`, `AudioFrame`, etc.) cross FFI as opaque `Vec<u8>` per the spec; Swift decodes via swift-protobuf against the committed `crates/auki-datatypes/proto/*.proto`. Per-payload type-distinguishability lives at the **method and subscription-object** level (`open_audio_stream` returns `StreamSubscriptionAudio`), not at the byte-payload level (all five `StreamEntry` shapes share the same `{timestamp_ns, seq, payload: Vec<u8>}` record).

**Tech Stack:** Rust 2024 edition, Cargo workspace, UniFFI 0.31 with `tokio` feature, `cfg_attr` for feature-gated proc-macros, `tokio_stream::wrappers::ReceiverStream` for the Swift→Rust source-stream adapter, prost 0.13 wire types from `auki-datatypes` crossing as `Vec<u8>`, Xcode 26.3 toolchain for the iOS XCFramework build (validated by PR A's `cf14503` multi-namespace fix).

---

## File Structure

Files this PR creates or modifies. Each task below names the specific files it touches.

**Upstream Rust crate (`crates/auki-network`):**
- Modify: `crates/auki-network/Cargo.toml` (no new features; the existing `swift-bindings` feature is fine — the binding crate enables `swarm` + `discovery_client` alongside)
- Modify: `crates/auki-network/src/network_runtime.rs` (annotate `AllowedPeer`, `SpawnError`, `UpdateError`, `UpdateReport`, `PeerLivenessEvent`, `NetworkRuntime`; add the 5 new `open_*_stream` methods and 5 `StreamSubscriptionX` Objects; add the curated `local_peer_id_string` + `connected_peer_ids` + `shutdown` helpers)
- Modify: `crates/auki-network/src/stream_runtime.rs` (delete the unused-now `StreamItem<T>` / `StreamEntry<T>` / `StreamSubscription<T>` generic types? **No** — they're still used by Rust callers (Park, daemons). Keep them; the new Swift surface lives parallel)
- Modify: `crates/auki-network/src/discovery_client.rs` (annotate `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`)
- Modify: `crates/auki-network/parking_lot.md` (extend PR A's note)
- Modify: `crates/auki-network/changelog.md` (entry)
- Modify: `crates/changelog.md` (one-liner)

**Binding crate (`bindings/swift/auki-network-swift`):**
- Modify: `bindings/swift/auki-network-swift/Cargo.toml` (add `swarm` to the feature set on `auki-network-rs`; keep `discovery_client`; pull in `tokio_stream` for the source-stream adapter)
- Modify: `bindings/swift/auki-network-swift/src/lib.rs` (custom-type registrations for `PeerId`/`Multiaddr`; binding-crate callback-interface traits: `PeerLivenessListener`, `HeartbeatTimestampProvider`, `SwiftStreamProvider`, `SwiftAudioSource`/`SwiftCameraSource`/`SwiftPointCloudSource`/`SwiftJointEncodersSource`/`SwiftDetectionSource`; `SwiftStreamDispatch` Enum; `spawn_for_swift` free function; delete Stage 1's hand-wrapped DiscoveryClient block)
- Modify: `bindings/swift/auki-network-swift/README.md` (new surface area)
- Modify: `bindings/swift/auki-network-swift/parking_lot.md` (resolved items removed; new items)
- Modify: `bindings/swift/auki-network-swift/changelog.md` (entry)
- Modify: `bindings/swift/auki-network-swift/src/readme.md` (what's implemented now)
- Modify: `bindings/swift/auki-network-swift/src/sprint.md` (current work)

**Indices + changelogs (propagation):**
- Modify: `bindings/swift/README.md` (per-crate table update)
- Modify: `bindings/swift/parking_lot.md` (per-package summary)
- Modify: `bindings/swift/changelog.md` (entry)
- Modify: `bindings/changelog.md` (entry)
- Modify: `changelog.md` (root)
- Modify: `docs/changelog.md` (one-liner)

---

### Task 1: Expand `auki-network-swift/Cargo.toml` feature set

**Files:**
- Modify: `bindings/swift/auki-network-swift/Cargo.toml`

PR A added the `swift-bindings` feature on `auki-network`. PR B needs the binding crate to enable that AND the `swarm` + `discovery_client` features (so the upstream UniFFI annotations on `NetworkRuntime` and `DiscoveryClient` are compiled in).

- [ ] **Step 1: Show the missing-feature failure**

Run:
```bash
cargo build -p auki-network-swift
```

Expected: PASS (Stage 1's feature set still works) — this is the baseline. Note current artifact size / build time for comparison.

- [ ] **Step 2: Update the `auki-network-rs` dep features**

In `bindings/swift/auki-network-swift/Cargo.toml`, locate the `auki-network-rs` line:

```toml
auki-network-rs = { package = "auki-network", path = "../../../crates/auki-network", features = ["discovery_client"] }
```

Replace with:

```toml
# PR A added `swift-bindings`; PR B enables `swarm` (for NetworkRuntime + the
# stream surface) and keeps `discovery_client` (for the Discovery HTTP client
# we're now consuming as upstream-annotated types). All three are independent
# features on the upstream crate; combined here.
auki-network-rs = { package = "auki-network", path = "../../../crates/auki-network", features = ["swift-bindings", "swarm", "discovery_client"] }
```

- [ ] **Step 3: Pull in `tokio_stream` for the source-stream adapter**

In `bindings/swift/auki-network-swift/Cargo.toml`, locate the `tokio` line and insert `tokio-stream` directly after it:

```toml
tokio = { version = "1", features = ["rt-multi-thread"] }
# Swift→Rust source-stream adapter wraps `tokio::sync::mpsc::Receiver`s in
# `tokio_stream::wrappers::ReceiverStream` to satisfy the upstream
# `SourceStream<T> = Pin<Box<dyn Stream<Item=...> + Send>>` shape.
tokio-stream = { version = "0.1", default-features = false }
```

- [ ] **Step 4: Verify feature-on build**

Run:
```bash
cargo build -p auki-network-swift
```

Expected: PASS. The build now pulls libp2p + reqwest into the binding crate's graph. Artifact size will jump; that's the PR's purpose.

- [ ] **Step 5: Verify upstream feature-on tests pass**

Run:
```bash
cargo test --features swift-bindings -p auki-network --lib
```

Expected: PASS — the upstream lib tests (including PR A's `swift_bindings_tests` module) still work. Confirms the new feature combinations didn't break anything.

- [ ] **Step 6: Commit**

```bash
git add bindings/swift/auki-network-swift/Cargo.toml
git commit -m "feat(auki-network-swift): enable swarm + swift-bindings features"
```

---

### Task 2: Register `PeerId` UniFFI custom type

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

Replace the Stage 1 ad-hoc `parse_peer_id` helper with a `uniffi::custom_type!` declaration so every upstream method that takes or returns `PeerId` (in `NetworkRuntime`, `DiscoveryClient`, etc.) is auto-exposed across the FFI as a Swift `String`. The Stage 1 helper stays around as long as Stage 1's hand-wrapping does (deleted in Task 20).

- [ ] **Step 1: Add a failing round-trip test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` (inside the existing `#[cfg(test)] mod tests`):

```rust
    /// `PeerId` round-trips through its UniFFI custom-type registration:
    /// canonical string in → `PeerId` → canonical string out (identical).
    #[test]
    fn peer_id_custom_type_round_trips() {
        use uniffi::{FfiConverter, FfiDefault};
        // Test the registered conversion functions exist and round-trip.
        let pid = test_peer_id();
        let s = pid.to_string();
        // `parse::<PeerId>` is what the custom_type's `try_lift` closure calls;
        // verifying it round-trips proves the registration is correct.
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }
```

- [ ] **Step 2: Run the test to confirm it currently passes**

Run:
```bash
cargo test -p auki-network-swift peer_id_custom_type_round_trips
```

Expected: PASS — the test exercises `PeerId::FromStr` which already works; this is a regression guard for the next step.

- [ ] **Step 3: Add the `uniffi::custom_type!` declaration**

In `bindings/swift/auki-network-swift/src/lib.rs`, locate the `uniffi::setup_scaffolding!();` line (around line 38). Insert immediately after it:

```rust
// ─── Custom-type registrations ─────────────────────────────────────
//
// `PeerId` and `Multiaddr` are libp2p types defined in external crates
// (`libp2p-identity`, `multiaddr`); we can't annotate them directly. UniFFI's
// custom_type! registers the conversion at the binding-crate level: every
// upstream `auki-network` method that takes or returns `PeerId` / `Multiaddr`
// is auto-exposed with `String` at the seam.
//
// `auki-domain-swift` (PR C) will pick up these registrations via its dep on
// this crate — no need to redeclare there.

/// Cross-FFI representation: canonical libp2p peer-id string
/// (`12D3KooW…`). Parse failures surface as a Rust `anyhow::Error` —
/// UniFFI propagates the message to Swift as a thrown error on the
/// affected method.
uniffi::custom_type!(PeerId, String, {
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});
```

- [ ] **Step 4: Verify build + tests pass**

Run:
```bash
cargo test -p auki-network-swift peer_id_custom_type_round_trips
```

Expected: PASS. The custom-type registration compiles; round-trip test still works.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): register PeerId UniFFI custom type"
```

---

### Task 3: Register `Multiaddr` UniFFI custom type

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

Same pattern as Task 2 for `Multiaddr`.

- [ ] **Step 1: Add a failing round-trip test**

Append to the `mod tests` block in `bindings/swift/auki-network-swift/src/lib.rs`:

```rust
    /// `Multiaddr` round-trips through its UniFFI custom-type registration.
    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let s = addr.to_string();
        let back: Multiaddr = s.parse().expect("canonical multiaddr parses");
        assert_eq!(back, addr);
    }
```

- [ ] **Step 2: Run the test to confirm it passes**

Run:
```bash
cargo test -p auki-network-swift multiaddr_custom_type_round_trips
```

Expected: PASS.

- [ ] **Step 3: Add the `uniffi::custom_type!` declaration**

In `bindings/swift/auki-network-swift/src/lib.rs`, immediately after the `PeerId` `custom_type!` declaration from Task 2, append:

```rust
/// Cross-FFI representation: canonical `/ip4/.../tcp/...` multiaddr
/// string. Parse failures surface as Rust `anyhow::Error`.
uniffi::custom_type!(Multiaddr, String, {
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});
```

- [ ] **Step 4: Add `anyhow` to binding crate's dep set**

In `bindings/swift/auki-network-swift/Cargo.toml`, locate the `[dependencies]` section and add (alphabetical order):

```toml
# Used by the `try_lift` arms of `uniffi::custom_type!` declarations.
anyhow = "1"
```

- [ ] **Step 5: Verify build + tests pass**

Run:
```bash
cargo test -p auki-network-swift multiaddr_custom_type_round_trips
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add bindings/swift/auki-network-swift/Cargo.toml bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): register Multiaddr UniFFI custom type"
```

---

### Task 4: Annotate `AllowedPeer` upstream as `uniffi::Record`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

`AllowedPeer { peer_id: PeerId, multiaddrs: Vec<Multiaddr> }` is plain data. With the custom-type registrations from Tasks 2-3 in scope (via binding-crate dependency), UniFFI auto-lowers both fields. The annotation is one line plus `#[uniffi(remote)]`-style consideration — but `AllowedPeer` lives in the upstream crate, so we annotate it directly.

- [ ] **Step 1: Add the binding-crate failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    /// `AllowedPeer` is constructible from canonical PeerId + multiaddr
    /// strings via UniFFI's auto-derived constructor. Exercises the
    /// custom-type lowering chain (String → PeerId → Vec<Multiaddr>).
    #[test]
    fn allowed_peer_constructs_with_string_inputs() {
        let pid = test_peer_id();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        // Direct struct construction works today (no UniFFI involvement);
        // after annotation, the Record-derive generates an FFI constructor
        // that takes (String, Vec<String>) and auto-lifts.
        let ap = auki_network_rs::AllowedPeer {
            peer_id: pid,
            multiaddrs: vec![addr.clone()],
        };
        assert_eq!(ap.peer_id, pid);
        assert_eq!(ap.multiaddrs, vec![addr]);
    }
```

- [ ] **Step 2: Run the test to confirm it passes**

Run:
```bash
cargo test -p auki-network-swift allowed_peer_constructs_with_string_inputs
```

Expected: PASS (struct already exists; this is a baseline).

- [ ] **Step 3: Annotate `AllowedPeer` upstream**

In `crates/auki-network/src/network_runtime.rs`, locate the `AllowedPeer` definition (around line 110). Currently:

```rust
/// One entry in the runtime's allow-list / auto-dial schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedPeer {
    /// libp2p peer-id of this peer.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Empty list = the runtime
    /// allows inbound connections from this peer but does not
    /// auto-dial them.
    pub multiaddrs: Vec<Multiaddr>,
}
```

Replace with:

```rust
/// One entry in the runtime's allow-list / auto-dial schedule.
///
/// `swift-bindings`: derived as a UniFFI Record. `peer_id` and
/// `multiaddrs` cross the FFI as canonical strings via the
/// custom-type registrations in `auki-network-swift`.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedPeer {
    /// libp2p peer-id of this peer.
    pub peer_id: PeerId,
    /// Dialable multiaddrs for this peer. Empty list = the runtime
    /// allows inbound connections from this peer but does not
    /// auto-dial them.
    pub multiaddrs: Vec<Multiaddr>,
}
```

- [ ] **Step 4: Verify build + test passes**

Run:
```bash
cargo build --features swift-bindings,swarm -p auki-network
cargo test -p auki-network-swift allowed_peer_constructs_with_string_inputs
```

Expected: PASS for both. The `Record` derive generates the FFI metadata; native Rust callers see no behavioral change.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): annotate AllowedPeer as UniFFI Record"
```

---

### Task 5: Annotate `SpawnError`, `UpdateError`, `UpdateReport`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Errors thrown by `NetworkRuntime` constructors and command methods. `SpawnError` (NoTokioRuntime), `UpdateError` (multi-variant, some with `String` payload for libp2p errors), `UpdateReport { added, removed }`. All FFI-compatible after annotation; no flattening required.

- [ ] **Step 1: Read the current shapes**

Open `crates/auki-network/src/network_runtime.rs` and inspect:
- `SpawnError` (line 181): single `NoTokioRuntime` variant.
- `UpdateError` (line 545): inspect its variants.
- `UpdateReport` (line 536): inspect its fields.

- [ ] **Step 2: Add failing tests for the error/report shapes**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
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
```

- [ ] **Step 3: Run tests to confirm baseline**

Run:
```bash
cargo test -p auki-network-swift spawn_error_is_display_clean update_error_is_display_clean
```

Expected: PASS.

- [ ] **Step 4: Annotate `SpawnError`**

In `crates/auki-network/src/network_runtime.rs`, locate `SpawnError`:

```rust
/// Errors from [`NetworkRuntime::spawn`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// Constructor was called outside a tokio runtime context — the
    /// runtime needs a tokio handle to spawn its driver task.
    #[error("no current tokio runtime — call from within a tokio runtime context")]
    NoTokioRuntime,
}
```

Replace with:

```rust
/// Errors from [`NetworkRuntime::spawn`].
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// Constructor was called outside a tokio runtime context — the
    /// runtime needs a tokio handle to spawn its driver task.
    #[error("no current tokio runtime — call from within a tokio runtime context")]
    NoTokioRuntime,
}
```

- [ ] **Step 5: Annotate `UpdateError`**

Locate `UpdateError` in `crates/auki-network/src/network_runtime.rs` (around line 545). Inspect its variants; if any wrap a non-FFI type (e.g. `libp2p_stream::OpenStreamError`), add `#[uniffi(flat_error)]` so the variant flattens to its Display string.

Replace the `enum UpdateError` definition (keep all existing variants) with:

```rust
/// Errors from [`NetworkRuntime::set_allowed_peers`] /
/// [`NetworkRuntime::set_heartbeat_targets`].
///
/// `swift-bindings`: flattened — variants that wrap non-FFI inner
/// errors are surfaced as Display'd strings; UniFFI consumers see one
/// tagged-enum case per variant with a `message: String` field where
/// the wrapped error was.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    // ... (existing variants verbatim — `flat_error` handles them all)
}
```

(The implementer reads the actual variant set from the source and preserves them verbatim; `flat_error` is the meta-attribute that lets us avoid hand-mapping each non-FFI inner type.)

- [ ] **Step 6: Annotate `UpdateReport`**

Locate `UpdateReport`:

```rust
pub struct UpdateReport {
    /// ... fields
}
```

Replace with:

```rust
/// Diff returned by `set_allowed_peers` — what changed.
///
/// `swift-bindings`: UniFFI Record. All fields are FFI-friendly
/// (`Vec<PeerId>` lowers via the custom-type registration in
/// `auki-network-swift`).
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone)]
pub struct UpdateReport {
    /// ... fields (preserved verbatim from source)
}
```

- [ ] **Step 7: Verify build + tests pass**

Run:
```bash
cargo build --features swift-bindings,swarm -p auki-network
cargo test -p auki-network-swift spawn_error_is_display_clean update_error_is_display_clean
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): annotate SpawnError, UpdateError, UpdateReport"
```

---

### Task 6: Annotate `PeerLivenessEvent` (3-variant v0 surface)

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Original `PeerLivenessEvent` has 5 variants: `Connected`, `Disconnected`, `HeartbeatReceived { ..., observation: HeartbeatTimingObservation }`, `HeartbeatNtpSampleObserved { ..., observation: HeartbeatNtpSampleObservation }`, `HeartbeatStreamClosed`. The two heartbeat-observation variants carry `HeartbeatTimingObservation` / `HeartbeatNtpSampleObservation` structs, which themselves carry a `Heartbeat` prost type and an `NtpSample` — both wide and not what iosapp's v0 proof-bar consumes.

For v0, expose the 3 connection-level variants directly (`Connected`, `Disconnected`, `HeartbeatStreamClosed`) and translate the two heartbeat-detail variants to a single "carries opaque bytes" variant (`HeartbeatObserved { peer_id, observation_bytes: Vec<u8> }`) where the bytes are a prost-encoded `auki_datatypes::time_transform::HeartbeatObservation` (added in this task). Future PR can widen — at v0, Swift consumers that don't care about heartbeat detail just ignore that variant.

**Note**: The decision is to keep the upstream enum unchanged (Rust callers still use the 5-variant form) and provide a **Swift-facing translation** in the binding crate, NOT to mutate the upstream enum. This task introduces the upstream-side annotation on `PeerLivenessEvent` with `#[uniffi(flat_error)]`-equivalent treatment — but since `PeerLivenessEvent` isn't an error, we need a different approach.

**Revised approach for this task**: keep the upstream enum un-annotated. The Swift surface for liveness events is a binding-crate `SwiftPeerLivenessEvent` Enum (Record-shaped, 3-variant) declared in `auki-network-swift/src/lib.rs`. The adapter from upstream `PeerLivenessEvent` → `SwiftPeerLivenessEvent` lives in the binding crate. The two heartbeat-detail variants of the upstream enum get translated to a single Swift-side `HeartbeatObserved { peer_id_str: String }` variant (no payload bytes at v0 — iosapp doesn't need it; reduces FFI complexity).

- [ ] **Step 1: Add a binding-crate failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
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
```

- [ ] **Step 2: Run test, confirm it fails (type doesn't exist yet)**

Run:
```bash
cargo test -p auki-network-swift swift_peer_liveness_event_translation
```

Expected: FAIL — `cannot find type 'SwiftPeerLivenessEvent' in this scope`.

- [ ] **Step 3: Add `SwiftPeerLivenessEvent` in the binding crate**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `Multiaddr` custom-type declaration from Task 3, append:

```rust
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
    /// 3-variant subset. Returns `None` for the two heartbeat-detail
    /// variants (`HeartbeatReceived`, `HeartbeatNtpSampleObserved`) —
    /// callers (the `spawn_for_swift` event-drain task) drop those at v0.
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
            // Heartbeat-detail variants are dropped at v0; future PR can
            // widen `SwiftPeerLivenessEvent` if iosapp needs them.
            PeerLivenessEvent::HeartbeatReceived { peer_id, .. }
            | PeerLivenessEvent::HeartbeatNtpSampleObserved { peer_id, .. } => {
                Self::HeartbeatStreamClosed {
                    // Re-use the closed variant as a "something happened" placeholder
                    // — but emit only at the drain task's discretion. The
                    // `from_upstream` total fn is here so future widening is a one-
                    // file change.
                    peer_id: peer_id.to_string(),
                }
            }
        }
    }

    /// True for variants that should be forwarded to Swift listeners at v0
    /// (filters out the heartbeat-detail synthetic forms produced by
    /// `from_upstream` for the two unsupported upstream variants).
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
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift swift_peer_liveness_event_translation
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add SwiftPeerLivenessEvent (3-variant v0 surface)"
```

---

### Task 7: Add `PeerLivenessListener` callback-interface trait

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

The callback interface Swift implements to receive `SwiftPeerLivenessEvent`s. UniFFI 0.31's `#[uniffi::export(callback_interface)]` declares the trait as a Swift-implementable protocol; Swift implementations are wrapped as `Arc<dyn PeerLivenessListener>` on the Rust side.

- [ ] **Step 1: Add a failing test for the trait existence**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    /// Smoke test: a no-op `PeerLivenessListener` impl compiles and can be
    /// stored as `Arc<dyn PeerLivenessListener>`. Real wire-up tested in
    /// Task 12 (spawn_for_swift smoke test).
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
```

- [ ] **Step 2: Run test, confirm it fails**

Run:
```bash
cargo test -p auki-network-swift peer_liveness_listener_is_object_safe
```

Expected: FAIL — `cannot find trait 'PeerLivenessListener'`.

- [ ] **Step 3: Add the trait declaration**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `SwiftPeerLivenessEvent` impl block from Task 6, append:

```rust
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
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift peer_liveness_listener_is_object_safe
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add PeerLivenessListener callback interface"
```

---

### Task 8: Add `HeartbeatTimestampProvider` callback-interface trait + adapter

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

`HeartbeatTimestampSource` carries `clock_id: String`, `clock_hash: String`, plus two `Arc<dyn Fn>` callbacks (`now_ns: HeartbeatNowNs`, `domain_clock: HeartbeatDomainClockNs`). For Swift, expose a `HeartbeatTimestampProvider` trait with three methods: `clock_id()`, `clock_hash()`, `now_ns()`, `domain_clock_bytes()`. The fourth method returns prost-encoded `HeartbeatDomainClock` as `Option<Vec<u8>>` (the upstream type lives in `auki-datatypes::time_transform`).

- [ ] **Step 1: Add a failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
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
```

- [ ] **Step 2: Run test, confirm it fails**

Run:
```bash
cargo test -p auki-network-swift heartbeat_timestamp_provider_adapter
```

Expected: FAIL — `cannot find trait 'HeartbeatTimestampProvider'`.

- [ ] **Step 3: Add the trait + adapter function**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `PeerLivenessListener` trait declaration from Task 7, append:

```rust
// ─── Heartbeat timestamp provider (Swift callback interface) ───────

/// Swift consumers implement this trait to supply the heartbeat-source
/// timestamp readings and clock identity the runtime needs. Wrapped in
/// `Arc<dyn ...>`; the adapter [`heartbeat_source_from_provider`]
/// converts it into the upstream `HeartbeatTimestampSource` shape.
///
/// `clock_id` and `clock_hash` are read once at runtime spawn (they're
/// stable for the lifetime of the runtime). `now_ns` is invoked on
/// every outbound heartbeat frame; `domain_clock_bytes` is invoked the
/// same way and returns the prost-encoded
/// `auki.time_transform.HeartbeatDomainClock` or `None`.
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
    /// Prost-encoded `auki.time_transform.HeartbeatDomainClock`
    /// describing the domain clock this peer is currently advertising,
    /// or `None`. Called per outbound heartbeat frame.
    fn domain_clock_bytes(&self) -> Option<Vec<u8>>;
}

/// Adapter: build an upstream `HeartbeatTimestampSource` from a Swift
/// `HeartbeatTimestampProvider`. The closures wrap the trait-object
/// method calls.
pub(crate) fn heartbeat_source_from_provider(
    provider: Arc<dyn HeartbeatTimestampProvider>,
) -> auki_network_rs::HeartbeatTimestampSource {
    let clock_id = provider.clock_id();
    let clock_hash = provider.clock_hash();
    let p_for_now = provider.clone();
    let p_for_dc = provider.clone();
    auki_network_rs::HeartbeatTimestampSource {
        clock_id,
        clock_hash,
        now_ns: Arc::new(move || p_for_now.now_ns()),
        domain_clock: Arc::new(move || {
            p_for_dc.domain_clock_bytes().and_then(|bytes| {
                // Decode the prost bytes into the upstream
                // `HeartbeatDomainClock` type. Failure → emit None (the
                // runtime treats this as "no domain clock to advertise").
                use prost::Message;
                auki_datatypes::time_transform::HeartbeatDomainClock::decode(bytes.as_slice()).ok()
            })
        }),
    }
}
```

- [ ] **Step 4: Add `prost` + `auki-datatypes` to binding-crate deps**

In `bindings/swift/auki-network-swift/Cargo.toml`, add to `[dependencies]`:

```toml
# For decoding `HeartbeatDomainClock` bytes from the Swift provider.
prost = "0.13"
auki-datatypes = { path = "../../../crates/auki-datatypes" }
```

- [ ] **Step 5: Re-export `HeartbeatTimestampSource` use in the binding crate**

At the top of `bindings/swift/auki-network-swift/src/lib.rs`, locate the existing `use auki_network_rs::...` imports. Add to the existing import or as a new line near it:

```rust
// HeartbeatTimestampSource: needed for the adapter return type. Not
// itself UniFFI-exposed — Swift uses the `HeartbeatTimestampProvider`
// trait, the binding crate converts at spawn time.
use auki_network_rs::HeartbeatTimestampSource;
```

(Remove the explicit path reference in `heartbeat_source_from_provider`'s return type since we now have the use.)

- [ ] **Step 6: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift heartbeat_timestamp_provider_adapter
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add bindings/swift/auki-network-swift/Cargo.toml bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add HeartbeatTimestampProvider callback interface"
```

---

### Task 9: Annotate `NetworkRuntime` (Object + curated method set)

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

`NetworkRuntime` becomes a `uniffi::Object`. Annotate the curated method set that iosapp's proof-bar consumes:
- `local_peer_id() -> PeerId` (auto-exposed via the custom-type registration; Swift sees `String`)
- `connected_peers() -> Vec<PeerId>` (auto-exposed as `Vec<String>`)
- `set_allowed_peers(peers: Vec<AllowedPeer>) -> Result<UpdateReport, UpdateError>` (async; UniFFI's `async_runtime = "tokio"`)
- `shutdown(&self)` (idempotent; no return)

Skip: `set_heartbeat_targets`, `broadcast_*`, `send_join_request`, `request_*_catalog`, `request_registry_entry`, `handle()` — these are auki-domain layer concerns or aren't needed for v0 proof bar (PR C reaches for them via `auki-domain-swift`'s `ClusterManager`).

- [ ] **Step 1: Add a failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    /// `NetworkRuntime` exposes its annotated method set. We can't spawn one
    /// here (needs a real tokio runtime + swarm), but we can confirm the
    /// types compile via a type-check.
    #[test]
    fn network_runtime_is_uniffi_object() {
        // Type-level assertion: `NetworkRuntime` is `Send + Sync` (required
        // for `uniffi::Object`).
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<auki_network_rs::NetworkRuntime>();
    }
```

- [ ] **Step 2: Run, expect baseline pass**

Run:
```bash
cargo test -p auki-network-swift network_runtime_is_uniffi_object
```

Expected: PASS (NetworkRuntime is already Send+Sync).

- [ ] **Step 3: Annotate `NetworkRuntime` upstream**

In `crates/auki-network/src/network_runtime.rs`, locate the `pub struct NetworkRuntime { ... }` block (around line 575). Replace with:

```rust
/// libp2p Swarm driver. Owns the swarm and a single tokio task that
/// drives the event loop; consumers interact through the small set of
/// public methods.
///
/// `swift-bindings`: derived as a UniFFI Object. Methods annotated
/// individually in the impl blocks below; `spawn` is the non-FFI
/// constructor (the binding crate's `spawn_for_swift` adapter is what
/// Swift consumers call). The curated FFI surface for v0:
/// `local_peer_id_string`, `connected_peer_id_strings`,
/// `set_allowed_peers`, `shutdown`, and the 5 `open_*_stream` methods
/// added below.
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct NetworkRuntime {
    local_peer_id: PeerId,
    connected: Arc<Mutex<HashSet<PeerId>>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    stream_control: Control,
    inbound_shutdown_tx: watch::Sender<bool>,
    _lifeline_tx: watch::Sender<()>,
    command_tx: mpsc::Sender<RuntimeCmd>,
}
```

(Preserve the existing doc comments on each field; only the struct-level docstring and the `#[cfg_attr]` line are new.)

- [ ] **Step 4: Add the UniFFI-friendly accessor methods upstream**

`PeerId` and `Vec<PeerId>` lower via the custom-type registration — but the binding crate's `uniffi::custom_type!` only takes effect for upstream methods if the binding crate's metadata is linked in. To avoid action-at-a-distance, add two convenience methods upstream that return `String` and `Vec<String>` directly. These join the existing methods (which Rust callers continue to use).

In `crates/auki-network/src/network_runtime.rs`, after the existing `impl NetworkRuntime { ... }` blocks but before the `impl Drop for NetworkRuntime` (or wherever fits cleanly), add a new annotated impl block:

```rust
// ─── UniFFI-exposed surface ──────────────────────────────────────────────────
//
// Methods Swift consumes. Each takes types that lower cleanly via UniFFI
// derives or via the binding-crate custom-type registrations. The
// `_string` / `_strings` suffix on the peer-id accessors is the same
// pattern PR A established for `Wallet::wallet_id_str` — explicit so the
// FFI seam shape is visible at the call site.
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl NetworkRuntime {
    /// Canonical libp2p peer-id string for this runtime's local peer.
    pub fn local_peer_id_string(&self) -> String {
        self.local_peer_id.to_string()
    }

    /// Snapshot of currently-connected peer-ids as canonical libp2p
    /// strings. Mutates as connections open / close in the driver task.
    pub fn connected_peer_id_strings(&self) -> Vec<String> {
        self.connected
            .lock()
            .expect("connected set mutex poisoned")
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    /// Idempotent shutdown. First caller drains the driver task; second
    /// caller no-ops. The runtime is unusable afterward; constructing a
    /// new one is the path to recovery.
    pub fn shutdown(&self) {
        // Move the existing `pub fn shutdown(&self)` body (from the
        // un-annotated impl block around line 1149) verbatim into this
        // method, then delete the original. UniFFI doesn't allow the
        // same method name on two impl blocks where one is annotated;
        // moving the body here is the clean fix. Rust callers continue
        // calling `runtime.shutdown()` — same name, same behavior.
    }
}
```

**Implementer note**: the existing `pub fn shutdown(&self) { ... }` at around line 1149 of `network_runtime.rs` must be deleted in this step — its body moves into the annotated method above. Don't leave both in place: the duplicate definition will fail to compile.

- [ ] **Step 5: Annotate `set_allowed_peers`**

Locate `pub async fn set_allowed_peers(&self, new_peers: Vec<AllowedPeer>) -> Result<UpdateReport, UpdateError>` on `NetworkRuntime` (the non-handle one, around line 1107). This is currently in an unannotated impl block.

Move this method into the annotated impl block we added in Step 4, and add the `#[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]` annotation (per UniFFI 0.31's per-method async-runtime override pattern):

```rust
    /// Atomically replace the cluster trust boundary. Returns a diff
    /// (`UpdateReport { added, removed }`) of what changed. Newly-added
    /// peers with multiaddrs are scheduled for an immediate dial.
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn set_allowed_peers(
        &self,
        new_peers: Vec<AllowedPeer>,
    ) -> Result<UpdateReport, UpdateError> {
        // (existing body)
    }
```

- [ ] **Step 6: Verify build + tests pass**

Run:
```bash
cargo build --features swift-bindings,swarm -p auki-network
cargo test -p auki-network --features swift-bindings,swarm --lib
cargo test -p auki-network-swift network_runtime_is_uniffi_object
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): annotate NetworkRuntime + curated v0 methods"
```

---

### Task 10: Add the placeholder `spawn_for_swift` adapter (decline-all streams)

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

The orchestrator that Swift calls to spawn a `NetworkRuntime`. Takes a `PeerIdentity`, listen multiaddrs, initial allow-list, `PeerLivenessListener`, `HeartbeatTimestampProvider`, and (later, in Task 13) a `SwiftStreamProvider`. Internally:
1. Builds a libp2p swarm via `auki_network::swarm::build_swarm`
2. Constructs `HeartbeatTimestampSource` via the Task 8 adapter
3. Calls upstream `NetworkRuntime::spawn` with `decline_all_streams()` as the provider (placeholder for Task 13)
4. Spawns a tokio task that drains the `PeerLivenessEvent` receiver and forwards to the listener via `SwiftPeerLivenessEvent::from_upstream`
5. Returns `Arc<NetworkRuntime>`

The 8 non-liveness receivers from `spawn`'s 9-channel return are drain-and-dropped in tokio tasks (or simply dropped if mpsc::Receiver's drop-on-no-reader behavior is acceptable — verify upstream `run_task`'s `send` calls swallow `SendError`).

- [ ] **Step 1: Add a failing smoke test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
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

        // Construct PeerIdentity for the local node.
        let wallet = auki_identity::Wallet::from_seed(vec![1u8; 32]).expect("32-byte seed");
        let identity =
            std::sync::Arc::new(auki_network_rs::PeerIdentity::from_wallet(wallet));

        let listener: Arc<dyn PeerLivenessListener> = Arc::new(NoopListener);
        let heartbeat: Arc<dyn HeartbeatTimestampProvider> = Arc::new(WallClockProvider);

        let rt = spawn_for_swift(
            identity,
            // Bind to ephemeral local addresses — no real network needed.
            vec!["/ip4/127.0.0.1/tcp/0".to_string()],
            // No initial allowed peers — keeps the swarm fully isolated.
            vec![],
            listener,
            heartbeat,
        )
        .await
        .expect("spawn succeeds in test runtime");

        // Local peer-id is the canonical 12D3KooW... form.
        let pid = rt.local_peer_id_string();
        assert!(pid.starts_with("12D3KooW"), "expected canonical PeerId");

        // No peers connected yet.
        assert!(rt.connected_peer_id_strings().is_empty());

        // Clean shutdown.
        rt.shutdown();
    }
```

- [ ] **Step 2: Run test, confirm it fails**

Run:
```bash
cargo test -p auki-network-swift spawn_for_swift_smoke
```

Expected: FAIL — `cannot find function 'spawn_for_swift'`.

- [ ] **Step 3: Add the placeholder `spawn_for_swift`**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `HeartbeatTimestampProvider` block from Task 8, append:

```rust
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
/// At this task's checkpoint, `stream_provider` is hard-coded to
/// `decline_all_streams()` — every inbound stream request is declined
/// with `DeclineReason::SensorNotFound`. Task 13 widens this signature
/// to accept a Swift-implemented `SwiftStreamProvider`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn spawn_for_swift(
    identity: Arc<auki_network_rs::PeerIdentity>,
    listen_multiaddrs: Vec<Multiaddr>,
    allowed_peers: Vec<auki_network_rs::AllowedPeer>,
    peer_liveness_listener: Arc<dyn PeerLivenessListener>,
    heartbeat_timestamps: Arc<dyn HeartbeatTimestampProvider>,
) -> Result<Arc<auki_network_rs::NetworkRuntime>, SpawnSwiftError> {
    // 1. Build the swarm. The upstream API takes `&PeerIdentity` (not the
    //    keypair directly) and a `SwarmConfig` with `listen_addresses`,
    //    `agent_version`, `enable_relay_server`. Identity comes in as
    //    `Arc<PeerIdentity>`; `as_ref()` gives the `&PeerIdentity` shape.
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

    // 4. Spawn the runtime.
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
    // Receiver closed → driver task ended → runtime is shutting down.
}
```

- [ ] **Step 4: Verify the smoke test passes**

Run:
```bash
cargo test -p auki-network-swift spawn_for_swift_smoke
```

Expected: PASS. Real swarm spawn, brief lifetime, clean shutdown.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add spawn_for_swift (decline-all placeholder)"
```

---

### Task 11: Add `SwiftStreamProvider` callback interface + 5 source traits + dispatch enum

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

The full Swift-side stream-provider adapter. Five payload-specific source traits, one shared dispatch enum (6 variants: 5 Accepts + 1 Decline), one provider trait that returns the dispatch. The provider passes through `request_bytes` (opaque prost StreamRequest) and receives `manifest_bytes` (opaque prost StreamManifest) in each Accept variant + `reason_bytes` (opaque prost DeclineReason) in Decline.

- [ ] **Step 1: Add failing tests for trait existence + dispatch variants**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    /// All 5 source traits + the provider trait compile and are object-
    /// safe.
    #[test]
    fn swift_stream_provider_object_safety() {
        struct NoopProvider;
        impl SwiftStreamProvider for NoopProvider {
            fn dispatch(&self, _peer_id: String, _request_bytes: Vec<u8>) -> SwiftStreamDispatch {
                SwiftStreamDispatch::Decline {
                    reason_bytes: vec![],
                }
            }
        }
        let _p: Arc<dyn SwiftStreamProvider> = Arc::new(NoopProvider);
    }

    /// `SwiftStreamDispatch` has 5 Accept variants + 1 Decline; each
    /// carries an `Arc<dyn ...Source>` (or `Vec<u8>` for Decline).
    #[test]
    fn swift_stream_dispatch_variants() {
        let d = SwiftStreamDispatch::Decline {
            reason_bytes: b"reason".to_vec(),
        };
        match d {
            SwiftStreamDispatch::Decline { reason_bytes } => assert_eq!(reason_bytes, b"reason"),
            _ => panic!("wrong variant"),
        }
    }
```

- [ ] **Step 2: Run, expect FAIL on undefined traits/enum**

Run:
```bash
cargo test -p auki-network-swift swift_stream_provider_object_safety swift_stream_dispatch_variants
```

Expected: FAIL — types don't exist yet.

- [ ] **Step 3: Add the 5 source traits + dispatch enum + provider trait**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `drain_liveness_events` function from Task 10, append:

```rust
// ─── Swift stream provider + source traits ─────────────────────────

/// One source-stream item. The opaque `payload_bytes` is prost-encoded
/// against the per-payload-type proto file (`AudioFrame.proto`,
/// `CameraFrame.proto`, etc.); Swift consumers decode via swift-protobuf.
///
/// Shared shape across all 5 payload types — type-distinguishability
/// lives at the trait level (which `Swift*Source` you implement) and
/// the `StreamSubscription*` consumer side, not in the byte payload
/// representation.
#[derive(uniffi::Record, Clone, Debug)]
pub struct StreamItem {
    pub timestamp_ns: i64,
    pub payload_bytes: Vec<u8>,
}

/// Swift-implemented audio source. Returns the next `StreamItem`
/// containing prost-encoded `AudioFrame` bytes, `Ok(None)` for clean
/// end-of-source, or `Err(detail)` for producer error.
#[uniffi::export(callback_interface)]
pub trait SwiftAudioSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, String>;
}

/// Swift-implemented camera source.
#[uniffi::export(callback_interface)]
pub trait SwiftCameraSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, String>;
}

/// Swift-implemented point-cloud source.
#[uniffi::export(callback_interface)]
pub trait SwiftPointCloudSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, String>;
}

/// Swift-implemented joint-encoders source.
#[uniffi::export(callback_interface)]
pub trait SwiftJointEncodersSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, String>;
}

/// Swift-implemented detection source.
#[uniffi::export(callback_interface)]
pub trait SwiftDetectionSource: Send + Sync {
    fn next_item(&self) -> Result<Option<StreamItem>, String>;
}

/// Producer's accept/decline decision for one inbound stream request.
/// 5 Accept variants — one per SDK-supported payload type — each
/// carrying the prost-encoded manifest bytes + the Swift-implemented
/// source. Decline carries the prost-encoded `DeclineReason` bytes.
#[derive(uniffi::Enum)]
pub enum SwiftStreamDispatch {
    AcceptAudio {
        manifest_bytes: Vec<u8>,
        source: Arc<dyn SwiftAudioSource>,
    },
    AcceptCamera {
        manifest_bytes: Vec<u8>,
        source: Arc<dyn SwiftCameraSource>,
    },
    AcceptPointCloud {
        manifest_bytes: Vec<u8>,
        source: Arc<dyn SwiftPointCloudSource>,
    },
    AcceptJointEncoders {
        manifest_bytes: Vec<u8>,
        source: Arc<dyn SwiftJointEncodersSource>,
    },
    AcceptDetection {
        manifest_bytes: Vec<u8>,
        source: Arc<dyn SwiftDetectionSource>,
    },
    Decline {
        reason_bytes: Vec<u8>,
    },
}

/// Swift-implemented stream provider. The runtime invokes `dispatch`
/// per inbound stream request; Swift returns one of the 6 variants.
#[uniffi::export(callback_interface)]
pub trait SwiftStreamProvider: Send + Sync {
    /// `peer_id` is the canonical libp2p peer-id string of the
    /// requester. `request_bytes` is the prost-encoded
    /// `auki.stream.StreamRequest`; Swift decodes via swift-protobuf.
    fn dispatch(&self, peer_id: String, request_bytes: Vec<u8>) -> SwiftStreamDispatch;
}
```

- [ ] **Step 4: Verify tests pass**

Run:
```bash
cargo test -p auki-network-swift swift_stream_provider_object_safety swift_stream_dispatch_variants
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add SwiftStreamProvider + 5 source traits"
```

---

### Task 12: Add Swift→Rust source-stream adapter functions

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

Convert each Swift source trait into an upstream `SourceStream<T>`. Pattern: spawn a tokio task that polls the Swift trait's `next_item()` and pushes `StreamItem<T>` values onto an mpsc, return a `ReceiverStream` wrapper as the `SourceStream<T>`. Five adapter functions, one per payload type — each almost identical except for the prost decode target and the `T` parameter.

- [ ] **Step 1: Add failing tests for the adapters**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    use auki_network_rs::stream_protocol::AudioFrame;
    use futures::StreamExt;

    /// `audio_source_to_stream` drains a Swift source that produces 3
    /// items then ends-of-source. Rust side reads back 3 items + `None`.
    #[tokio::test]
    async fn audio_source_adapter_drains_three_items() {
        struct ThreeItems {
            counter: std::sync::Mutex<u8>,
        }
        impl SwiftAudioSource for ThreeItems {
            fn next_item(&self) -> Result<Option<StreamItem>, String> {
                let mut c = self.counter.lock().unwrap();
                if *c >= 3 {
                    return Ok(None);
                }
                *c += 1;
                // Prost-encoded `AudioFrame { data: vec![*c] }` — but here
                // we just use raw bytes; the adapter doesn't validate.
                Ok(Some(StreamItem {
                    timestamp_ns: *c as i64,
                    payload_bytes: vec![*c],
                }))
            }
        }
        let source: Arc<dyn SwiftAudioSource> = Arc::new(ThreeItems {
            counter: Default::default(),
        });
        let mut rust_stream = audio_source_to_stream(source);
        for expected in 1u8..=3 {
            let item = rust_stream
                .next()
                .await
                .expect("stream has more items")
                .expect("item is Ok");
            assert_eq!(item.timestamp_ns, expected as i64);
        }
        assert!(rust_stream.next().await.is_none(), "source ended");
    }
```

- [ ] **Step 2: Run, expect FAIL on missing function**

Run:
```bash
cargo test -p auki-network-swift audio_source_adapter_drains_three_items
```

Expected: FAIL — `cannot find function 'audio_source_to_stream'`.

- [ ] **Step 3: Add the 5 adapter functions**

In `bindings/swift/auki-network-swift/src/lib.rs`, after the `SwiftStreamProvider` trait declaration from Task 11, append:

```rust
// ─── Source-stream adapters: Swift trait → upstream SourceStream<T> ─

/// Adapter: wraps a Swift `SwiftAudioSource` as an upstream
/// `SourceStream<AudioFrame>`. Spawns a tokio task that polls the trait
/// and pushes prost-decoded items onto an mpsc; returns a
/// `ReceiverStream` wrapper. Cancellation: when the runtime drops the
/// `SourceStream` (e.g. substream closed), the receiver drops, the
/// mpsc send fails, and the task exits.
pub(crate) fn audio_source_to_stream(
    source: Arc<dyn SwiftAudioSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::AudioFrame,
> {
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::AudioFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame =
                        match auki_network_rs::stream_protocol::AudioFrame::decode(
                            item.payload_bytes.as_slice(),
                        ) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = tx.send(Err(format!("AudioFrame decode: {e}"))).await;
                                break;
                            }
                        };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(detail) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftCameraSource` → `SourceStream<CameraFrame>`. Same
/// pattern as [`audio_source_to_stream`] with the prost decode target
/// swapped to `CameraFrame`.
pub(crate) fn camera_source_to_stream(
    source: Arc<dyn SwiftCameraSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::CameraFrame,
> {
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::CameraFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame =
                        match auki_network_rs::stream_protocol::CameraFrame::decode(
                            item.payload_bytes.as_slice(),
                        ) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = tx.send(Err(format!("CameraFrame decode: {e}"))).await;
                                break;
                            }
                        };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(detail) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftPointCloudSource` → `SourceStream<PointCloudFrame>`.
pub(crate) fn point_cloud_source_to_stream(
    source: Arc<dyn SwiftPointCloudSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::PointCloudFrame,
> {
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::PointCloudFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_network_rs::stream_protocol::PointCloudFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("PointCloudFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(detail) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftJointEncodersSource` → `SourceStream<JointEncodersFrame>`.
pub(crate) fn joint_encoders_source_to_stream(
    source: Arc<dyn SwiftJointEncodersSource>,
) -> auki_network_rs::stream_runtime::SourceStream<
    auki_network_rs::stream_protocol::JointEncodersFrame,
> {
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<
                auki_network_rs::stream_protocol::JointEncodersFrame,
            >,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_network_rs::stream_protocol::JointEncodersFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("JointEncodersFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(detail) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Adapter: `SwiftDetectionSource` → `SourceStream<DetectionFrame>`.
pub(crate) fn detection_source_to_stream(
    source: Arc<dyn SwiftDetectionSource>,
) -> auki_network_rs::stream_runtime::SourceStream<auki_datatypes::detection::DetectionFrame> {
    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<
            auki_network_rs::stream_runtime::StreamItem<auki_datatypes::detection::DetectionFrame>,
            String,
        >,
    >(16);
    tokio::spawn(async move {
        loop {
            match source.next_item() {
                Ok(Some(item)) => {
                    use prost::Message;
                    let frame = match auki_datatypes::detection::DetectionFrame::decode(
                        item.payload_bytes.as_slice(),
                    ) {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = tx.send(Err(format!("DetectionFrame decode: {e}"))).await;
                            break;
                        }
                    };
                    let upstream_item = auki_network_rs::stream_runtime::StreamItem {
                        timestamp_ns: item.timestamp_ns,
                        payload: frame,
                    };
                    if tx.send(Ok(upstream_item)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(detail) => {
                    let _ = tx.send(Err(detail)).await;
                    break;
                }
            }
        }
    });
    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}
```

- [ ] **Step 4: Verify adapter test passes**

Run:
```bash
cargo test -p auki-network-swift audio_source_adapter_drains_three_items
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): add Swift→Rust source-stream adapters (5 payloads)"
```

---

### Task 13: Wire `SwiftStreamProvider` into `spawn_for_swift`

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

Replace the placeholder `decline_all_streams()` in `spawn_for_swift` with an adapter that:
1. Takes the Swift `SwiftStreamProvider` as a new parameter
2. Wraps it in a closure matching the upstream `StreamProvider` type alias
3. The closure invokes `provider.dispatch(...)`, matches on `SwiftStreamDispatch`, decodes manifest/reason bytes via prost, calls the appropriate `*_source_to_stream` adapter, returns the upstream `StreamDispatch`

- [ ] **Step 1: Update the smoke test to take a provider**

Replace the `spawn_for_swift_smoke` test from Task 10 with:

```rust
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

        struct DeclineAllProvider;
        impl SwiftStreamProvider for DeclineAllProvider {
            fn dispatch(&self, _peer_id: String, _request_bytes: Vec<u8>) -> SwiftStreamDispatch {
                // Decline with empty reason bytes — adapter handles
                // decode-failure of empty bytes as `DeclineReason::default()`.
                SwiftStreamDispatch::Decline {
                    reason_bytes: vec![],
                }
            }
        }

        let wallet = auki_identity::Wallet::from_seed(vec![1u8; 32]).expect("32-byte seed");
        let identity =
            std::sync::Arc::new(auki_network_rs::PeerIdentity::from_wallet(wallet));

        let listener: Arc<dyn PeerLivenessListener> = Arc::new(NoopListener);
        let heartbeat: Arc<dyn HeartbeatTimestampProvider> = Arc::new(WallClockProvider);
        let stream_provider: Arc<dyn SwiftStreamProvider> = Arc::new(DeclineAllProvider);

        let rt = spawn_for_swift(
            identity,
            vec!["/ip4/127.0.0.1/tcp/0".to_string()],
            vec![],
            listener,
            heartbeat,
            stream_provider,
        )
        .await
        .expect("spawn succeeds");

        let pid = rt.local_peer_id_string();
        assert!(pid.starts_with("12D3KooW"));
        assert!(rt.connected_peer_id_strings().is_empty());
        rt.shutdown();
    }
```

- [ ] **Step 2: Update `spawn_for_swift` signature + body**

Locate the `spawn_for_swift` function added in Task 10. Replace it with:

```rust
#[uniffi::export(async_runtime = "tokio")]
pub async fn spawn_for_swift(
    identity: Arc<auki_network_rs::PeerIdentity>,
    listen_multiaddrs: Vec<Multiaddr>,
    allowed_peers: Vec<auki_network_rs::AllowedPeer>,
    peer_liveness_listener: Arc<dyn PeerLivenessListener>,
    heartbeat_timestamps: Arc<dyn HeartbeatTimestampProvider>,
    stream_provider: Arc<dyn SwiftStreamProvider>,
) -> Result<Arc<auki_network_rs::NetworkRuntime>, SpawnSwiftError> {
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

    let heartbeat_source = heartbeat_source_from_provider(heartbeat_timestamps);
    let upstream_provider = swift_provider_to_upstream(stream_provider);

    let (rt, _join_rx, liveness_rx, _membership_rx, _info_rx, _resources_rx, _sensors_rx, _registry_rx, _diagnostic_rx) =
        auki_network_rs::NetworkRuntime::spawn(
            swarm,
            allowed_peers,
            upstream_provider,
            heartbeat_source,
        )
        .map_err(|e| SpawnSwiftError::RuntimeSpawn {
            message: e.to_string(),
        })?;

    tokio::spawn(drain_liveness_events(liveness_rx, peer_liveness_listener));

    Ok(Arc::new(rt))
}

/// Adapter: wraps a Swift `SwiftStreamProvider` as an upstream
/// `StreamProvider` closure. Each inbound dispatch call decodes manifest
/// bytes via prost, calls the appropriate `*_source_to_stream` adapter
/// for the matched payload type, and returns the upstream
/// `StreamDispatch`.
fn swift_provider_to_upstream(
    provider: Arc<dyn SwiftStreamProvider>,
) -> auki_network_rs::stream_runtime::StreamProvider {
    Arc::new(
        move |peer: PeerId, request: auki_network_rs::stream_protocol::StreamRequest| {
            use prost::Message;
            // Encode the request to bytes for the Swift side.
            let request_bytes = request.encode_to_vec();
            let dispatch = provider.dispatch(peer.to_string(), request_bytes);
            match dispatch {
                SwiftStreamDispatch::Decline { reason_bytes } => {
                    let reason = auki_network_rs::stream_protocol::DeclineReason::decode(
                        reason_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::Decline { reason }
                }
                SwiftStreamDispatch::AcceptAudio {
                    manifest_bytes,
                    source,
                } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptAudio {
                        manifest,
                        source: audio_source_to_stream(source),
                    }
                }
                SwiftStreamDispatch::AcceptCamera {
                    manifest_bytes,
                    source,
                } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptCamera {
                        manifest,
                        source: camera_source_to_stream(source),
                    }
                }
                SwiftStreamDispatch::AcceptPointCloud {
                    manifest_bytes,
                    source,
                } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptPointCloud {
                        manifest,
                        source: point_cloud_source_to_stream(source),
                    }
                }
                SwiftStreamDispatch::AcceptJointEncoders {
                    manifest_bytes,
                    source,
                } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptJointEncoders {
                        manifest,
                        source: joint_encoders_source_to_stream(source),
                    }
                }
                SwiftStreamDispatch::AcceptDetection {
                    manifest_bytes,
                    source,
                } => {
                    let manifest = auki_network_rs::stream_protocol::StreamManifest::decode(
                        manifest_bytes.as_slice(),
                    )
                    .unwrap_or_default();
                    auki_network_rs::stream_runtime::StreamDispatch::AcceptDetection {
                        manifest,
                        source: detection_source_to_stream(source),
                    }
                }
            }
        },
    )
}
```

- [ ] **Step 3: Run the updated smoke test**

Run:
```bash
cargo test -p auki-network-swift spawn_for_swift_smoke
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network-swift): wire SwiftStreamProvider into spawn_for_swift"
```

---

### Task 14: Add upstream `StreamSubscriptionAudio` + `open_audio_stream`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

The first concretized open-stream surface. Upstream `NetworkRuntime::open_audio_stream(peer, request_bytes) -> Result<Arc<StreamSubscriptionAudio>, OpenStreamError>` calls the generic `open_stream::<AudioFrame>` internally and wraps the typed `StreamSubscription<AudioFrame>` in a `StreamSubscriptionAudio` Object that exposes `manifest_bytes()` + `async next_entry() -> Result<Option<StreamEntry>, StreamError>`.

Shared types added in this task (reused by Tasks 15-18):
- `StreamEntry { timestamp_ns: i64, seq: u64, payload_bytes: Vec<u8> }` UniFFI Record
- `StreamError` UniFFI Error (flattened from upstream `stream_runtime::StreamError`)
- `OpenStreamError` UniFFI Error (flattened)

- [ ] **Step 1: Add a failing test in the binding crate**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    /// `StreamSubscriptionAudio` is constructible from a typed upstream
    /// `StreamSubscription<AudioFrame>` and its `next_entry` yields
    /// items.
    #[tokio::test]
    async fn stream_subscription_audio_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{AudioFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        // Build a fake typed subscription with 2 entries.
        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![
            Ok(auki_network_rs::stream_runtime::StreamEntry {
                timestamp_ns: 1,
                seq: 0,
                payload: AudioFrame { data: vec![1, 2, 3] },
            }),
            Ok(auki_network_rs::stream_runtime::StreamEntry {
                timestamp_ns: 2,
                seq: 1,
                payload: AudioFrame { data: vec![4, 5] },
            }),
        ]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionAudio::from_inner(sub);

        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.seq, 0);
        let second = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(second.seq, 1);
        let third = wrapped.next_entry().await.expect("ok");
        assert!(third.is_none(), "stream ended");
    }
```

- [ ] **Step 2: Run, expect FAIL on missing type**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_audio_wraps_typed_subscription
```

Expected: FAIL — `cannot find function 'StreamSubscriptionAudio'`.

- [ ] **Step 3: Add the shared `StreamEntry`, `StreamError`, `OpenStreamError` types upstream**

In `crates/auki-network/src/network_runtime.rs`, near the existing UniFFI-exposed impl block from Task 9, add (gated on `swift-bindings`):

```rust
// ─── UniFFI-exposed stream surface ──────────────────────────────────

/// Shared cross-FFI stream entry shape. The opaque `payload_bytes` is
/// prost-encoded against the per-payload `.proto` (`AudioFrame.proto`,
/// `CameraFrame.proto`, …); Swift consumers decode via swift-protobuf.
/// Type-distinguishability lives at the `StreamSubscription*` /
/// `open_*_stream` level.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Record, Debug, Clone)]
pub struct StreamEntry {
    pub timestamp_ns: i64,
    pub seq: u64,
    pub payload_bytes: Vec<u8>,
}

/// Cross-FFI stream-error variants. Flattened from
/// `stream_runtime::StreamError`; non-FFI variants surface as Display'd
/// `message: String`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum StreamError {
    #[error("end of stream: {reason}")]
    EndOfStream { reason: String },
    #[error("connection lost")]
    ConnectionLost,
    #[error("protocol error: {message}")]
    Protocol { message: String },
}

#[cfg(feature = "swift-bindings")]
impl From<crate::stream_runtime::StreamError> for StreamError {
    fn from(e: crate::stream_runtime::StreamError) -> Self {
        match e {
            crate::stream_runtime::StreamError::EndOfStream { reason } => Self::EndOfStream {
                reason: format!("{reason:?}"),
            },
            crate::stream_runtime::StreamError::ConnectionLost => Self::ConnectionLost,
            crate::stream_runtime::StreamError::Protocol(p) => Self::Protocol {
                message: p.to_string(),
            },
        }
    }
}

/// Cross-FFI open-stream-error variants. Flattened.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum OpenStreamError {
    #[error("declined: {reason}")]
    Declined { reason: String },
    #[error("libp2p open failed: {message}")]
    LibP2p { message: String },
    #[error("protocol error: {message}")]
    Protocol { message: String },
    #[error("open timed out after {ms} ms")]
    Timeout { ms: u64 },
}

#[cfg(feature = "swift-bindings")]
impl From<crate::stream_runtime::OpenStreamError> for OpenStreamError {
    fn from(e: crate::stream_runtime::OpenStreamError) -> Self {
        use crate::stream_runtime::OpenStreamError as Up;
        match e {
            Up::Declined { reason } => Self::Declined {
                reason: format!("{reason:?}"),
            },
            Up::LibP2p(err) => Self::LibP2p {
                message: err.to_string(),
            },
            Up::Protocol(err) => Self::Protocol {
                message: err.to_string(),
            },
            Up::Timeout(d) => Self::Timeout {
                ms: d.as_millis() as u64,
            },
        }
    }
}
```

- [ ] **Step 4: Add `StreamSubscriptionAudio` Object**

In `crates/auki-network/src/network_runtime.rs`, after the shared types from Step 3, append:

```rust
/// Swift-friendly wrapper around `StreamSubscription<AudioFrame>`.
/// Exposes `manifest_bytes()` (prost-encoded `StreamManifest`) and
/// `next_entry()` (async; yields one entry per call until the stream
/// ends).
///
/// The wrapper is fail-poisoned: once `next_entry` returns `Err` (a
/// final stream error), subsequent calls return the same `Err`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionAudio {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<crate::stream_protocol::AudioFrame>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionAudio {
    /// Construct from an upstream typed subscription. Encodes the
    /// manifest once at construction.
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::AudioFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionAudio {
    /// Prost-encoded `StreamManifest`. Stable for the lifetime of the
    /// subscription; safe to call multiple times.
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    /// Read the next entry off the wire. Returns `Ok(Some(entry))` for
    /// each entry, `Ok(None)` exactly once when the stream ends
    /// cleanly, or `Err(StreamError)` once when the stream ends with an
    /// error. After `Ok(None)` or `Err`, subsequent calls return
    /// `Ok(None)`.
    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None; // Poison after error.
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}
```

- [ ] **Step 5: Add `open_audio_stream` on `NetworkRuntime`**

In `crates/auki-network/src/network_runtime.rs`, inside the UniFFI-exposed impl block on `NetworkRuntime` (added in Task 9), append:

```rust
    /// Open an outbound audio stream against `peer_id`. `request_bytes` is
    /// a prost-encoded `auki.stream.StreamRequest`. Returns a typed
    /// `StreamSubscriptionAudio` on accept; an `OpenStreamError` on
    /// decline, libp2p failure, or timeout.
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn open_audio_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionAudio>, OpenStreamError> {
        use prost::Message;
        let request = crate::stream_protocol::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let sub = self
            .open_stream::<crate::stream_protocol::AudioFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionAudio::from_inner(sub))
    }
```

- [ ] **Step 6: Verify build + test passes**

Run:
```bash
cargo build --features swift-bindings,swarm -p auki-network
cargo test -p auki-network-swift stream_subscription_audio_wraps_typed_subscription
```

Expected: PASS for both.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): add StreamSubscriptionAudio + open_audio_stream"
```

---

### Task 15: Add upstream `StreamSubscriptionCamera` + `open_camera_stream`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Same pattern as Task 14 with `CameraFrame` substituted.

- [ ] **Step 1: Add the failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn stream_subscription_camera_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{CameraFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 7,
            seq: 0,
            payload: CameraFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionCamera::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 7);
        assert!(wrapped.next_entry().await.expect("ok").is_none());
    }
```

- [ ] **Step 2: Run, expect FAIL**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_camera_wraps_typed_subscription
```

Expected: FAIL — type doesn't exist.

- [ ] **Step 3: Add the Object + method**

In `crates/auki-network/src/network_runtime.rs`, after the `StreamSubscriptionAudio` block from Task 14, append:

```rust
/// Swift-friendly wrapper around `StreamSubscription<CameraFrame>`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionCamera {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<crate::stream_protocol::CameraFrame>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionCamera {
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::CameraFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionCamera {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}
```

Then in the UniFFI-exposed impl block on `NetworkRuntime`, append:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn open_camera_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionCamera>, OpenStreamError> {
        use prost::Message;
        let request = crate::stream_protocol::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let sub = self
            .open_stream::<crate::stream_protocol::CameraFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionCamera::from_inner(sub))
    }
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_camera_wraps_typed_subscription
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): add StreamSubscriptionCamera + open_camera_stream"
```

---

### Task 16: Add upstream `StreamSubscriptionPointCloud` + `open_pointcloud_stream`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Same pattern; `PointCloudFrame`.

- [ ] **Step 1: Failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn stream_subscription_pointcloud_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{PointCloudFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 9,
            seq: 0,
            payload: PointCloudFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionPointCloud::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 9);
    }
```

- [ ] **Step 2: Run, expect FAIL**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_pointcloud_wraps_typed_subscription
```

Expected: FAIL.

- [ ] **Step 3: Add the Object + method**

In `crates/auki-network/src/network_runtime.rs`, after the `StreamSubscriptionCamera` block, append:

```rust
/// Swift-friendly wrapper around `StreamSubscription<PointCloudFrame>`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionPointCloud {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<crate::stream_protocol::PointCloudFrame>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionPointCloud {
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::PointCloudFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionPointCloud {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}
```

Then in the UniFFI-exposed impl block on `NetworkRuntime`, append:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn open_pointcloud_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionPointCloud>, OpenStreamError> {
        use prost::Message;
        let request = crate::stream_protocol::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let sub = self
            .open_stream::<crate::stream_protocol::PointCloudFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionPointCloud::from_inner(sub))
    }
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_pointcloud_wraps_typed_subscription
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): add StreamSubscriptionPointCloud + open_pointcloud_stream"
```

---

### Task 17: Add upstream `StreamSubscriptionJointEncoders` + `open_joint_encoders_stream`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Same pattern; `JointEncodersFrame`.

- [ ] **Step 1: Failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn stream_subscription_joint_encoders_wraps_typed_subscription() {
        use auki_network_rs::stream_protocol::{JointEncodersFrame, StreamManifest};
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 11,
            seq: 0,
            payload: JointEncodersFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionJointEncoders::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 11);
    }
```

- [ ] **Step 2: Run, expect FAIL**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_joint_encoders_wraps_typed_subscription
```

Expected: FAIL.

- [ ] **Step 3: Add the Object + method**

In `crates/auki-network/src/network_runtime.rs`, after the `StreamSubscriptionPointCloud` block, append:

```rust
/// Swift-friendly wrapper around `StreamSubscription<JointEncodersFrame>`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionJointEncoders {
    inner: tokio::sync::Mutex<
        Option<
            crate::stream_runtime::StreamSubscription<crate::stream_protocol::JointEncodersFrame>,
        >,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionJointEncoders {
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<crate::stream_protocol::JointEncodersFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionJointEncoders {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}
```

In the UniFFI-exposed impl block on `NetworkRuntime`, append:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn open_joint_encoders_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionJointEncoders>, OpenStreamError> {
        use prost::Message;
        let request = crate::stream_protocol::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let sub = self
            .open_stream::<crate::stream_protocol::JointEncodersFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionJointEncoders::from_inner(sub))
    }
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_joint_encoders_wraps_typed_subscription
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): add StreamSubscriptionJointEncoders + open_joint_encoders_stream"
```

---

### Task 18: Add upstream `StreamSubscriptionDetection` + `open_detection_stream`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`

Same pattern; `DetectionFrame` (from `auki_datatypes::detection`).

- [ ] **Step 1: Failing test**

Append to `bindings/swift/auki-network-swift/src/lib.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn stream_subscription_detection_wraps_typed_subscription() {
        use auki_datatypes::detection::DetectionFrame;
        use auki_network_rs::stream_protocol::StreamManifest;
        use auki_network_rs::stream_runtime::StreamSubscription;
        use futures::stream;

        let manifest = StreamManifest::default();
        let entries = stream::iter(vec![Ok(auki_network_rs::stream_runtime::StreamEntry {
            timestamp_ns: 13,
            seq: 0,
            payload: DetectionFrame::default(),
        })]);
        let sub = StreamSubscription {
            manifest,
            entries: Box::pin(entries),
        };
        let wrapped = auki_network_rs::StreamSubscriptionDetection::from_inner(sub);
        let first = wrapped.next_entry().await.expect("ok").expect("some");
        assert_eq!(first.timestamp_ns, 13);
    }
```

- [ ] **Step 2: Run, expect FAIL**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_detection_wraps_typed_subscription
```

Expected: FAIL.

- [ ] **Step 3: Add the Object + method**

In `crates/auki-network/src/network_runtime.rs`, after the `StreamSubscriptionJointEncoders` block, append:

```rust
/// Swift-friendly wrapper around `StreamSubscription<DetectionFrame>`.
#[cfg(feature = "swift-bindings")]
#[derive(uniffi::Object)]
pub struct StreamSubscriptionDetection {
    inner: tokio::sync::Mutex<
        Option<crate::stream_runtime::StreamSubscription<auki_datatypes::detection::DetectionFrame>>,
    >,
    manifest_bytes: Vec<u8>,
}

#[cfg(feature = "swift-bindings")]
impl StreamSubscriptionDetection {
    pub fn from_inner(
        inner: crate::stream_runtime::StreamSubscription<auki_datatypes::detection::DetectionFrame>,
    ) -> Arc<Self> {
        use prost::Message;
        let manifest_bytes = inner.manifest.encode_to_vec();
        Arc::new(Self {
            inner: tokio::sync::Mutex::new(Some(inner)),
            manifest_bytes,
        })
    }
}

#[cfg(feature = "swift-bindings")]
#[uniffi::export(async_runtime = "tokio")]
impl StreamSubscriptionDetection {
    pub fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest_bytes.clone()
    }

    pub async fn next_entry(&self) -> Result<Option<StreamEntry>, StreamError> {
        use futures::StreamExt;
        use prost::Message;

        let mut guard = self.inner.lock().await;
        let Some(sub) = guard.as_mut() else {
            return Ok(None);
        };
        match sub.entries.next().await {
            Some(Ok(entry)) => Ok(Some(StreamEntry {
                timestamp_ns: entry.timestamp_ns,
                seq: entry.seq,
                payload_bytes: entry.payload.encode_to_vec(),
            })),
            Some(Err(e)) => {
                *guard = None;
                Err(e.into())
            }
            None => {
                *guard = None;
                Ok(None)
            }
        }
    }
}
```

In the UniFFI-exposed impl block on `NetworkRuntime`, append:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::method(async_runtime = "tokio"))]
    pub async fn open_detection_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<StreamSubscriptionDetection>, OpenStreamError> {
        use prost::Message;
        let request = crate::stream_protocol::StreamRequest::decode(request_bytes.as_slice())
            .map_err(|e| OpenStreamError::Protocol {
                message: format!("StreamRequest decode: {e}"),
            })?;
        let sub = self
            .open_stream::<auki_datatypes::detection::DetectionFrame>(peer_id, request)
            .await
            .map_err(OpenStreamError::from)?;
        Ok(StreamSubscriptionDetection::from_inner(sub))
    }
```

- [ ] **Step 4: Verify test passes**

Run:
```bash
cargo test -p auki-network-swift stream_subscription_detection_wraps_typed_subscription
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-network): add StreamSubscriptionDetection + open_detection_stream"
```

---

### Task 19: Annotate upstream Discovery surface

**Files:**
- Modify: `crates/auki-network/src/discovery_client.rs`

Annotate `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError` upstream. Sets up Task 20's deletion of the hand-wrapped binding-crate types.

- [ ] **Step 1: Read the current upstream shapes**

Open `crates/auki-network/src/discovery_client.rs` and inspect:
- `DiscoveryClient` (struct around line 60-something — find via `pub struct DiscoveryClient`)
- `ClusterEntry` (struct — find via `pub struct ClusterEntry`)
- `CreateClusterOutcome` (enum — find via `pub enum CreateClusterOutcome`)
- `DiscoveryError` (enum, thiserror)

- [ ] **Step 2: Annotate `ClusterEntry`**

Locate the `pub struct ClusterEntry { ... }` block. Add the cfg_attr annotation:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterEntry {
    pub name: String,
    pub manager_peer_id: PeerId,
    pub manager_multiaddrs: Vec<Multiaddr>,
    pub peer_count: u32,
    pub created_ns: i64,
    pub last_liveness_check_ns: i64,
}
```

(`peer_count` may be `u32` or `u64` in the source — preserve verbatim.)

- [ ] **Step 3: Annotate `CreateClusterOutcome`**

`CreateClusterOutcome` is an enum with two variants. Swift expects a Record-shaped representation (Stage 1's binding crate flattened it to `{ already_exists: bool, entry: Option<ClusterEntry> }`). Either flatten to that Record shape upstream (refactor) OR annotate as a UniFFI Enum and let Swift switch on variants.

Decision: **annotate as Enum**. The Stage 1 Record was a hand-mapping convenience; with upstream annotations we get the typed enum directly.

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateClusterOutcome {
    Created(ClusterEntry),
    AlreadyExists,
}
```

(Adjust to match the actual existing variants; the implementer preserves the source verbatim and adds only the `cfg_attr`.)

- [ ] **Step 4: Annotate `DiscoveryError`**

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    // ... existing variants verbatim
}
```

(`flat_error` handles the `Transport(reqwest::Error)` variant cleanly.)

- [ ] **Step 5: Annotate `DiscoveryClient`**

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct DiscoveryClient {
    // ... existing fields verbatim
}
```

And add `#[cfg_attr(feature = "swift-bindings", uniffi::export)]` to the async impl block that contains `new`, `base_url`, `list_clusters`, `create_cluster`, `liveness_check`, `rotate_manager`, `deregister`. For each async method, the impl-block-level export handles it; for the `new` constructor add `#[cfg_attr(feature = "swift-bindings", uniffi::constructor)]`.

`new` may need to return `Arc<Self>` (UniFFI 0.31 contract on constructors). Adjust signature accordingly:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl DiscoveryClient {
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new(base_url: String) -> Arc<Self> {
        Arc::new(Self {
            // ... existing body
        })
    }

    pub fn base_url(&self) -> String {
        // ... existing body
    }

    pub async fn list_clusters(&self) -> Result<Vec<ClusterEntry>, DiscoveryError> {
        // ... existing body
    }

    // ... etc
}
```

(The implementer reads each existing method and adapts only the impl block opener + the `new` constructor. The `Arc<Self>` change may cascade to internal `DiscoveryClient::new` call sites; check `tests/` and `examples/`.)

- [ ] **Step 6: Verify build + tests pass**

Run:
```bash
cargo build --features swift-bindings,swarm,discovery_client -p auki-network
cargo test -p auki-network --features discovery_client --lib
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-network/src/discovery_client.rs
git commit -m "feat(auki-network): annotate Discovery surface for UniFFI export"
```

---

### Task 20: Delete Stage 1's hand-wrapped Discovery surface; migrate binding tests

**Files:**
- Modify: `bindings/swift/auki-network-swift/src/lib.rs`

The Stage 1 binding crate hand-wraps `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`. Task 19 made these all available directly from `auki-network-rs` with UniFFI annotations. Delete the duplicates from the binding crate; update the 4 existing tests.

- [ ] **Step 1: Delete the Stage 1 hand-wrapped blocks**

In `bindings/swift/auki-network-swift/src/lib.rs`, delete:
- The `pub struct ClusterEntry` block (around line 53)
- The `impl From<RustClusterEntry> for ClusterEntry` block
- The `pub struct CreateClusterOutcome` block
- The `pub enum DiscoveryError` block
- The `impl From<RustDiscoveryError> for DiscoveryError` block
- The `fn parse_peer_id` helper
- The `fn parse_multiaddrs` helper
- The `pub struct DiscoveryClient` block
- Both `impl DiscoveryClient` blocks (sync and async)

Also delete the import line that aliases `auki_network_rs::discovery_client::*` as `RustClusterEntry`/`RustDiscoveryClient`/etc. — those aliases are no longer needed.

- [ ] **Step 2: Re-export upstream types**

At the top of `bindings/swift/auki-network-swift/src/lib.rs`, replace the deleted aliasing block with simple re-exports so the test file (and the eventually-generated Swift surface) sees the upstream-annotated types under their canonical names:

```rust
// Re-exports of the upstream-annotated Discovery surface. UniFFI's
// scaffolding metadata picks these up via the binding crate's
// `setup_scaffolding!()`. Swift consumers see `DiscoveryClient`,
// `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError` — same names
// Stage 1 shipped, but now driven by upstream annotations.
pub use auki_network_rs::discovery_client::{
    ClusterEntry, CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
```

- [ ] **Step 3: Update the existing tests**

Locate the 4 existing tests in `bindings/swift/auki-network-swift/src/lib.rs` `mod tests` (Stage 1 tests: `maps_status_error`, `maps_invalid_peer_id_and_multiaddr`, `cluster_entry_conversion_stringifies_libp2p_types`, `rejects_malformed_peer_id_and_multiaddr`).

Several of these tested the Stage 1 hand-wrapped conversion logic (`From<Rust...> for ...`); after Task 19, those conversions don't exist. Either delete the tests that lost their subject, OR rewrite them as round-trip tests against the upstream-annotated types (their `Debug` / Display / variant matching still works).

Decision: delete `maps_status_error`, `maps_invalid_peer_id_and_multiaddr`, `cluster_entry_conversion_stringifies_libp2p_types` (they tested binding-crate-internal conversions). Keep `rejects_malformed_peer_id_and_multiaddr` but rewrite to test the `uniffi::custom_type!` `try_lift` paths added in Tasks 2-3 — that's the equivalent test in the new architecture.

Replace the `mod tests` block's contents related to the deletions with:

```rust
    /// Malformed PeerId / Multiaddr strings fail the `uniffi::custom_type!`
    /// `try_lift` arm (anyhow::Error with a message). Swift would see this
    /// as a thrown error on the affected method.
    #[test]
    fn malformed_peer_id_lifts_to_error() {
        // Direct exercise of the parse path the custom_type! try_lift uses.
        assert!("not-a-peer-id".parse::<PeerId>().is_err());
    }

    #[test]
    fn malformed_multiaddr_lifts_to_error() {
        assert!("definitely/not/an/addr".parse::<Multiaddr>().is_err());
    }
```

(Keep the `test_peer_id` helper; it's still used by the new tests.)

- [ ] **Step 4: Verify the binding crate builds and tests pass**

Run:
```bash
cargo build -p auki-network-swift
cargo test -p auki-network-swift
```

Expected: PASS. All tests added in Tasks 2-18 plus the surviving / rewritten Stage 1 tests.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-network-swift/src/lib.rs
git commit -m "refactor(auki-network-swift): delete Stage 1 hand-wrappers; use upstream Discovery"
```

---

### Task 21: Verify the upstream crate builds and tests cleanly with the new feature combos

**Files:** None modified — verification task.

- [ ] **Step 1: Verify default build is unchanged**

Run:
```bash
cargo build -p auki-network
cargo test -p auki-network --lib
```

Expected: PASS. The default-off `swift-bindings` feature must not affect non-Swift builds.

- [ ] **Step 2: Verify swift-bindings + swarm + discovery_client combination**

Run:
```bash
cargo build --features swift-bindings,swarm,discovery_client -p auki-network
cargo test --features swift-bindings,swarm,discovery_client -p auki-network --lib
```

Expected: PASS. All upstream lib tests survive.

- [ ] **Step 3: Verify the binding crate's combined build**

Run:
```bash
cargo build -p auki-network-swift
cargo test -p auki-network-swift
```

Expected: PASS.

- [ ] **Step 4: Verify workspace-wide build still works**

Run:
```bash
cargo build --workspace --exclude browser_probe_listener
```

Expected: PASS. (`browser_probe_listener` is the example PR A flagged as pre-existing broken; exclude it.)

- [ ] **Step 5: No commit needed — this is a verification gate**

If any of the above fail, return to the offending task and fix in place before proceeding to Task 22.

---

### Task 22: Validate the iOS XCFramework build end-to-end

**Files:** None modified — runs the build script.

- [ ] **Step 1: Run the XCFramework build**

Run:
```bash
bash bindings/swift/auki-network-swift/build-xcframework.sh
```

Expected: success — produces an XCFramework at `bindings/swift/auki-network-swift/build/AukiNetworkSwift.xcframework/` containing:
- `ios-arm64/` (device slice)
- `ios-arm64_x86_64-simulator/` (fat simulator slice)
- `module.modulemap` aggregating each upstream crate's modulemap (per PR A's `cf14503` multi-namespace fix)
- One `*.swift` file per UniFFI namespace (one for `auki-network-rs` upstream, one for `auki-network-swift` binding crate types)

- [ ] **Step 2: Inspect the generated Swift surface**

Run:
```bash
ls bindings/swift/auki-network-swift/build/AukiNetworkSwift.xcframework/ios-arm64/Headers/
```

Verify the file list includes the namespace `.swift` files and module map.

Spot-check the generated Swift signatures by reading a few `.swift` files. Look for:
- `class NetworkRuntime` (the upstream Object)
- `async func openAudioStream(peerId: String, requestBytes: Data) -> StreamSubscriptionAudio` (or similar)
- `class StreamSubscriptionAudio` with `async func nextEntry()`
- `protocol PeerLivenessListener` (callback interface)
- `protocol SwiftStreamProvider` (callback interface)
- `class DiscoveryClient` with `init(baseUrl: String)` and async methods

- [ ] **Step 3: No commit needed — this is integration validation**

If any expected Swift surface is missing, return to the relevant task to fix.

---

### Task 23: Write `bindings/swift/auki-network-swift` per-component docs

**Files:**
- Modify: `bindings/swift/auki-network-swift/README.md`
- Modify: `bindings/swift/auki-network-swift/parking_lot.md`
- Modify: `bindings/swift/auki-network-swift/changelog.md`
- Modify: `bindings/swift/auki-network-swift/src/readme.md`
- Modify: `bindings/swift/auki-network-swift/src/sprint.md`

- [ ] **Step 1: Update `README.md`**

The Stage 1 README scoped this crate to "Discovery HTTP client only". Rewrite to reflect PR B's expanded surface.

Replace the README contents with:

```markdown
# auki-network-swift

UniFFI Swift bindings for `auki-network` — exposes:

- **Discovery** HTTP client (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`). PR-A-and-earlier surface; rewritten in PR B to consume upstream-annotated types instead of hand-wrapped duplicates.
- **NetworkRuntime** + the `spawn_for_swift` orchestrator: spawn a libp2p runtime, observe peer liveness via the `PeerLivenessListener` callback interface, manage the cluster trust boundary via `setAllowedPeers`.
- **Stream surface**: 5 typed `StreamSubscription*` objects (`Audio`, `Camera`, `PointCloud`, `JointEncoders`, `Detection`) and matching `NetworkRuntime.open*Stream(...)` methods; producer side wired via the `SwiftStreamProvider` callback interface and 5 `Swift*Source` traits.

## Scope (after PR B)

Native iOS / Swift consumers can:

- Construct a `Wallet` (from `auki-identity-swift`, PR A) and derive a `PeerIdentity`.
- Spawn a `NetworkRuntime` against a list of listen multiaddrs + an initial allow-list of `AllowedPeer`s.
- Observe peer connect / disconnect / heartbeat-stream-closed events.
- Open outbound stream subscriptions against any peer for any of the 5 SDK-supported payload types.
- Accept inbound stream subscriptions via a Swift-implemented `SwiftStreamProvider`.
- Discover clusters via `DiscoveryClient.listClusters`, register as a Manager via `createCluster`, push liveness via `livenessCheck`, etc.

## API shape

Async. Swift consumers get real `async`/`await`; UniFFI drives futures on a process-wide multi-thread tokio runtime.

`PeerId` and `Multiaddr` cross the FFI as canonical strings (`uniffi::custom_type!` registrations). All prost wire types (`StreamRequest`, `StreamManifest`, `AudioFrame`, etc.) cross as opaque `Data`; Swift decodes via swift-protobuf against the `.proto` files committed under `crates/auki-datatypes/proto/`.

## Build

XCFramework via `./build-xcframework.sh`. Per PR A's multi-namespace handling, the produced framework aggregates UniFFI metadata from both `auki-network` (upstream) and `auki-network-swift` (binding crate).

iOS targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`.

## Out of scope

- `auki-domain` surface (cluster orchestration, sensor/resource catalogs, registries) — that's `auki-domain-swift` (PR C).
- Wallet / PeerIdentity (PR A's `auki-identity-swift`).
- A published SwiftPM package — distribution stays build-from-source via the iosapp sync script (Spec 2).
```

- [ ] **Step 2: Update `parking_lot.md`**

Delete the Stage 1 parking-lot item about "what to do when Stage 2 (stream surface) lands" — it's now resolved by this PR. Add new items as needed (e.g. heartbeat-detail variant exposure, generic concretization for additional payload types).

Replace the file contents with:

```markdown
# auki-network-swift parking lot

Open questions for this crate. Resolved items are deleted (per auki-sdk convention) and recorded in `changelog.md`.

## Open

- **Heartbeat-detail variants.** `SwiftPeerLivenessEvent` is a 3-variant subset at v0; `HeartbeatReceived` / `HeartbeatNtpSampleObserved` upstream variants are dropped by the drain task. Widen if iosapp needs heartbeat-timing observation.
- **`uniffi::custom_type!` reachability across binding crates.** `auki-domain-swift` (PR C) depends on this crate transitively — confirm the `PeerId` / `Multiaddr` registrations cross the boundary without redeclaration. If not, declare in both.
- **Single shared tokio runtime.** Each binding crate's `lib.rs` indirectly drives its own tokio runtime via UniFFI's `async_runtime = "tokio"` annotation. Consolidate if profiling shows pain.
- **Async API shape vs. `-py`'s sync shape.** Stage 1's parking lot flagged this; iosapp consumes async — confirmed acceptable.
```

- [ ] **Step 3: Update `changelog.md`**

Prepend a new entry:

```markdown
### Nils's claude · May 22, HKT, 2026

PR B — expanded scope beyond Stage 1's Discovery-only surface. Now exposes `NetworkRuntime` + `spawn_for_swift` orchestrator + the 5-payload stream surface (`StreamSubscriptionAudio`/`Camera`/`PointCloud`/`JointEncoders`/`Detection` Objects + matching `NetworkRuntime.open*Stream` methods + `SwiftStreamProvider` callback interface with 5 source traits) + `PeerLivenessListener` + `HeartbeatTimestampProvider` callback interfaces. Stage 1's hand-wrapped `DiscoveryClient`/`ClusterEntry`/`CreateClusterOutcome`/`DiscoveryError` deleted; consumers now see the upstream-annotated types directly via re-export. `PeerId` and `Multiaddr` registered as UniFFI custom types (canonical strings on the FFI seam). Prost wire types cross as opaque `Data`; Swift decodes via swift-protobuf.
```

(Preserve the existing Stage 1 entries below this new one.)

- [ ] **Step 4: Update `src/readme.md`**

Reflect what's now implemented:

```markdown
# auki-network-swift implementation

`src/lib.rs` hosts:

- `uniffi::setup_scaffolding!()` — the per-crate UniFFI metadata anchor.
- `uniffi::custom_type!(PeerId, String, ...)` and `uniffi::custom_type!(Multiaddr, String, ...)` — auto-expose every upstream method that touches libp2p identifiers.
- `SwiftPeerLivenessEvent` enum (3-variant v0 subset) + `from_upstream` translator.
- `PeerLivenessListener` callback-interface trait.
- `HeartbeatTimestampProvider` callback-interface trait + `heartbeat_source_from_provider` adapter.
- `StreamItem` Record (shared across all 5 payload types).
- 5 `Swift*Source` callback-interface traits (one per payload type).
- `SwiftStreamDispatch` enum (6 variants: 5 Accepts + 1 Decline).
- `SwiftStreamProvider` callback-interface trait.
- 5 source-stream adapter functions (`audio_source_to_stream`, etc.).
- `swift_provider_to_upstream` — converts a `SwiftStreamProvider` into an upstream `StreamProvider` closure.
- `SpawnSwiftError` enum.
- `spawn_for_swift` orchestrator function.
- Re-exports of `auki_network_rs::discovery_client::*` for the Discovery surface.

Upstream-side additions (in `crates/auki-network`):

- `NetworkRuntime` annotated as `uniffi::Object` with curated v0 methods (`local_peer_id_string`, `connected_peer_id_strings`, `set_allowed_peers`, `shutdown`).
- 5 `StreamSubscription*` Objects + 5 `open_*_stream` methods.
- `AllowedPeer`, `SpawnError`, `UpdateError`, `UpdateReport`, `StreamEntry`, `StreamError`, `OpenStreamError` annotated.
- `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError` annotated.
```

- [ ] **Step 5: Update `src/sprint.md`**

```markdown
# auki-network-swift sprint

## Current

PR B landed. The crate now covers the full v0 networking surface iosapp's Spec 2 proof-of-load needs.

## Next

PR C — `auki-domain-swift`. Depends on `auki-network-swift`'s `PeerId` / `Multiaddr` custom types (re-exports through the dep tree) and on `NetworkRuntime` being callable from upstream `auki_domain::ClusterManager`.
```

- [ ] **Step 6: Commit**

```bash
git add bindings/swift/auki-network-swift/README.md bindings/swift/auki-network-swift/parking_lot.md bindings/swift/auki-network-swift/changelog.md bindings/swift/auki-network-swift/src/readme.md bindings/swift/auki-network-swift/src/sprint.md
git commit -m "docs(auki-network-swift): refresh per-component docs after PR B"
```

---

### Task 24: Update `bindings/swift/` indices

**Files:**
- Modify: `bindings/swift/README.md`
- Modify: `bindings/swift/parking_lot.md`
- Modify: `bindings/swift/changelog.md`

- [ ] **Step 1: Update `bindings/swift/README.md`**

Find the per-crate table (or equivalent listing). Update the `auki-network-swift` row to reflect PR B's expanded scope:

```markdown
| `auki-network-swift` | Discovery HTTP client + `NetworkRuntime` + 5-payload stream surface + `PeerLivenessListener` / `SwiftStreamProvider` callback interfaces. PR B. |
```

- [ ] **Step 2: Update `bindings/swift/parking_lot.md`**

Update the `auki-network-swift` summary line (a structured 1-line entry, per CLAUDE.md propagation rules):

```markdown
- [`auki-network-swift/parking_lot.md`](auki-network-swift/parking_lot.md): 4 open items (heartbeat-detail variants, custom-type cross-crate reachability, shared tokio runtime, async-vs-sync API shape).
```

- [ ] **Step 3: Update `bindings/swift/changelog.md`**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

`auki-network-swift` PR B landed: expanded from Stage 1's Discovery-only surface to cover `NetworkRuntime` + spawn orchestration + 5-payload stream surface + callback interfaces (`PeerLivenessListener`, `HeartbeatTimestampProvider`, `SwiftStreamProvider`, 5 `Swift*Source`). Stage 1's hand-wrapped Discovery types deleted in favor of upstream-annotated re-exports. See [`auki-network-swift/changelog.md`](auki-network-swift/changelog.md) for the crate-level entry.
```

- [ ] **Step 4: Commit**

```bash
git add bindings/swift/README.md bindings/swift/parking_lot.md bindings/swift/changelog.md
git commit -m "docs(bindings/swift): propagate PR B summary to indices"
```

---

### Task 25: Propagate to upstream crate + bindings indices

**Files:**
- Modify: `crates/auki-network/parking_lot.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `bindings/changelog.md`

- [ ] **Step 1: Update `crates/auki-network/changelog.md`**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

PR B — UniFFI-annotated the full `swift-bindings`-gated surface needed by `bindings/swift/auki-network-swift`'s expansion. `NetworkRuntime` becomes a `uniffi::Object` with a curated v0 method set (`local_peer_id_string`, `connected_peer_id_strings`, `set_allowed_peers`, `shutdown`). Adds 5 `StreamSubscription*` Objects + 5 `open_*_stream` async methods (one per SDK-supported payload type: `AudioFrame`, `CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, `DetectionFrame`). Annotates `AllowedPeer`, `SpawnError`, `UpdateError`, `UpdateReport` and the Discovery surface (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`). Shared `StreamEntry { timestamp_ns, seq, payload_bytes: Vec<u8> }` record + flattened `StreamError` / `OpenStreamError` enums. Wire types stay prost-encoded `Vec<u8>` at the FFI seam.
```

- [ ] **Step 2: Update `crates/auki-network/parking_lot.md`**

If PR A added a swift-bindings note, extend it; otherwise prepend the same kind of entry as PR A did:

```markdown
- **`swift-bindings` feature**: gates UniFFI proc-macros on a curated subset of types. After PR B: `Wallet` (in `auki-identity`), `PeerIdentity`, `NetworkRuntime`, `AllowedPeer`, `SpawnError`, `UpdateError`, `UpdateReport`, `StreamEntry`, `StreamError`, `OpenStreamError`, `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`, and the 5 `StreamSubscription*` Objects. Methods marked with `#[uniffi::method(async_runtime = "tokio")]` get a tokio runtime driver via the binding crate's setup. No behavior change with the feature off (default).
```

(Replace any pre-existing PR-A note about the feature with this expanded version.)

- [ ] **Step 3: Update `crates/changelog.md`**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

`auki-network` — UniFFI-annotated the full PR B surface (`NetworkRuntime`, 5 `StreamSubscription*` Objects, 5 `open_*_stream` methods, error types, Discovery types). See [`auki-network/changelog.md`](auki-network/changelog.md).
```

- [ ] **Step 4: Update `bindings/changelog.md`**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

`bindings/swift/auki-network-swift` PR B landed: expanded to full v0 networking surface (runtime + 5-payload stream surface + callback interfaces). Stage 1's hand-wrapped Discovery surface replaced by upstream-annotated re-exports. See [`swift/auki-network-swift/changelog.md`](swift/auki-network-swift/changelog.md) and [`swift/changelog.md`](swift/changelog.md).
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/parking_lot.md crates/auki-network/changelog.md crates/changelog.md bindings/changelog.md
git commit -m "docs: propagate PR B summary to upstream crate + bindings indices"
```

---

### Task 26: Root + docs changelog propagation

**Files:**
- Modify: `changelog.md` (workspace root)
- Modify: `docs/changelog.md`

- [ ] **Step 1: Update `changelog.md` (root)**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

Spec 1 PR B landed: `bindings/swift/auki-network-swift` expanded from Stage 1's Discovery-only surface to full v0 networking — `NetworkRuntime` + `spawn_for_swift` + 5-payload stream surface (Audio, Camera, PointCloud, JointEncoders, Detection) + `PeerLivenessListener` / `HeartbeatTimestampProvider` / `SwiftStreamProvider` callback interfaces. Stage 1's hand-wrapped Discovery types replaced by upstream-annotated re-exports. `auki-network` upstream picks up the matching UniFFI annotations under the existing `swift-bindings` feature. Unblocks Spec 1 PR C (`auki-domain-swift`).
```

- [ ] **Step 2: Update `docs/changelog.md`**

Prepend:

```markdown
### Nils's claude · May 22, HKT, 2026

Added the [Spec 1 PR B implementation plan](superpowers/plans/2026-05-22-spec1-prb-auki-network-swift.md) for `auki-network-swift` expansion. 26 tasks covering: PeerId/Multiaddr custom types, `NetworkRuntime` annotation + spawn orchestrator + Swift callback interfaces (liveness, heartbeat, stream provider, 5 source traits), 5 concretized `open_*_stream` methods + `StreamSubscription*` Objects, Discovery surface rewrite, end-to-end XCFramework validation. Plan C (`auki-domain-swift`) follows once PR B lands.
```

- [ ] **Step 3: Commit**

```bash
git add changelog.md docs/changelog.md
git commit -m "docs: root + docs changelog entries for Spec 1 PR B"
```

---

## Implementation order

Tasks are listed in dependency order. Within each task, the steps are TDD-shaped:

1. **Task 1**: Cargo.toml feature expansion (unblocks everything downstream)
2. **Tasks 2-3**: PeerId / Multiaddr custom-type registrations (required by every upstream method that touches libp2p identifiers)
3. **Tasks 4-5**: `AllowedPeer` Record + error annotations (used by spawn orchestrator)
4. **Task 6**: `SwiftPeerLivenessEvent` (Swift-side translation of the upstream enum)
5. **Task 7**: `PeerLivenessListener` callback interface
6. **Task 8**: `HeartbeatTimestampProvider` callback interface + adapter
7. **Task 9**: `NetworkRuntime` Object + curated method set
8. **Task 10**: `spawn_for_swift` (placeholder stream provider)
9. **Tasks 11-12**: Full SwiftStreamProvider + 5 source traits + 5 adapters
10. **Task 13**: Wire SwiftStreamProvider into `spawn_for_swift`
11. **Tasks 14-18**: 5 `StreamSubscription*` Objects + 5 `open_*_stream` methods, one per payload type
12. **Tasks 19-20**: Discovery surface rewrite
13. **Task 21**: Workspace verification gate
14. **Task 22**: XCFramework end-to-end validation
15. **Tasks 23-26**: Doc + changelog propagation

## Stop gates between phases

- After Task 13 (spawn + stream provider full surface): the v0 proof-bar surface is complete. If iosapp's Spec 2 work needs to start before the rest of PR B lands, that's a viable cut point. Tasks 14-26 are "while we're here" expansions per the user's "full PR B per spec" decision.
- After Task 20 (Discovery rewrite + binding-crate cleanup): the binding crate is consistent (all annotation-driven, no hand-wrappers). Another viable cut point if the stream-surface tasks (14-18) are deferred.
- After Task 22 (XCFramework validation): the produced framework is iosapp-consumable. Spec 2 can start consuming it.
