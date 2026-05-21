# auki-domain-swift Implementation Plan (Spec 1, PR C)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the third and final binding crate of the [SDK Swift binding expansion](../specs/2026-05-20-sdk-swift-binding-expansion-design.md): `bindings/swift/auki-domain-swift`, providing native Swift access to the full `auki-domain::ClusterManager` surface with parity to `bindings/python/auki-domain-py` plus the upstream-only methods (clock sync, diagnostics) the user explicitly chose to include. After this PR, a Swift consumer can construct a `Wallet` (PR A), bootstrap a cluster Manager OR join an existing cluster, observe membership, fetch sensor/resource/registry catalogs, set sensor/resource catalog providers via Swift callback interfaces, open typed streams against any of the 5 SDK payload types, run clock sync, and broadcast diagnostics — all from native iOS code.

**Architecture:** PR A and PR B established the pattern: upstream `#[cfg_attr(feature = "swift-bindings", uniffi::*)]` annotations behind a new `swift-bindings` cargo feature on `crates/auki-domain`. The new binding crate `bindings/swift/auki-domain-swift/` is a thin scaffolding host — `uniffi::setup_scaffolding!()`, custom-type registrations for `PeerId`/`Multiaddr` (with the `remote` keyword), Swift-side callback adapters for the 2 catalog providers, and three orchestrator functions (`bootstrap_swift` / `create_cluster_swift` / `join_cluster_swift`) that build the libp2p swarm internally from a wallet seed + listen multiaddrs + agent version, matching the binding-layer ergonomics `auki-domain-py` established. Wire-type FFI shape: registry entries cross as **typed records** (user chose typed over the canonical-JSON shortcut Python uses), so `SensorBody`/`ClockBody`/`Frame*` enum trees are fully annotated. The 5 per-payload `StreamSubscription*` Objects from PR B are reused via `pub use` re-export — no duplication. Stream provider hookup for inbound producer-side (microphone) uses PR B's `SwiftStreamProvider` two-call protocol, exposed via this crate's `bootstrap_swift` parameter list.

**Tech Stack:** Rust 2024 edition, Cargo workspace, UniFFI 0.31 with `tokio` feature, `cfg_attr` for feature-gated proc-macros, libp2p 0.56 for swarm construction inside the bootstrap shims, prost 0.13 for wire types (cross FFI as `Vec<u8>`), Xcode 26.3 toolchain for iOS XCFramework, validated by PR A's `cf14503` multi-namespace fix and PR B's `WORKSPACE_ROOT` fix.

---

## File Structure

Files this PR creates or modifies.

**Upstream Rust crates (annotations behind new `swift-bindings` feature on `auki-domain`):**
- Modify: `crates/auki-domain/Cargo.toml` (new `swift-bindings` feature; optional `uniffi` + `anyhow` deps)
- Modify: `crates/auki-domain/src/lib.rs` (`uniffi::setup_scaffolding!()` invocation gated)
- Modify: `crates/auki-domain/src/cluster_manager.rs` (annotations on ClusterTarget, ClusterManager, DaemonInfo, all errors, all records on the public surface, SensorCatalogProvider + ResourceCatalogProvider as callback interfaces, 5 new `open_*_stream` methods)
- Modify: `crates/auki-domain/src/cluster_membership.rs` (annotations on ClusterMembership + ClusterMember)
- Modify: `crates/auki-network/src/participant.rs` (annotate ParticipantInfo if PR B didn't already)
- Modify: `crates/auki-network/src/resources_protocol.rs` (annotate ResourceEntry, SensorStreamResource, TransformEdgeResource + geometry types, ResourcesRequest, ResourcesResponse)
- Modify: `crates/auki-network/src/sensors_protocol.rs` (annotate SensorEntry, SensorsRequest, SensorsResponse)
- Modify: `crates/auki-network/src/registries_protocol.rs` (annotate SensorRegistryEntry, ClockRegistryEntry, FrameRegistryEntry, DetectorRegistryEntry + their nested type trees: SensorBody/ClockBody/Handedness/AxisConvention/AxisDirection/LengthUnit/Scope/ClockMeta/etc.)
- Modify: `crates/auki-network/src/diagnostic_protocol.rs` (annotate DiagnosticMessage, InboundDiagnosticMessage)
- Modify: `crates/auki-time/src/lib.rs` (annotate ClockTransformEstimate, DomainClockEstimate, related errors)
- Modify: `crates/auki-domain/parking_lot.md`, `crates/auki-domain/changelog.md`, `crates/changelog.md`

**New binding crate:**
- Create: `bindings/swift/auki-domain-swift/Cargo.toml`
- Create: `bindings/swift/auki-domain-swift/.gitignore`
- Create: `bindings/swift/auki-domain-swift/src/lib.rs`
- Create: `bindings/swift/auki-domain-swift/src/bin/uniffi-bindgen.rs`
- Create: `bindings/swift/auki-domain-swift/build-xcframework.sh`
- Create: `bindings/swift/auki-domain-swift/README.md`
- Create: `bindings/swift/auki-domain-swift/parking_lot.md`
- Create: `bindings/swift/auki-domain-swift/changelog.md`
- Create: `bindings/swift/auki-domain-swift/src/readme.md`
- Create: `bindings/swift/auki-domain-swift/src/sprint.md`

**Workspace + indices:**
- Modify: `Cargo.toml` (workspace `members` entry for the new crate)
- Modify: `bindings/swift/README.md`, `bindings/swift/parking_lot.md`, `bindings/swift/changelog.md`
- Modify: `bindings/changelog.md`
- Modify: `changelog.md` (root), `docs/changelog.md`

---

### Task 1: Add `swift-bindings` cargo feature to `crates/auki-domain`

**Files:**
- Modify: `crates/auki-domain/Cargo.toml`

Add the feature gate. Mirrors PR A's pattern on `auki-identity` and PR B's pattern on `auki-network`. The feature pulls optional `uniffi` (0.31, with `tokio`) + `anyhow` (1, for `custom_type!`'s `try_lift`). Propagates `swift-bindings` to both `auki-identity` and `auki-network` since downstream types (`Wallet`, `PeerId`, `NetworkRuntime`, etc.) are referenced from this crate's annotated methods.

- [ ] **Step 1: Show the missing-feature failure**

```bash
cargo build --features swift-bindings -p auki-domain
```

Expected: FAIL with `error: Package 'auki-domain' does not have feature 'swift-bindings'`.

- [ ] **Step 2: Add optional deps + feature**

Open `crates/auki-domain/Cargo.toml`. Locate the `[features]` block. Append:

```toml
# Enables UniFFI proc-macros on ClusterManager, ClusterTarget, all errors, all
# value records on the public surface (DaemonInfo, ClusterMember, ClusterMembership,
# ParticipantInfo, SensorEntry, registry-entry trees, etc.), and the two catalog-
# provider traits (SensorCatalogProvider, ResourceCatalogProvider) as callback
# interfaces. When off (default), the crate compiles exactly as today — no
# UniFFI dep in the graph.
#
# Propagates to auki-identity and auki-network because the bootstrap signatures
# accept PeerIdentity (auki-network) which is constructed from Wallet
# (auki-identity), and several method signatures cross-reference annotated
# types defined in those crates (PR A annotated Wallet/PeerIdentity; PR B
# annotated NetworkRuntime/StreamSubscription*/DiscoveryClient/etc.).
swift-bindings = [
    "dep:uniffi",
    "dep:anyhow",
    "auki-identity/swift-bindings",
    "auki-network/swift-bindings",
]
```

Locate the `[dependencies]` block. Append:

```toml
# Optional — pulled in only when the `swift-bindings` feature is on.
uniffi = { version = "0.31", features = ["tokio"], optional = true }
anyhow = { version = "1", optional = true }
```

- [ ] **Step 3: Verify feature-on build succeeds**

```bash
cargo build --features swift-bindings -p auki-domain
```

Expected: PASS. No annotations exist yet; uniffi is just an unused dep.

- [ ] **Step 4: Verify default build is unchanged**

```bash
cargo build -p auki-domain
cargo test -p auki-domain --lib
```

Expected: PASS, identical to before.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain/Cargo.toml
git commit -m "feat(auki-domain): add optional swift-bindings cargo feature"
```

---

### Task 2: Add `uniffi::setup_scaffolding!()` to `crates/auki-domain/src/lib.rs`

**Files:**
- Modify: `crates/auki-domain/src/lib.rs`

UniFFI 0.31 proc-macros expand to code referencing `crate::UniFfiTag`. The macro is created by `setup_scaffolding!()` — each crate that hosts UniFFI annotations needs its own invocation. This is the same pattern PR A established on `auki-identity` and PR B on `auki-network`.

- [ ] **Step 1: Read the current `lib.rs`**

```bash
cat crates/auki-domain/src/lib.rs
```

Expected: 48 lines with `pub mod browser_session; pub mod cluster_manager; pub mod cluster_membership; pub mod stream_manifest;` and re-exports.

- [ ] **Step 2: Add the scaffolding macro**

At the top of `crates/auki-domain/src/lib.rs`, after the existing doc comments but before any `pub mod`, insert:

```rust
// UniFFI scaffolding. Each annotated `Record` / `Enum` / `Object` /
// `Error` derive emits `impl FfiConverter<crate::UniFfiTag> for X`, and
// `UniFfiTag` is only defined where `setup_scaffolding!()` is invoked.
// Without this, building `--features swift-bindings` fails before the
// binding crate ever links. Gated so default builds stay scaffolding-free.
#[cfg(feature = "swift-bindings")]
uniffi::setup_scaffolding!();
```

- [ ] **Step 3: Verify build succeeds with the feature**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain
```

Expected: both PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-domain/src/lib.rs
git commit -m "feat(auki-domain): setup UniFFI scaffolding under swift-bindings feature"
```

---

### Task 3: Create the `auki-domain-swift` binding crate scaffolding

**Files:**
- Create: `bindings/swift/auki-domain-swift/Cargo.toml`
- Create: `bindings/swift/auki-domain-swift/.gitignore`
- Create: `bindings/swift/auki-domain-swift/src/lib.rs`
- Create: `bindings/swift/auki-domain-swift/src/bin/uniffi-bindgen.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create the workspace directory**

```bash
mkdir -p bindings/swift/auki-domain-swift/src/bin
```

- [ ] **Step 2: Create `Cargo.toml`**

Write to `bindings/swift/auki-domain-swift/Cargo.toml`:

```toml
[package]
name = "auki-domain-swift"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "UniFFI Swift bindings for `auki-domain` — exposes the cluster Manager surface (bootstrap / membership / catalogs / streams / clock sync / diagnostics) to native iOS (Swift) peers."

[lib]
# `staticlib` is the iOS-consumable artifact (dynamic libs are heavily
# constrained on-device); `cdylib` lets library-mode `uniffi-bindgen`
# introspect it on the host; `rlib` keeps in-workspace Rust consumers
# working. Lib name distinct from `auki-domain`'s own lib name so both
# can be direct deps in one workspace, mirroring the
# `auki-domain-py` precedent.
name = "auki_domain_swift"
crate-type = ["staticlib", "cdylib", "rlib"]

[features]
default = []
# Enables the `uniffi-bindgen` helper binary (Swift codegen host tool).
# Default-off so a normal `cargo build`/`cargo test` of the library
# doesn't pull the UniFFI CLI.
cli = ["uniffi/cli"]

[dependencies]
# Upstream domain crate, renamed via `package =` to avoid lib name collision.
# The `swift-bindings` feature must be on for the UniFFI proc-macros to compile.
# `swarm` and `discovery_client` are pulled because bootstrap_swift constructs
# both a libp2p swarm and a DiscoveryClient internally from Swift kwargs.
auki-domain-rs = { package = "auki-domain", path = "../../../crates/auki-domain", features = ["swift-bindings"] }

# auki-network for: building the swarm in bootstrap_swift; constructing
# PeerIdentity from a Wallet; reaching the StreamSubscription* objects
# annotated by PR B (re-exported via `pub use` below); re-using the
# PeerLivenessListener / HeartbeatTimestampProvider / SwiftStreamProvider
# patterns from `auki-network-swift` (the binding-crate-side callback
# interfaces live there, NOT in this crate — depend on auki-network-swift
# below to import them).
auki-network = { path = "../../../crates/auki-network", features = ["swift-bindings", "swarm", "discovery_client"] }

# Wallet::from_seed — bootstrap_swift takes 32-byte wallet seeds.
auki-identity = { path = "../../../crates/auki-identity", features = ["swift-bindings"] }

# Time-transform records (DomainClockEstimate, ClockTransformEstimate) carry
# auki-time types in their fields. Pulled in for the annotations.
auki-time = { path = "../../../crates/auki-time" }

# Cross-crate UniFFI dep: `auki-network-swift` defines the StreamSubscription*
# Swift surface, plus PeerLivenessListener / HeartbeatTimestampProvider /
# SwiftStreamProvider callback-interface traits. We `pub use` those here so
# Swift consumers see a single import surface (`AukiDomain`-style framework
# umbrella) and don't have to import auki-network-swift separately.
#
# The dep's `rlib` is pulled in; UniFFI's `use_remote_type!` macro (used in
# src/lib.rs) re-exposes the FfiConverter impls so this crate's UniFfiTag
# can also lower those types.
auki-network-swift = { path = "../auki-network-swift" }

# `PeerId` / `Multiaddr` parsing on the Swift ↔ Rust seam (same pattern as
# auki-network-swift — they cross the FFI as canonical strings).
libp2p-identity = { version = "0.2", default-features = false, features = ["ed25519", "peerid", "serde"] }
multiaddr = "0.18"

# libp2p stack: bootstrap_swift constructs a `Swarm<Behaviour>` internally.
libp2p = { version = "0.56", default-features = false, features = ["tokio", "tcp", "quic", "noise", "yamux", "macros"] }

# Process-wide multi-thread tokio runtime that UniFFI drives the
# exported async fns on.
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

# UniFFI proc-macro mode (no UDL). `tokio` feature enables
# `#[uniffi::export(async_runtime = "tokio")]`.
uniffi = { version = "0.31", features = ["tokio"] }

# FFI error enum Display / std::error::Error impls.
thiserror = "2"

# Used by the `try_lift` arms of `uniffi::custom_type!` declarations.
anyhow = "1"

# JSON round-trip for ClusterMembership.to_json / from_json helpers (matches
# auki-domain-py's binding-shim surface).
serde_json = "1"

# Swift codegen helper. Gated on `cli` so a plain library build/test
# doesn't require the UniFFI CLI; the build script enables the feature.
[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"
required-features = ["cli"]
```

- [ ] **Step 3: Create `.gitignore`**

Write to `bindings/swift/auki-domain-swift/.gitignore`:

```
target-xcframework/
```

(Same pattern as `auki-identity-swift` and `auki-network-swift`.)

- [ ] **Step 4: Create the scaffolding `src/lib.rs` (stub)**

Write to `bindings/swift/auki-domain-swift/src/lib.rs`:

```rust
//! UniFFI Swift bindings for `auki-domain`.
//!
//! ## Scope (v0 — PR C)
//!
//! Full ClusterManager surface for native iOS / Swift consumers, with
//! parity to [`bindings/python/auki-domain-py`](../../python/auki-domain-py)
//! plus the upstream-only methods (clock sync, diagnostics) explicitly
//! included per the design spec.
//!
//! - **Bootstrap orchestrators**: [`bootstrap_swift`], [`create_cluster_swift`],
//!   [`join_cluster_swift`] — each accepts a wallet seed + listen multiaddrs +
//!   agent version + DaemonInfo + optional SwiftStreamProvider (re-exported
//!   from `auki-network-swift`), builds the libp2p swarm internally, and
//!   returns an `Arc<ClusterManager>`.
//! - **ClusterManager** — full method set: cluster_name, local_peer_id,
//!   is_manager, manager_peer_id, peer_count, membership, admit_peer,
//!   participant_info, fetch_participant_info, fetch_*_catalog (sensors +
//!   resources), fetch_*_entry (sensor + clock + frame + detector), the
//!   5 typed `open_*_stream` methods, set_*_provider, set_registry_app_root,
//!   clock_sync_estimate / clock_sync_estimates / domain_clock_estimate /
//!   domain_time_now, broadcast_diagnostic_message / drain_diagnostic_messages,
//!   shutdown.
//! - **Callback interfaces** for the two catalog providers (re-exported
//!   from upstream — they're now `#[uniffi::export(callback_interface)]`
//!   in auki-domain).
//! - **Stream surface** — `StreamSubscriptionAudio`/`Camera`/`PointCloud`/
//!   `JointEncoders`/`Detection` Objects + the typed `open_*_stream`
//!   methods, all re-exported from PR B's `auki-network-swift`.
//!
//! Wire types (prost) cross as opaque `Vec<u8>`; Swift decodes via
//! swift-protobuf. PeerId / Multiaddr custom-type registrations are
//! inherited transitively from `auki-network-swift`'s dep.

use auki_network::AllowedPeer;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::sync::Arc;

uniffi::setup_scaffolding!();

// ─── Custom-type registrations ─────────────────────────────────────
//
// PeerId and Multiaddr custom_type! declarations live in auki-network-swift.
// UniFFI generates per-crate FfiConverter impls anchored on each crate's
// UniFfiTag — since this crate has its own UniFfiTag (from the
// setup_scaffolding!() above), we need our own custom_type registrations
// too, even though the upstream methods returning PeerId/Multiaddr were
// already auto-exposed for auki-network-swift's UniFfiTag.

uniffi::custom_type!(PeerId, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<PeerId>()
            .map_err(|e| anyhow::anyhow!("invalid peer-id {s:?}: {e}"))
    },
    lower: |p: PeerId| p.to_string(),
});

uniffi::custom_type!(Multiaddr, String, {
    remote,
    try_lift: |s: String| {
        s.parse::<Multiaddr>()
            .map_err(|e| anyhow::anyhow!("invalid multiaddr {s:?}: {e}"))
    },
    lower: |m: Multiaddr| m.to_string(),
});

// Subsequent tasks (5+) add:
//   - Re-exports of ClusterManager / ClusterTarget / DaemonInfo / records /
//     errors from upstream auki-domain.
//   - bootstrap_swift / create_cluster_swift / join_cluster_swift adapters.
//   - Re-exports of StreamSubscription* / Swift*Source / SwiftStreamProvider
//     from auki-network-swift so consumers see one API surface.

#[cfg(test)]
mod tests {
    use super::*;

    /// PeerId custom-type round-trip (regression guard for the binding
    /// crate's registration — different UniFfiTag than auki-network-swift's).
    #[test]
    fn peer_id_custom_type_round_trips() {
        let pid = libp2p_identity::Keypair::ed25519_from_bytes([5u8; 32])
            .expect("valid ed25519 seed")
            .public()
            .to_peer_id();
        let s = pid.to_string();
        let back: PeerId = s.parse().expect("canonical PeerId string parses");
        assert_eq!(back, pid);
    }

    #[test]
    fn multiaddr_custom_type_round_trips() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        assert_eq!(addr.to_string().parse::<Multiaddr>().unwrap(), addr);
    }
}
```

- [ ] **Step 5: Create `src/bin/uniffi-bindgen.rs`**

Write to `bindings/swift/auki-domain-swift/src/bin/uniffi-bindgen.rs`:

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

- [ ] **Step 6: Add the crate to the workspace members**

In the workspace root `Cargo.toml`, locate the `[workspace] members = [...]` block. Add `"bindings/swift/auki-domain-swift"` to the members list (alphabetical order with the existing swift binding crates).

- [ ] **Step 7: Verify the binding crate builds and tests pass**

```bash
cargo build -p auki-domain-swift
cargo test -p auki-domain-swift --lib
```

Expected: build succeeds; `peer_id_custom_type_round_trips` and `multiaddr_custom_type_round_trips` tests pass (the only tests at this checkpoint).

- [ ] **Step 8: Commit**

```bash
git add bindings/swift/auki-domain-swift/ Cargo.toml
git commit -m "feat(auki-domain-swift): create binding crate scaffolding"
```

---

### Task 4: Create the `build-xcframework.sh` script

**Files:**
- Create: `bindings/swift/auki-domain-swift/build-xcframework.sh`

Copy from `auki-network-swift`'s validated script (post-PR-B's `WORKSPACE_ROOT` fix), adapting the crate name. Multi-namespace UniFFI handling (one `.swift` file per upstream crate) is required because this binding aggregates types from `auki-identity`, `auki-network`, AND `auki-domain` (three upstream UniFfiTags) plus its own.

- [ ] **Step 1: Create the script**

Write to `bindings/swift/auki-domain-swift/build-xcframework.sh`:

```bash
#!/usr/bin/env bash
# Build the auki-domain-swift XCFramework + generated Swift bindings.
#
# Validated on rustc 1.94 + Xcode 26.3 against the three Apple targets
# below. Produces a two-slice AukiDomain.xcframework (device ios-arm64
# + fat simulator ios-arm64_x86_64-simulator) plus the generated Swift
# glue in $OUT/swift/, kept *outside* the xcframework Headers dir so
# SwiftPM consumers can pick it up at the package level while the
# xcframework Headers stay clean (FFI header + modulemap only).
#
# Multi-namespace UniFFI: this binding aggregates auki-identity,
# auki-network, auki-domain, and the binding crate's own UniFfiTags.
# uniffi-bindgen emits one set of {.swift, *FFI.h, *FFI.modulemap} per
# namespace; we concatenate them into a single module.modulemap (same
# fix PR A's cf14503 introduced for auki-identity-swift's multi-namespace
# case).
#
# Prereqs:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"
LIB_NAME="auki_domain_swift"
OUT="$CRATE_DIR/target-xcframework"
BINDINGS="$OUT/bindings"
mkdir -p "$BINDINGS"

cd "$WORKSPACE_ROOT"

# 1. Build the static lib for device + both simulator arches.
for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p auki-domain-swift --target "$TARGET"
done

# 2. Fat static lib for the simulator (xcframework rejects two slices
#    with the same platform), device lib stays standalone.
DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

# 3. Generate Swift bindings.
cargo run --release --features cli -p auki-domain-swift --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"

# Multi-namespace UniFFI output (auki_identity, auki_network, auki_domain,
# auki_domain_swift namespaces): consolidate the modulemaps into a single
# `module.modulemap` so Xcode/SwiftPM can find them as one umbrella module.
{
  for mm in "$BINDINGS"/*FFI.modulemap; do
    [ -f "$mm" ] || continue
    cat "$mm"
    echo
  done
} > "$BINDINGS/module.modulemap.tmp"
mv "$BINDINGS/module.modulemap.tmp" "$BINDINGS/module.modulemap"
find "$BINDINGS" -name "*FFI.modulemap" -delete

# Move every .swift out of $BINDINGS so step 4's `-headers $BINDINGS`
# packages only the FFI .h + modulemap.
SWIFT_OUT="$OUT/swift"
mkdir -p "$SWIFT_OUT"
for sf in "$BINDINGS"/*.swift; do
  [ -f "$sf" ] || continue
  mv "$sf" "$SWIFT_OUT/"
done

# 4. Assemble the XCFramework.
rm -rf "$OUT/AukiDomain.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT"    -headers "$BINDINGS" \
  -output "$OUT/AukiDomain.xcframework"

echo "XCFramework: $OUT/AukiDomain.xcframework"
echo "Swift glue : $SWIFT_OUT/${LIB_NAME}.swift"
```

- [ ] **Step 2: Make executable**

```bash
chmod +x bindings/swift/auki-domain-swift/build-xcframework.sh
```

- [ ] **Step 3: Commit**

```bash
git add bindings/swift/auki-domain-swift/build-xcframework.sh
git commit -m "feat(auki-domain-swift): add iOS XCFramework build script"
```

---

### Task 5: Annotate flat records on `auki-domain` + `auki-network`

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs` (DaemonInfo)
- Modify: `crates/auki-domain/src/cluster_membership.rs` (ClusterMember, ClusterMembership)
- Modify: `crates/auki-network/src/participant.rs` (ParticipantInfo — if PR B didn't already annotate it)
- Modify: `crates/auki-network/src/sensors_protocol.rs` (SensorEntry, SensorsRequest, SensorsResponse)
- Modify: `crates/auki-network/src/resources_protocol.rs` (ResourcesRequest)

All flat data records — annotate with `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]`. Fields cross via existing custom-type registrations (PeerId/Multiaddr → String).

- [ ] **Step 1: Add a binding-crate failing test for DaemonInfo**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn daemon_info_record_constructs() {
        let info = auki_domain_rs::cluster_manager::DaemonInfo {
            app: "test-app".to_string(),
            name: "test-name".to_string(),
            session_id: "session-1".to_string(),
            session_clock_id: "clock-1".to_string(),
            session_clock_hash: "hash-1".to_string(),
            app_instance: "instance-1".to_string(),
        };
        assert_eq!(info.app, "test-app");
    }
```

- [ ] **Step 2: Run baseline**

```bash
cargo test -p auki-domain-swift daemon_info_record_constructs
```

Expected: PASS (struct already exists; baseline).

- [ ] **Step 3: Annotate `DaemonInfo`**

Locate `pub struct DaemonInfo` in `crates/auki-domain/src/cluster_manager.rs` (use grep). Add above the existing `#[derive(...)]`:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

- [ ] **Step 4: Annotate `ClusterMember`**

Locate `pub struct ClusterMember` in `crates/auki-domain/src/cluster_membership.rs`. Add above the existing derives:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

Note: `successor_token: Option<Vec<u8>>` — `Vec<u8>` lowers natively as `Data` in Swift; `Option<Vec<u8>>` lowers as `Data?`. Fine.

- [ ] **Step 5: Annotate `ClusterMembership`**

Locate `pub struct ClusterMembership` in `crates/auki-domain/src/cluster_membership.rs`. Add above the existing derives:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

Note: `peers: Vec<ClusterMember>` — element type is now UniFFI Record, so the vec auto-lowers.

- [ ] **Step 6: Annotate `ParticipantInfo`**

Check `crates/auki-network/src/participant.rs`. If `ParticipantInfo` already has `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` (PR B may have added it), skip. Otherwise add it above the existing derives.

Verify with:
```bash
grep -A2 "pub struct ParticipantInfo" crates/auki-network/src/participant.rs
```

- [ ] **Step 7: Annotate `SensorEntry` + `SensorsRequest` + `SensorsResponse`**

Locate each in `crates/auki-network/src/sensors_protocol.rs`. Add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` above each.

Note for `SensorsResponse`: it's a single-field wrapper (`{ sensors: Vec<SensorEntry> }`). Annotating it is cheap; Swift consumers can also unwrap to `Vec<SensorEntry>` at the call site by accessing `.sensors`. Match the upstream shape.

- [ ] **Step 8: Annotate `ResourcesRequest`**

Locate in `crates/auki-network/src/resources_protocol.rs`. Add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` above it.

- [ ] **Step 9: Verify build + tests pass**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
cargo test -p auki-domain-swift daemon_info_record_constructs
```

Expected: all PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs crates/auki-domain/src/cluster_membership.rs crates/auki-network/src/sensors_protocol.rs crates/auki-network/src/resources_protocol.rs crates/auki-network/src/participant.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain,auki-network): annotate flat records for swift-bindings"
```

---

### Task 6: Annotate `ResourceEntry` tagged union + sub-records

**Files:**
- Modify: `crates/auki-network/src/resources_protocol.rs`

`ResourceEntry` is a tagged enum with two struct-heavy variants (`SensorStream(SensorStreamResource)`, `TransformEdge(TransformEdgeResource)`). Annotate as `uniffi::Enum`; both inner records need `uniffi::Record`. The geometry sub-records (`ResourcePinholeIntrinsics`, `ResourceVec3`, `ResourceQuat`, `ResourceSpatialTransform`) also need `uniffi::Record` because they appear as fields.

One field is challenging: `TransformEdgeResource::source: Option<serde_json::Value>`. `serde_json::Value` isn't FFI-lowerable. The Python binding flattens this to `source_json: Option<String>` (serialized) at the boundary. Plan C does the same — add a `#[cfg_attr(feature = "swift-bindings", uniffi(skip))]` on the original `source` field and add a `source_json: Option<String>` companion field that serializes/deserializes via custom getter logic.

**Simpler approach:** since UniFFI can't `skip` a single field on a Record, instead define a SHIM record `TransformEdgeResourceSwift` in the binding crate and add a `From<TransformEdgeResource> for TransformEdgeResourceSwift` conversion. That avoids polluting the upstream record's shape.

Actually the cleanest path: change `TransformEdgeResource::source` from `Option<serde_json::Value>` to `Option<String>` (storing the canonical JSON serialization) upstream too. This is a small breaking change but the Python binding already canonicalized to a JSON string at the FFI boundary; matching upstream to that representation simplifies both bindings.

**Decision for Plan C:** Use the binding-crate shim path. Define `TransformEdgeResourceSwift` in the binding crate, annotate the upstream `TransformEdgeResource` as `uniffi::Record` with a custom `lower`/`lift` (`#[uniffi(with = "...")]` per UniFFI 0.31). Actually UniFFI 0.31's per-field `with` attribute might not be available. Fall back: add a parallel `ResourceEntrySwift` enum in the binding crate that wraps each variant with the Swift-friendly representation.

To keep the plan progressing, **Step 1** of this task is to inspect the actual shape and pick the implementation path. If UniFFI 0.31 supports per-field custom lower/lift, use it; otherwise, do the binding-crate shim.

- [ ] **Step 1: Inspect the shape**

Read `crates/auki-network/src/resources_protocol.rs` and locate:
- `pub enum ResourceEntry { ... }`
- `pub struct SensorStreamResource { ... }`
- `pub struct TransformEdgeResource { ... }`
- `pub struct ResourcePinholeIntrinsics { ... }`
- `pub struct ResourceVec3 { ... }`
- `pub struct ResourceQuat { ... }`
- `pub struct ResourceSpatialTransform { ... }`

Note which fields use non-FFI types (`serde_json::Value`, custom enums, etc.).

- [ ] **Step 2: Annotate geometry records**

For each of `ResourcePinholeIntrinsics`, `ResourceVec3`, `ResourceQuat`, `ResourceSpatialTransform`, add above the existing derives:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

All have flat `f64` (or composite) fields and should annotate cleanly.

- [ ] **Step 3: Annotate `SensorStreamResource`**

Add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` above the existing derives. If any field is a non-FFI type (e.g. `Option<serde_json::Value>`), report BLOCKED and we'll add a shim. The plan assumes the 9 fields are all FFI-friendly (matches Python binding's expectations of flat-able fields).

- [ ] **Step 4: Annotate or shim `TransformEdgeResource`**

If `source: Option<serde_json::Value>` exists, the implementer chooses one of:
- (a) Change the upstream field to `Option<String>` (canonical JSON string).
- (b) Add a binding-crate shim record `TransformEdgeResourceSwift` and a conversion.

Going with (a) because it simplifies the upstream and the Python binding already pre-serializes this field. The change is small and additive (the canonical JSON value is what gets compared/checked anyway). Make the upstream field `Option<String>`; update internal usages of `source` to deserialize via `serde_json::from_str` when needed.

If the upstream impl uses the `Value` for direct comparison or programmatic manipulation, fall back to path (b) and implement the shim.

Then add:
```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

- [ ] **Step 5: Annotate `ResourceEntry`**

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq /* preserve existing derives */)]
pub enum ResourceEntry {
    SensorStream(SensorStreamResource),
    TransformEdge(TransformEdgeResource),
}
```

- [ ] **Step 6: Annotate `ResourcesResponse`**

In the same file, `pub struct ResourcesResponse { pub resources: Vec<ResourceEntry> }`. Annotate:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
```

- [ ] **Step 7: Verify build**

```bash
cargo build --features swift-bindings -p auki-network
cargo build -p auki-domain-swift
```

Expected: PASS for both.

- [ ] **Step 8: Add a test exercising the new types**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn resource_entry_variants_construct() {
        use auki_network::resources_protocol::*;
        let stream_resource = ResourceEntry::SensorStream(SensorStreamResource {
            sensor_id: "audio-1".to_string(),
            sensor_hash: "hash-a".to_string(),
            // Set remaining fields to their natural defaults — the test
            // exercises shape, not contents. If the type changes, update
            // here.
            ..Default::default()
        });
        assert!(matches!(stream_resource, ResourceEntry::SensorStream(_)));
    }
```

If `SensorStreamResource` doesn't implement `Default`, derive it or hand-construct the test object with explicit fields.

```bash
cargo test -p auki-domain-swift resource_entry_variants_construct
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/auki-network/src/resources_protocol.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-network): annotate ResourceEntry + sub-records for swift-bindings"
```

---

### Task 7: Annotate `SensorRegistryEntry` + SensorBody tree

**Files:**
- Modify: `crates/auki-network/src/registries_protocol.rs`

User picked "typed records" over the canonical-JSON shortcut. So annotate the full SensorBody enum + all nested Camera/PointCloud/Audio/JointEncoders/PointField/PointFieldDataType types.

- [ ] **Step 1: Inspect the type tree**

Read the relevant region of `crates/auki-network/src/registries_protocol.rs`. Locate:
- `pub struct SensorRegistryEntry { sensor_id: String, body: SensorBody }`
- `pub enum SensorBody { Camera(...), PointCloud(...), Audio(...), JointEncoders(...) }`
- Plus the Camera, PointCloud, Audio, JointEncoders structs and any PointField, PointFieldDataType, etc.

- [ ] **Step 2: Annotate every leaf type**

For each struct in the tree (Camera, PointCloud, Audio, JointEncoders, PointField, etc.), add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` above existing derives.

For each leaf enum (PointFieldDataType etc.), add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]`.

For `SensorBody` itself, add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]`.

For `SensorRegistryEntry`, add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]`.

- [ ] **Step 3: Watch for serde tag attributes**

If `SensorBody` has `#[serde(tag = "type", content = "...")]` or similar serde-specific tags, UniFFI may not interact cleanly with them. UniFFI's enum representation is its own tagged-union shape, not driven by serde's. Adding `derive(uniffi::Enum)` alongside serde derives is fine — they don't conflict at the type level — but the WIRE representation Swift sees is UniFFI's, not the serde JSON shape. If Swift consumers also need the JSON shape, they encode/decode with serde_json.

- [ ] **Step 4: Verify build**

```bash
cargo build --features swift-bindings -p auki-network
cargo build -p auki-domain-swift
```

Expected: PASS. If UniFFI complains about field types in nested structs (e.g. an unusual integer type), report and address per-field.

- [ ] **Step 5: Add a test**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn sensor_registry_entry_camera_variant() {
        use auki_network::registries_protocol::*;
        // Construct a Camera-variant SensorRegistryEntry. Exact field
        // construction depends on the upstream shape — exercise the
        // type-level shape only.
        let entry = SensorRegistryEntry {
            sensor_id: "cam-1".to_string(),
            body: SensorBody::Camera(Default::default()),
        };
        assert!(matches!(entry.body, SensorBody::Camera(_)));
    }
```

```bash
cargo test -p auki-domain-swift sensor_registry_entry_camera_variant
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/src/registries_protocol.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-network): annotate SensorRegistryEntry + SensorBody tree"
```

---

### Task 8: Annotate `ClockRegistryEntry` + ClockBody tree

**Files:**
- Modify: `crates/auki-network/src/registries_protocol.rs`

Same pattern as Task 7 for the clock tree. `ClockRegistryEntry { clock_id, body }` + `ClockBody::MonotonicClock(ClockMeta) | UtcClock(ClockMeta)` + `ClockMeta { unit, monotonic, epoch, scope }` + `Scope` enum.

- [ ] **Step 1: Inspect the type tree**

Find `ClockRegistryEntry`, `ClockBody`, `ClockMeta`, `Scope` in `crates/auki-network/src/registries_protocol.rs`.

- [ ] **Step 2: Annotate each type**

For each, add the appropriate annotation:

- `Scope` (enum): `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]`
- `ClockMeta` (struct): `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]`
- `ClockBody` (enum): `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]`
- `ClockRegistryEntry` (struct): `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]`

- [ ] **Step 3: Verify build**

```bash
cargo build --features swift-bindings -p auki-network
cargo build -p auki-domain-swift
```

Expected: PASS.

- [ ] **Step 4: Test**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn clock_registry_entry_variants_construct() {
        use auki_network::registries_protocol::*;
        let mono = ClockRegistryEntry {
            clock_id: "wallclock".to_string(),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".to_string(),
                monotonic: true,
                epoch: None,
                scope: Scope::default(),
            }),
        };
        assert!(matches!(mono.body, ClockBody::MonotonicClock(_)));
    }
```

```bash
cargo test -p auki-domain-swift clock_registry_entry_variants_construct
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/registries_protocol.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-network): annotate ClockRegistryEntry + ClockBody tree"
```

---

### Task 9: Annotate `FrameRegistryEntry` + Handedness/AxisConvention + `DetectorRegistryEntry`

**Files:**
- Modify: `crates/auki-network/src/registries_protocol.rs`

- `FrameRegistryEntry { frame_id, handedness, axes, units }`
- `Handedness::Right | Left`
- `AxisConvention { x, y, z: AxisDirection }`
- `AxisDirection` — 6-variant enum
- `LengthUnit::Meters | Millimeters | Centimeters`
- `DetectorRegistryEntry` — Cuba T4 addition (matches the Python registry-fetch surface)

- [ ] **Step 1: Inspect + annotate**

Find each in `crates/auki-network/src/registries_protocol.rs`:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
pub enum Handedness { Right, Left }

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
pub enum AxisDirection { /* 6 variants */ }

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
pub enum LengthUnit { Meters, Millimeters, Centimeters }

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
pub struct AxisConvention { /* x, y, z: AxisDirection */ }

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
pub struct FrameRegistryEntry { /* fields */ }

#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]
pub struct DetectorRegistryEntry { /* fields */ }
```

- [ ] **Step 2: Verify build**

```bash
cargo build --features swift-bindings -p auki-network
cargo build -p auki-domain-swift
```

Expected: PASS.

- [ ] **Step 3: Test**

```rust
    #[test]
    fn frame_registry_entry_constructs() {
        use auki_network::registries_protocol::*;
        let entry = FrameRegistryEntry {
            frame_id: "world".to_string(),
            handedness: Handedness::Right,
            axes: AxisConvention::default(),
            units: LengthUnit::Meters,
        };
        assert_eq!(entry.handedness, Handedness::Right);
    }
```

```bash
cargo test -p auki-domain-swift frame_registry_entry_constructs
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-network/src/registries_protocol.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-network): annotate FrameRegistryEntry + DetectorRegistryEntry trees"
```

---

### Task 10: Annotate `DiagnosticMessage` + `InboundDiagnosticMessage`

**Files:**
- Modify: `crates/auki-network/src/diagnostic_protocol.rs`

User chose to include diagnostics. Annotate the two public types.

- [ ] **Step 1: Inspect**

Find `pub struct DiagnosticMessage` and `pub struct InboundDiagnosticMessage` (or `pub enum DiagnosticMessage` — check the actual shape) in `crates/auki-network/src/diagnostic_protocol.rs`.

- [ ] **Step 2: Annotate**

Add `#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]` (or `uniffi::Enum` if it's an enum) above the existing derives on each.

- [ ] **Step 3: Verify + test + commit**

```bash
cargo build --features swift-bindings -p auki-network
cargo build -p auki-domain-swift
```

Add a small constructor test if appropriate, then:

```bash
git add crates/auki-network/src/diagnostic_protocol.rs
git commit -m "feat(auki-network): annotate DiagnosticMessage for swift-bindings"
```

---

### Task 11: Annotate clock sync types in `auki-time`

**Files:**
- Modify: `crates/auki-time/src/lib.rs` (or wherever ClockTransformEstimate / DomainClockEstimate live)
- Possibly modify: `crates/auki-time/Cargo.toml` (add `swift-bindings` feature if not present)

User chose to include the 4 clock-sync methods (`domain_clock_estimate`, `domain_time_now`, `clock_sync_estimate`, `clock_sync_estimates`) — so the return types and error types they reference need UniFFI annotations.

- [ ] **Step 1: Locate the types**

```bash
grep -nE "pub (struct|enum) (ClockTransformEstimate|DomainClockEstimate|DomainClockEstimateError)" crates/auki-time/src/*.rs
```

These types may live in submodules. The plan assumes they're in `crates/auki-time/src/lib.rs` or `crates/auki-time/src/transform.rs`.

- [ ] **Step 2: Add `swift-bindings` feature to `crates/auki-time/Cargo.toml`**

If the crate doesn't already have a `swift-bindings` feature, mirror the pattern from Tasks 1 (auki-domain) / PR A (auki-identity): add `swift-bindings = ["dep:uniffi", "dep:anyhow"]` to `[features]`, and `uniffi`/`anyhow` as optional deps.

If `auki-time` types are already annotated for some other binding (Python?), confirm whether the existing annotations work for UniFFI 0.31 — if so, just propagate the `swift-bindings` feature dep on `auki-domain`'s feature: `swift-bindings = [..., "auki-time/swift-bindings"]` (extend Task 1's feature list).

- [ ] **Step 3: Add `uniffi::setup_scaffolding!()` (if adding fresh)**

In `crates/auki-time/src/lib.rs`:

```rust
#[cfg(feature = "swift-bindings")]
uniffi::setup_scaffolding!();
```

- [ ] **Step 4: Annotate the types**

For each of `ClockTransformEstimate`, `DomainClockEstimate`, `DomainClockEstimateError`, plus any nested types they reference (e.g. `SourceClockId`):

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Record))]    // if struct
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]      // if enum
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]     // if error enum
```

- [ ] **Step 5: Update Task 1's feature list**

In `crates/auki-domain/Cargo.toml`, extend the `swift-bindings` feature:

```toml
swift-bindings = [
    "dep:uniffi",
    "dep:anyhow",
    "auki-identity/swift-bindings",
    "auki-network/swift-bindings",
    "auki-time/swift-bindings",
]
```

- [ ] **Step 6: Verify + test + commit**

```bash
cargo build --features swift-bindings -p auki-time
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
```

```bash
git add crates/auki-time/Cargo.toml crates/auki-time/src/lib.rs crates/auki-domain/Cargo.toml
git commit -m "feat(auki-time,auki-domain): propagate swift-bindings feature; annotate clock-sync types"
```

---

### Task 12: Annotate `BootstrapError` + `CreateClusterError` + `JoinClusterError` + `AdmitError`

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

Bootstrap-family errors. All wrap nested error types (DiscoveryError, SendJoinRequestError, SpawnError, serde_json::Error, etc.). Use `flat_error` per the spec — non-FFI variants surface as Display'd `message: String`.

- [ ] **Step 1: Annotate each error**

Locate each `pub enum *Error` in `crates/auki-domain/src/cluster_manager.rs`. Add above the existing derives:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
```

Apply to:
- `BootstrapError`
- `CreateClusterError`
- `JoinClusterError`
- `AdmitError`

- [ ] **Step 2: Verify build**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
```

Expected: PASS.

- [ ] **Step 3: Add Display tests**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    #[test]
    fn bootstrap_family_errors_display_clean() {
        use auki_domain_rs::cluster_manager::*;
        let e1: BootstrapError = BootstrapError::Rejected("test reason".to_string());
        assert!(!e1.to_string().is_empty());
        let e2: AdmitError = AdmitError::Stopped;
        assert!(!e2.to_string().is_empty());
    }
```

Adjust variant names if they differ from the survey.

```bash
cargo test -p auki-domain-swift bootstrap_family_errors_display_clean
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain): annotate bootstrap-family errors for UniFFI export"
```

---

### Task 13: Annotate `Fetch*Error` errors + `BuildStreamManifestError`

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`
- Modify: `crates/auki-domain/src/stream_manifest.rs`

Catalog/registry fetch errors and the stream manifest build error. Same flat_error pattern.

- [ ] **Step 1: Annotate each**

In `crates/auki-domain/src/cluster_manager.rs`:
- `FetchSensorsCatalogError`
- `FetchResourcesCatalogError`
- `FetchRegistryEntryError`
- `FetchParticipantInfoError`

In `crates/auki-domain/src/stream_manifest.rs`:
- `BuildStreamManifestError`

For each:
```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
```

- [ ] **Step 2: Verify + commit**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
git add crates/auki-domain/src/cluster_manager.rs crates/auki-domain/src/stream_manifest.rs
git commit -m "feat(auki-domain): annotate fetch-error types for UniFFI export"
```

---

### Task 14: Annotate diagnostic + clock-sync error types

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs` (BroadcastDiagnosticError — may already be annotated; check)
- Modify: `crates/auki-time/src/lib.rs` (DomainClockEstimateError if not done in Task 11)
- Modify: `crates/auki-domain/src/cluster_manager.rs` (DomainTimeNowError, DomainClockEstimateUnavailable)

- [ ] **Step 1: Check `BroadcastDiagnosticError`**

```bash
grep -A2 "pub enum BroadcastDiagnosticError" crates/auki-network/src/network_runtime.rs
```

If not annotated, add:
```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Error))]
#[cfg_attr(feature = "swift-bindings", uniffi(flat_error))]
```

- [ ] **Step 2: Annotate `DomainTimeNowError` and `DomainClockEstimateUnavailable`**

Find each in `crates/auki-domain/src/cluster_manager.rs`. Add the same pair of cfg_attr annotations.

- [ ] **Step 3: Verify + commit**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
git add crates/auki-domain/src/cluster_manager.rs crates/auki-network/src/network_runtime.rs
git commit -m "feat(auki-domain): annotate diagnostic + clock-sync errors"
```

---

### Task 15: Annotate `ClusterTarget` enum + 4 static constructors

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

`ClusterTarget` is a 4-variant enum with one `String` field per variant. Static factory methods (`create(name)`, `join(name)`, `join_or_create(name)`, `most_recent_or_create(fallback_name)`) need UniFFI constructor annotations.

- [ ] **Step 1: Annotate the enum**

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Enum))]
#[derive(Debug, Clone, PartialEq, Eq /* preserve existing */)]
pub enum ClusterTarget {
    Create { name: String },
    Join { name: String },
    JoinOrCreate { name: String },
    MostRecentOrCreate { fallback_name: String },
}
```

If existing variants use tuple form `Create(String)` vs struct form `Create { name: String }`, preserve the existing form and add the cfg_attr. UniFFI supports both shapes.

- [ ] **Step 2: Annotate the 4 static factory methods**

If `impl ClusterTarget` exists with the 4 static methods:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl ClusterTarget {
    pub fn create(name: String) -> Self { Self::Create { name } }
    pub fn join(name: String) -> Self { Self::Join { name } }
    pub fn join_or_create(name: String) -> Self { Self::JoinOrCreate { name } }
    pub fn most_recent_or_create(fallback_name: String) -> Self { Self::MostRecentOrCreate { fallback_name } }
}
```

Note: in UniFFI, `derive(uniffi::Enum)` already generates a constructor per variant — Swift gets `ClusterTarget.create(name:)` etc. for free. The standalone factory methods may be redundant. Verify by inspecting the generated Swift surface in Task 24's XCFramework validation; if the variant constructors are sufficient, drop the standalone methods (don't add the `uniffi::export` annotation).

- [ ] **Step 3: Test**

```rust
    #[test]
    fn cluster_target_variants_construct() {
        use auki_domain_rs::cluster_manager::*;
        let t = ClusterTarget::Create { name: "test".to_string() };
        assert!(matches!(t, ClusterTarget::Create { .. }));
    }
```

```bash
cargo test -p auki-domain-swift cluster_target_variants_construct
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain): annotate ClusterTarget as UniFFI Enum"
```

---

### Task 16: Annotate `SensorCatalogProvider` + `ResourceCatalogProvider` as callback interfaces

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

Both upstream traits become Swift-implementable protocols.

- [ ] **Step 1: Read current trait shapes**

```bash
grep -B1 -A6 "pub trait SensorCatalogProvider\|pub trait ResourceCatalogProvider" crates/auki-domain/src/cluster_manager.rs
```

Expected: each is `pub trait XCatalogProvider: Send + Sync + 'static { fn snapshot(&self) -> Vec<XEntry>; fn snapshot_for_request(...) -> Vec<XEntry> { /* default impl */ } }`.

- [ ] **Step 2: Annotate**

Add `#[cfg_attr(feature = "swift-bindings", uniffi::export(callback_interface))]` above each trait declaration.

UniFFI 0.31's callback interface treatment: only methods on the trait are exposed. Default-impl methods are kept Rust-side. Swift implementations of the protocol provide `snapshot()`; the default `snapshot_for_request` continues to be the Rust default.

If `snapshot_for_request` takes a `&Path` parameter, UniFFI can't lower a borrowed Path — but since it's a default method (not part of the callback interface), Swift implementations don't see it. The trait declaration stays as-is.

- [ ] **Step 3: Verify build**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
```

Expected: PASS. If UniFFI complains about the trait bounds (`Send + Sync + 'static`), confirm UniFFI 0.31's callback-interface contract — it should be compatible with these bounds.

- [ ] **Step 4: Add object-safety smoke test**

```rust
    #[test]
    fn sensor_catalog_provider_is_object_safe() {
        struct NoopProvider;
        impl auki_domain_rs::cluster_manager::SensorCatalogProvider for NoopProvider {
            fn snapshot(&self) -> Vec<auki_network::sensors_protocol::SensorEntry> {
                vec![]
            }
        }
        let _p: Box<dyn auki_domain_rs::cluster_manager::SensorCatalogProvider> = Box::new(NoopProvider);
    }
```

```bash
cargo test -p auki-domain-swift sensor_catalog_provider_is_object_safe
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain): annotate catalog provider traits as UniFFI callback interfaces"
```

---

### Task 17: Annotate `ClusterManager` Object + identity accessors + simple sync methods

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

Annotate the struct as `uniffi::Object`. Annotate the identity accessor methods + simple sync methods in their own `#[uniffi::export]` impl block.

Methods in this task (sync, no async): `cluster_name`, `local_peer_id_string`, `local_multiaddrs_strings`, `manager_peer_id_string`, `is_manager`, `peer_count`, `membership`, `participant_info`, `set_registry_app_root`, `shutdown` (note: async on upstream — handle separately if needed).

The `_string` / `_strings` suffix pattern matches PR B's convention for PeerId → String conversion. Add helper methods that return String instead of PeerId for the Swift-friendly surface; keep the existing typed methods for Rust callers.

- [ ] **Step 1: Annotate `ClusterManager`**

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct ClusterManager {
    // existing fields verbatim
}
```

- [ ] **Step 2: Add the UniFFI-exposed impl block**

Add a new impl block near the existing ones:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl ClusterManager {
    /// Cluster name (the human-readable Discovery id).
    pub fn cluster_name_str(&self) -> String {
        self.cluster_name().to_string()
    }

    /// Canonical libp2p peer-id string for this Manager's local peer.
    pub fn local_peer_id_string(&self) -> String {
        self.local_peer_id().to_string()
    }

    /// Local listen multiaddrs as canonical strings.
    pub fn local_multiaddr_strings(&self) -> Vec<String> {
        self.local_multiaddrs().iter().map(|m| m.to_string()).collect()
    }

    /// Manager peer-id (current Manager — may be self or another peer).
    pub fn manager_peer_id_string(&self) -> String {
        self.manager_peer_id().to_string()
    }

    /// True iff this peer is the current cluster Manager.
    pub fn is_manager(&self) -> bool {
        // delegate to existing impl
        Self::is_manager(self)
    }

    /// Current peer count snapshot (including self).
    pub fn peer_count(&self) -> usize {
        // delegate to existing impl — note: usize may need conversion to u32
        // for FFI. If UniFFI rejects usize, change to u32 here.
        Self::peer_count(self)
    }

    /// Snapshot of cluster membership.
    pub fn membership(&self) -> auki_domain_rs::cluster_membership::ClusterMembership {
        Self::membership(self)
    }

    /// Local ParticipantInfo snapshot.
    pub fn participant_info(&self) -> auki_network::ParticipantInfo {
        Self::participant_info(self)
    }

    /// Configure registry app root (for resolving registry entries to disk).
    /// Takes a UTF-8 path string; Swift sends a String.
    pub fn set_registry_app_root(&self, app_root: String) {
        Self::set_registry_app_root(self, app_root);
    }
}
```

**Important:** the `is_manager` and `peer_count` methods would conflict with the underlying typed methods if the names are reused on `ClusterManager` directly. The trick of calling `Self::method(self)` won't work because Rust resolves to the same method (recursion).

**Cleaner approach:** delete the standalone Rust-side accessors from their current impl block (move their bodies into the annotated block), OR add the annotated block as an `impl ClusterManager` with `#[uniffi::export]` that calls into helper-named methods (e.g. `is_manager_inner`).

The cleanest path is the first: move each method body verbatim into the annotated impl block, and delete the original definition. Rust callers continue to call `manager.is_manager()` and `manager.peer_count()` — same name, same behavior. The `_string` / `_strings` helpers are NEW (additive) — keep the typed `local_peer_id() -> PeerId` etc. in the un-annotated block.

Rewrite the annotated block (this is the production version):

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl ClusterManager {
    /// Cluster name.
    pub fn cluster_name(&self) -> String {
        self.cluster_name.clone()  // assuming field access
    }

    /// Canonical libp2p peer-id string for the local peer.
    pub fn local_peer_id_string(&self) -> String {
        self.local_peer_id().to_string()
    }

    /// Local listen multiaddrs as canonical strings.
    pub fn local_multiaddr_strings(&self) -> Vec<String> {
        self.local_multiaddrs().iter().map(|m| m.to_string()).collect()
    }

    /// Current Manager peer-id as canonical string.
    pub fn manager_peer_id_string(&self) -> String {
        self.manager_peer_id().to_string()
    }

    pub fn is_manager(&self) -> bool {
        // existing body
    }

    pub fn peer_count_u32(&self) -> u32 {
        self.peer_count() as u32
    }

    pub fn membership(&self) -> ClusterMembership {
        // existing body
    }

    pub fn participant_info(&self) -> auki_network::ParticipantInfo {
        // existing body
    }

    pub fn set_registry_app_root(&self, app_root: String) {
        // existing body adapted to accept String
    }
}
```

The implementer is to: MOVE each existing method body from its original impl block to this new annotated block, and DELETE the original definition. Rust callers see no behavioral change.

Note: the `cluster_name` existing method returns `&str`. The annotated version must return owned `String` (UniFFI 0.31 can't lower `&str`). Either:
- (a) Move + change to `String`. Internal callers update to use `.as_str()` if they need a borrow.
- (b) Keep the existing `&str` method un-annotated; add a `cluster_name_str()` annotated wrapper that calls it.

Choose (b) — preserves the typed Rust API.

Same for `local_peer_id() -> PeerId`, `local_multiaddrs() -> &[Multiaddr]`, `manager_peer_id() -> PeerId`. Add the `_string` / `_strings` variants as new annotated methods; keep the typed ones un-annotated.

For `is_manager` / `peer_count` / `membership` / `participant_info`: these return native types that don't conflict — move them into the annotated block (or, for `peer_count`, expose as `peer_count() -> u32` via a wrapper if `usize` is rejected by UniFFI 0.31).

- [ ] **Step 3: Verify build**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
```

Expected: PASS. If `usize` is rejected, change `peer_count` to `peer_count_u32` and update callers in `auki-domain` internal code accordingly. If `&[Multiaddr]` to `Vec<String>` conversion is rejected, ensure the new annotated method does `.iter().map(|m| m.to_string()).collect()`.

- [ ] **Step 4: Add ClusterManager Send+Sync type-level test**

```rust
    #[test]
    fn cluster_manager_is_uniffi_object() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<auki_domain_rs::cluster_manager::ClusterManager>();
    }
```

```bash
cargo test -p auki-domain-swift cluster_manager_is_uniffi_object
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain): annotate ClusterManager Object + identity/simple sync surface"
```

---

### Task 18: Annotate ClusterManager async fetch + admit methods

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

Async methods exposed via `#[uniffi::export(async_runtime = "tokio")]` in their own annotated impl block:

- `bootstrap` / `create_cluster` / `join_cluster` — NOT exposed directly (Swift uses the binding-crate `*_swift` adapters in Task 22; the upstream methods stay un-annotated).
- `admit_peer(peer_id: PeerId, multiaddrs: Vec<Multiaddr>) -> Result<ClusterMember, AdmitError>` — exposed.
- `fetch_participant_info(peer_id: PeerId) -> Result<ParticipantInfo, FetchParticipantInfoError>` — exposed.
- `fetch_sensors_catalog(peer_id: PeerId) -> Result<SensorsResponse, FetchSensorsCatalogError>` — exposed.
- `fetch_sensors_catalog_with(peer_id: PeerId, request: SensorsRequest) -> Result<SensorsResponse, FetchSensorsCatalogError>` — exposed.
- `fetch_resources_catalog(peer_id: PeerId) -> Result<ResourcesResponse, FetchResourcesCatalogError>` — exposed.
- `fetch_resources_catalog_with(peer_id: PeerId, request: ResourcesRequest) -> Result<ResourcesResponse, FetchResourcesCatalogError>` — exposed.
- `fetch_sensor_entry(peer_id: PeerId, sensor_id: String, sensor_hash: String) -> Result<SensorRegistryEntry, FetchRegistryEntryError>` — exposed.
- `fetch_clock_entry(peer_id: PeerId, clock_id: String, clock_hash: String) -> Result<ClockRegistryEntry, FetchRegistryEntryError>` — exposed.
- `fetch_frame_entry(peer_id: PeerId, frame_id: String, frame_hash: String) -> Result<FrameRegistryEntry, FetchRegistryEntryError>` — exposed.
- `fetch_detector_entry(peer_id: PeerId, detector_id: String, detector_hash: String) -> Result<DetectorRegistryEntry, FetchRegistryEntryError>` — exposed.
- `shutdown() -> Result<(), DiscoveryError>` — exposed (async).

Many existing methods take `impl Into<String>` parameters. UniFFI requires concrete `String`. Either:
- (a) Change the upstream signature to take `String` directly (additive — `String: Into<String>` so existing callers still work but with one fewer auto-coercion).
- (b) Add `_swift` wrapper methods in a separate impl block.

Path (a) is cleaner. The implementer changes each `impl Into<String>` parameter to `String` and verifies internal callers still work (most should — `String::from(s)` and direct `String` arguments are universal).

- [ ] **Step 1: Locate the methods**

```bash
grep -nE "pub (async )?fn (admit_peer|fetch_|shutdown)" crates/auki-domain/src/cluster_manager.rs
```

- [ ] **Step 2: Convert `impl Into<String>` parameters**

For each method that takes `impl Into<String>`, change to `String` directly. Update the method body to use `.into()` calls if necessary (e.g. `let sensor_id: String = sensor_id;` is a no-op; existing `impl Into<String>` -> `let _: String = sensor_id.into();` patterns become just `sensor_id`).

- [ ] **Step 3: Move methods into an annotated impl block**

Add a new annotated impl block:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl ClusterManager {
    pub async fn admit_peer(
        &self,
        peer_id: PeerId,
        multiaddrs: Vec<Multiaddr>,
    ) -> Result<ClusterMember, AdmitError> {
        // existing body
    }

    pub async fn fetch_participant_info(
        &self,
        peer_id: PeerId,
    ) -> Result<auki_network::ParticipantInfo, FetchParticipantInfoError> {
        // existing body
    }

    pub async fn fetch_sensors_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<auki_network::sensors_protocol::SensorsResponse, FetchSensorsCatalogError> {
        // existing body
    }

    pub async fn fetch_sensors_catalog_with(
        &self,
        peer_id: PeerId,
        request: auki_network::sensors_protocol::SensorsRequest,
    ) -> Result<auki_network::sensors_protocol::SensorsResponse, FetchSensorsCatalogError> {
        // existing body
    }

    pub async fn fetch_resources_catalog(
        &self,
        peer_id: PeerId,
    ) -> Result<auki_network::resources_protocol::ResourcesResponse, FetchResourcesCatalogError> {
        // existing body
    }

    pub async fn fetch_resources_catalog_with(
        &self,
        peer_id: PeerId,
        request: auki_network::resources_protocol::ResourcesRequest,
    ) -> Result<auki_network::resources_protocol::ResourcesResponse, FetchResourcesCatalogError> {
        // existing body
    }

    pub async fn fetch_sensor_entry(
        &self,
        peer_id: PeerId,
        sensor_id: String,
        sensor_hash: String,
    ) -> Result<auki_network::registries_protocol::SensorRegistryEntry, FetchRegistryEntryError> {
        // existing body adapted to use String parameters directly
    }

    pub async fn fetch_clock_entry(
        &self,
        peer_id: PeerId,
        clock_id: String,
        clock_hash: String,
    ) -> Result<auki_network::registries_protocol::ClockRegistryEntry, FetchRegistryEntryError> {
        // existing body
    }

    pub async fn fetch_frame_entry(
        &self,
        peer_id: PeerId,
        frame_id: String,
        frame_hash: String,
    ) -> Result<auki_network::registries_protocol::FrameRegistryEntry, FetchRegistryEntryError> {
        // existing body
    }

    pub async fn fetch_detector_entry(
        &self,
        peer_id: PeerId,
        detector_id: String,
        detector_hash: String,
    ) -> Result<auki_network::registries_protocol::DetectorRegistryEntry, FetchRegistryEntryError> {
        // existing body
    }

    pub async fn shutdown(&self) -> Result<(), auki_network::discovery_client::DiscoveryError> {
        // existing body
    }
}
```

Each method's body moves from its existing un-annotated impl block to this new annotated block. Delete the original definitions.

- [ ] **Step 4: Verify build + run upstream tests**

```bash
cargo build --features swift-bindings -p auki-domain
cargo test --features swift-bindings -p auki-domain --lib
cargo build -p auki-domain-swift
```

Expected: PASS for all. If upstream tests fail, the `impl Into<String>` → `String` change may have broken a call site that needed the auto-coercion. Update the call site to `s.to_string()` or `s.into()` as needed.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs
git commit -m "feat(auki-domain): annotate ClusterManager async fetch + admit methods"
```

---

### Task 19: Annotate ClusterManager clock-sync, diagnostic, provider-setter, and 5 per-payload open_*_stream methods

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`

Final ClusterManager method set:

**Clock sync (sync methods):**
- `clock_sync_estimate(local_clock_id: String, remote_clock_id: String) -> Option<ClockTransformEstimate>`
- `clock_sync_estimates() -> Vec<ClockTransformEstimate>`
- `domain_clock_estimate() -> Result<DomainClockEstimate, DomainClockEstimateUnavailable>`
- `domain_time_now() -> Result<i64, DomainTimeNowError>`

**Diagnostics (sync methods):**
- `broadcast_diagnostic_message(message: DiagnosticMessage) -> Result<(), BroadcastDiagnosticError>`
- `drain_diagnostic_messages() -> Vec<InboundDiagnosticMessage>`

**Provider setters (sync):**
- `set_sensor_catalog_provider(provider: Box<dyn SensorCatalogProvider>)` — Box per UniFFI 0.31 callback-interface contract; internally promote to Arc.
- `set_resource_catalog_provider(provider: Box<dyn ResourceCatalogProvider>)` — same.

**Per-payload streams (async):**
- `open_audio_stream(peer_id: PeerId, request_bytes: Vec<u8>) -> Result<Arc<StreamSubscriptionAudio>, OpenStreamError>`
- 4 more for Camera / PointCloud / JointEncoders / Detection

Each `open_*_stream` is a thin delegator to `NetworkRuntime::open_*_stream` (already annotated in PR B). The ClusterManager-level methods give Swift consumers a single Object to interact with for all streams (avoids needing them to reach the underlying NetworkRuntime).

- [ ] **Step 1: Add the sync annotated impl block**

Append in `crates/auki-domain/src/cluster_manager.rs`:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl ClusterManager {
    pub fn clock_sync_estimate(
        &self,
        local_clock_id: String,
        remote_clock_id: String,
    ) -> Option<auki_time::ClockTransformEstimate> {
        Self::clock_sync_estimate(self, &local_clock_id, &remote_clock_id)
    }

    pub fn clock_sync_estimates(&self) -> Vec<auki_time::ClockTransformEstimate> {
        Self::clock_sync_estimates(self)
    }

    pub fn domain_clock_estimate(
        &self,
    ) -> Result<auki_time::DomainClockEstimate, DomainClockEstimateUnavailable> {
        Self::domain_clock_estimate(self)
    }

    pub fn domain_time_now(&self) -> Result<i64, DomainTimeNowError> {
        Self::domain_time_now(self)
    }

    pub fn broadcast_diagnostic_message(
        &self,
        message: auki_network::diagnostic_protocol::DiagnosticMessage,
    ) -> Result<(), auki_network::network_runtime::BroadcastDiagnosticError> {
        Self::broadcast_diagnostic_message(self, message)
    }

    pub fn drain_diagnostic_messages(
        &self,
    ) -> Vec<auki_network::diagnostic_protocol::InboundDiagnosticMessage> {
        Self::drain_diagnostic_messages(self)
    }

    pub fn set_sensor_catalog_provider(&self, provider: Box<dyn SensorCatalogProvider>) {
        let provider: Arc<dyn SensorCatalogProvider> = Arc::from(provider);
        Self::set_sensor_catalog_provider(self, provider);
    }

    pub fn set_resource_catalog_provider(&self, provider: Box<dyn ResourceCatalogProvider>) {
        let provider: Arc<dyn ResourceCatalogProvider> = Arc::from(provider);
        Self::set_resource_catalog_provider(self, provider);
    }
}
```

The `Self::method(self, ...)` indirection invokes the existing un-annotated method body. For methods like `clock_sync_estimate` where the existing impl takes borrowed `&str` parameters, the wrapper takes owned `String` and passes refs to the inner.

If `Self::method(self, ...)` syntax doesn't work for instance methods (UniFFI proc-macros might generate a method also named `method`, causing conflict), an alternative: rename the un-annotated methods to `*_inner` and have the annotated method call the renamed inner. This is mechanical refactoring — the implementer chooses the cleanest path during execution.

- [ ] **Step 2: Add the async open_*_stream block**

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export(async_runtime = "tokio"))]
impl ClusterManager {
    pub async fn open_audio_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<auki_network::network_runtime::StreamSubscriptionAudio>, auki_network::network_runtime::OpenStreamError> {
        // Delegate to the underlying NetworkRuntime's annotated method.
        self.network_runtime().open_audio_stream(peer_id, request_bytes).await
    }

    pub async fn open_camera_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<auki_network::network_runtime::StreamSubscriptionCamera>, auki_network::network_runtime::OpenStreamError> {
        self.network_runtime().open_camera_stream(peer_id, request_bytes).await
    }

    pub async fn open_pointcloud_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<auki_network::network_runtime::StreamSubscriptionPointCloud>, auki_network::network_runtime::OpenStreamError> {
        self.network_runtime().open_pointcloud_stream(peer_id, request_bytes).await
    }

    pub async fn open_joint_encoders_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<auki_network::network_runtime::StreamSubscriptionJointEncoders>, auki_network::network_runtime::OpenStreamError> {
        self.network_runtime().open_joint_encoders_stream(peer_id, request_bytes).await
    }

    pub async fn open_detection_stream(
        &self,
        peer_id: PeerId,
        request_bytes: Vec<u8>,
    ) -> Result<Arc<auki_network::network_runtime::StreamSubscriptionDetection>, auki_network::network_runtime::OpenStreamError> {
        self.network_runtime().open_detection_stream(peer_id, request_bytes).await
    }
}
```

`self.network_runtime()` is an accessor that returns `&NetworkRuntime` — if the existing API uses a differently-named field/method, replace accordingly. The implementer inspects the actual struct shape.

- [ ] **Step 3: Verify build + tests**

```bash
cargo build --features swift-bindings -p auki-domain
cargo build -p auki-domain-swift
cargo test --features swift-bindings -p auki-domain --lib
```

Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs
git commit -m "feat(auki-domain): annotate clock-sync, diagnostic, provider-setter, and stream methods"
```

---

### Task 20: Re-export upstream-annotated types from the binding crate

**Files:**
- Modify: `bindings/swift/auki-domain-swift/src/lib.rs`

Now that the upstream surface is fully annotated, the binding crate `pub use`s every type Swift consumers will reach for. Without these re-exports, Swift would need separate imports for each upstream crate; with them, a single `import AukiDomain` (or whatever the framework is called) suffices.

UniFFI metadata propagation: when the binding crate's `cargo build` links the upstream crates (with `swift-bindings` feature on), the UniFFI scaffolding metadata for the upstream types is included in the cdylib. uniffi-bindgen reads the metadata symbols and generates per-namespace `.swift` files. The `pub use` here is for Rust-side reachability — does NOT affect what UniFFI sees, which is determined by the upstream annotations.

- [ ] **Step 1: Add the re-exports**

In `bindings/swift/auki-domain-swift/src/lib.rs`, after the custom-type registrations:

```rust
// ─── Upstream type re-exports ──────────────────────────────────────
//
// Swift consumers reach these via the AukiDomain framework's umbrella
// module. The pub use re-exports keep Rust-side type paths short for
// the binding-crate adapter functions below.

pub use auki_domain_rs::cluster_manager::{
    AdmitError, BootstrapError, ClusterManager, ClusterTarget, CreateClusterError, DaemonInfo,
    DomainClockEstimateUnavailable, DomainTimeNowError, FetchParticipantInfoError,
    FetchRegistryEntryError, FetchResourcesCatalogError, FetchSensorsCatalogError,
    JoinClusterError, ResourceCatalogProvider, SensorCatalogProvider,
};
pub use auki_domain_rs::cluster_membership::{ClusterMember, ClusterMembership};
pub use auki_domain_rs::stream_manifest::BuildStreamManifestError;

pub use auki_network::AllowedPeer;
pub use auki_network::diagnostic_protocol::{DiagnosticMessage, InboundDiagnosticMessage};
pub use auki_network::discovery_client::{
    ClusterEntry, CreateClusterOutcome, DiscoveryClient, DiscoveryError,
};
pub use auki_network::network_runtime::{
    BroadcastDiagnosticError, OpenStreamError, StreamError, StreamEntry,
    StreamSubscriptionAudio, StreamSubscriptionCamera, StreamSubscriptionDetection,
    StreamSubscriptionJointEncoders, StreamSubscriptionPointCloud,
};
pub use auki_network::registries_protocol::{
    AxisConvention, AxisDirection, ClockBody, ClockMeta, ClockRegistryEntry, DetectorRegistryEntry,
    FrameRegistryEntry, Handedness, LengthUnit, RegistryKind, Scope, SensorBody,
    SensorRegistryEntry,
};
pub use auki_network::resources_protocol::{
    ResourceEntry, ResourcePinholeIntrinsics, ResourceQuat, ResourceSpatialTransform,
    ResourceVec3, ResourcesRequest, ResourcesResponse, SensorStreamResource,
    TransformEdgeResource,
};
pub use auki_network::sensors_protocol::{SensorEntry, SensorsRequest, SensorsResponse};
pub use auki_network::ParticipantInfo;

pub use auki_time::{ClockTransformEstimate, DomainClockEstimate};

// Swift callback-interface traits + StreamSubscription Swift glue
// re-exported from auki-network-swift (PR B). Swift consumers see them
// under the AukiDomain umbrella via the cross-crate dep.
pub use auki_network_swift::{
    HeartbeatTimestampProvider, PeerLivenessListener, StreamItem, SwiftAudioSource,
    SwiftCameraSource, SwiftDetectionSource, SwiftJointEncodersSource,
    SwiftPeerLivenessEvent, SwiftPointCloudSource, SwiftSourceError, SwiftStreamDecision,
    SwiftStreamProvider,
};
```

If any of these `pub use` paths fail (because the type isn't actually exported from the parent module), the implementer inspects the source and corrects. The list is comprehensive but may overstate — adjust as needed.

- [ ] **Step 2: Verify build**

```bash
cargo build -p auki-domain-swift
cargo test -p auki-domain-swift --lib
```

Expected: PASS. Build failures here indicate either:
- A `pub use` path is wrong (typo or wrong module).
- An upstream type was missed in annotation (Tasks 5-14).
- UniFFI's cross-crate metadata propagation needs `use_remote_type!` declarations (uncommon for plain `pub use`, but possible).

If `use_remote_type!` IS needed for any cross-crate type, declare it explicitly:

```rust
uniffi::use_remote_type!(auki_network_swift::SwiftStreamProvider);
```

- [ ] **Step 3: Commit**

```bash
git add bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain-swift): pub use upstream-annotated types under one umbrella"
```

---

### Task 21: Add `bootstrap_swift` orchestrator

**Files:**
- Modify: `bindings/swift/auki-domain-swift/src/lib.rs`

The Swift entry point for "join existing OR create" cluster bootstrap. Takes a wallet seed + listen multiaddrs + agent version + discovery URL + DaemonInfo + optional stream-provider callback (re-exported from PR B). Builds the libp2p swarm internally; constructs the `PeerIdentity` from the wallet; calls upstream `ClusterManager::bootstrap`.

The Python binding takes:
```
bootstrap(target, wallet_seed, discovery_url, listen_addresses, agent_version, daemon_info, stream_provider=None, external_addresses=None)
```

Swift mirrors this, with `wallet_seed` as `Vec<u8>` (32 bytes; the function validates length).

- [ ] **Step 1: Add a failing smoke test**

Append to `bindings/swift/auki-domain-swift/src/lib.rs` `mod tests`:

```rust
    /// `bootstrap_swift` builds a real swarm against an ephemeral local
    /// listen multiaddr and a fake Discovery URL; we expect Discovery
    /// to fail (no server) but the SWARM construction + identity
    /// derivation should succeed before we hit the network. So we test
    /// for a DiscoveryError in BootstrapError, not a swarm-build failure.
    #[tokio::test]
    async fn bootstrap_swift_swarm_construction_succeeds() {
        let result = bootstrap_swift(
            ClusterTarget::Create { name: "test-cluster".to_string() },
            vec![1u8; 32],  // 32-byte wallet seed
            "http://127.0.0.1:9".to_string(),  // unreachable Discovery URL
            vec!["/ip4/127.0.0.1/tcp/0".to_string()],
            "test-agent/0.0".to_string(),
            DaemonInfo {
                app: "test-app".to_string(),
                name: "test-instance".to_string(),
                session_id: "session-1".to_string(),
                session_clock_id: "clock-1".to_string(),
                session_clock_hash: "hash-1".to_string(),
                app_instance: "instance-1".to_string(),
            },
            None,  // no SwiftStreamProvider
            None,  // no external addresses
        ).await;

        // We expect a Discovery network failure, not a swarm-build or
        // identity-derivation failure. The exact error shape depends on
        // flat_error formatting — match on BootstrapError variant.
        match result {
            Err(BootstrapError::Discovery(_)) => { /* expected: no Discovery server */ }
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("unexpected success against unreachable Discovery"),
        }
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test -p auki-domain-swift bootstrap_swift_swarm_construction_succeeds
```

Expected: FAIL — `cannot find function 'bootstrap_swift'`.

- [ ] **Step 3: Add the bootstrap orchestrator + error wrapper**

In `bindings/swift/auki-domain-swift/src/lib.rs`, after the re-exports:

```rust
// ─── Bootstrap orchestrators ────────────────────────────────────────
//
// Swift consumers don't construct a `Swarm<Behaviour>` directly; these
// three orchestrators take a wallet seed + listen multiaddrs + agent
// version + DaemonInfo + optional SwiftStreamProvider, build the swarm
// internally, and delegate to the upstream ClusterManager constructors.
// Pattern matches auki-domain-py's bootstrap / create_cluster /
// join_cluster shims.

/// Errors from [`bootstrap_swift`], [`create_cluster_swift`], and
/// [`join_cluster_swift`]. Flattens swarm-build failures and 32-byte
/// seed length failures to a `message: String`; nested BootstrapError /
/// CreateClusterError / JoinClusterError variants are surfaced via the
/// respective return-type errors.
#[derive(uniffi::Error, Debug, thiserror::Error)]
pub enum BootstrapSwiftError {
    #[error("invalid wallet seed: expected 32 bytes, got {actual}")]
    InvalidSeed { actual: u32 },
    #[error("swarm build: {message}")]
    SwarmBuild { message: String },
    #[error("identity derivation: {message}")]
    IdentityDerivation { message: String },
}

/// Build the libp2p swarm + PeerIdentity from Swift inputs. Used by
/// the three orchestrators below.
async fn build_swarm_and_identity(
    wallet_seed: Vec<u8>,
    listen_addresses: Vec<String>,
    external_addresses: Option<Vec<String>>,
    agent_version: String,
) -> Result<
    (
        auki_network::PeerIdentity,
        Vec<Multiaddr>,
        libp2p::Swarm<auki_network::swarm::Behaviour>,
    ),
    BootstrapSwiftError,
> {
    if wallet_seed.len() != 32 {
        return Err(BootstrapSwiftError::InvalidSeed {
            actual: wallet_seed.len() as u32,
        });
    }
    let wallet = auki_identity::Wallet::from_seed(wallet_seed)
        .map_err(|e| BootstrapSwiftError::IdentityDerivation {
            message: e.to_string(),
        })?;
    let identity = auki_network::PeerIdentity::from_wallet(wallet);

    let listen_multiaddrs: Vec<Multiaddr> = listen_addresses
        .iter()
        .map(|s| s.parse::<Multiaddr>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| BootstrapSwiftError::SwarmBuild {
            message: format!("invalid listen multiaddr: {e}"),
        })?;

    let external_multiaddrs: Vec<Multiaddr> = match external_addresses {
        Some(addrs) => addrs
            .iter()
            .map(|s| s.parse::<Multiaddr>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| BootstrapSwiftError::SwarmBuild {
                message: format!("invalid external multiaddr: {e}"),
            })?,
        None => vec![],
    };

    let swarm = auki_network::swarm::build_swarm(
        &identity,
        auki_network::swarm::SwarmConfig {
            listen_addresses: listen_multiaddrs.clone(),
            agent_version,
            enable_relay_server: false,
        },
    )
    .map_err(|e| BootstrapSwiftError::SwarmBuild {
        message: e.to_string(),
    })?;

    // External addresses (e.g. router-mapped public IP:port) are added
    // post-build via `swarm.add_external_address(...)`. The Python binding
    // does the same.
    let mut swarm = swarm;
    for addr in external_multiaddrs {
        swarm.add_external_address(addr);
    }

    Ok((identity, listen_multiaddrs, swarm))
}

/// Swift entry point for joining-or-creating a cluster. Mirrors
/// `auki-domain-py`'s `ClusterManager.bootstrap` static method.
#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap_swift(
    target: ClusterTarget,
    wallet_seed: Vec<u8>,
    discovery_url: String,
    listen_addresses: Vec<String>,
    agent_version: String,
    daemon_info: DaemonInfo,
    stream_provider: Option<Box<dyn SwiftStreamProvider>>,
    external_addresses: Option<Vec<String>>,
) -> Result<Arc<ClusterManager>, BootstrapError> {
    let (identity, listen_multiaddrs, swarm) = build_swarm_and_identity(
        wallet_seed,
        listen_addresses,
        external_addresses,
        agent_version,
    )
    .await
    .map_err(|e| BootstrapError::Rejected(e.to_string()))?;

    // Convert SwiftStreamProvider (Box<dyn>) → upstream StreamProvider
    // closure. If None, install decline_all_streams.
    let stream_provider_closure = match stream_provider {
        Some(p) => {
            let p: Arc<dyn SwiftStreamProvider> = Arc::from(p);
            auki_network_swift::swift_provider_to_upstream(p)
        }
        None => auki_network::stream_runtime::decline_all_streams(),
    };

    let manager = ClusterManager::bootstrap(
        target,
        identity,
        listen_multiaddrs,
        discovery_url,
        swarm,
        stream_provider_closure,
        daemon_info,
    )
    .await?;
    Ok(Arc::new(manager))
}
```

Note: `auki_network_swift::swift_provider_to_upstream` is the adapter function PR B added (Task 13). It's currently `pub(crate)` — needs to be `pub` for this Plan C re-use. The implementer changes the visibility in `auki-network-swift/src/lib.rs` as part of this task and re-runs PR B's tests to ensure nothing broke.

- [ ] **Step 4: Verify test passes**

```bash
cargo test -p auki-domain-swift bootstrap_swift_swarm_construction_succeeds
```

Expected: PASS. The test runs against an unreachable Discovery URL and expects `Err(BootstrapError::Discovery(...))`.

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-domain-swift/src/lib.rs bindings/swift/auki-network-swift/src/lib.rs
git commit -m "feat(auki-domain-swift): add bootstrap_swift orchestrator + shared swarm builder"
```

---

### Task 22: Add `create_cluster_swift` + `join_cluster_swift` orchestrators

**Files:**
- Modify: `bindings/swift/auki-domain-swift/src/lib.rs`

Two more orchestrators following the same pattern as `bootstrap_swift`. Each takes the same inputs minus the `target` enum (replaced by an explicit `cluster_name` String). Delegates to `ClusterManager::create_cluster` and `ClusterManager::join_cluster` respectively.

- [ ] **Step 1: Add the two orchestrators**

After `bootstrap_swift`:

```rust
/// Swift entry point for creating a new cluster. Mirrors
/// `auki-domain-py`'s `ClusterManager.create_cluster` static method.
#[uniffi::export(async_runtime = "tokio")]
pub async fn create_cluster_swift(
    cluster_name: String,
    wallet_seed: Vec<u8>,
    discovery_url: String,
    listen_addresses: Vec<String>,
    agent_version: String,
    daemon_info: DaemonInfo,
    stream_provider: Option<Box<dyn SwiftStreamProvider>>,
    external_addresses: Option<Vec<String>>,
) -> Result<Arc<ClusterManager>, CreateClusterError> {
    let (identity, listen_multiaddrs, swarm) = build_swarm_and_identity(
        wallet_seed,
        listen_addresses,
        external_addresses,
        agent_version,
    )
    .await
    .map_err(|e| CreateClusterError::Discovery(
        auki_network::discovery_client::DiscoveryError::InvalidPeerId(e.to_string()),
    ))?;

    let stream_provider_closure = match stream_provider {
        Some(p) => {
            let p: Arc<dyn SwiftStreamProvider> = Arc::from(p);
            auki_network_swift::swift_provider_to_upstream(p)
        }
        None => auki_network::stream_runtime::decline_all_streams(),
    };

    let manager = ClusterManager::create_cluster(
        cluster_name,
        identity,
        listen_multiaddrs,
        discovery_url,
        swarm,
        stream_provider_closure,
        daemon_info,
    )
    .await?;
    Ok(Arc::new(manager))
}

/// Swift entry point for joining an existing cluster. Mirrors
/// `auki-domain-py`'s `ClusterManager.join_cluster` static method.
#[uniffi::export(async_runtime = "tokio")]
pub async fn join_cluster_swift(
    cluster_name: String,
    wallet_seed: Vec<u8>,
    discovery_url: String,
    listen_addresses: Vec<String>,
    agent_version: String,
    daemon_info: DaemonInfo,
    stream_provider: Option<Box<dyn SwiftStreamProvider>>,
    external_addresses: Option<Vec<String>>,
) -> Result<Arc<ClusterManager>, JoinClusterError> {
    let (identity, listen_multiaddrs, swarm) = build_swarm_and_identity(
        wallet_seed,
        listen_addresses,
        external_addresses,
        agent_version,
    )
    .await
    .map_err(|e| JoinClusterError::NotFound(e.to_string()))?;

    let stream_provider_closure = match stream_provider {
        Some(p) => {
            let p: Arc<dyn SwiftStreamProvider> = Arc::from(p);
            auki_network_swift::swift_provider_to_upstream(p)
        }
        None => auki_network::stream_runtime::decline_all_streams(),
    };

    let manager = ClusterManager::join_cluster(
        cluster_name,
        identity,
        listen_multiaddrs,
        discovery_url,
        swarm,
        stream_provider_closure,
        daemon_info,
    )
    .await?;
    Ok(Arc::new(manager))
}
```

The `BootstrapSwiftError` mapping into the various `*Error` types in the failure paths is best-effort — there's no clean match in `CreateClusterError` for a "swarm build failed" variant. The error gets folded into whichever variant is least lossy (e.g., `Discovery::InvalidPeerId` for `CreateClusterError`). If this gets noisy, add a new variant to each error upstream (`*Error::SwarmBuild { message: String }`). For v0, the current mapping is acceptable; document in parking_lot.

- [ ] **Step 2: Add a smoke test for `create_cluster_swift`**

```rust
    #[tokio::test]
    async fn create_cluster_swift_swarm_construction_succeeds() {
        let result = create_cluster_swift(
            "test-cluster".to_string(),
            vec![2u8; 32],
            "http://127.0.0.1:9".to_string(),
            vec!["/ip4/127.0.0.1/tcp/0".to_string()],
            "test-agent/0.0".to_string(),
            DaemonInfo {
                app: "test-app".to_string(),
                name: "test-instance".to_string(),
                session_id: "session-2".to_string(),
                session_clock_id: "clock-2".to_string(),
                session_clock_hash: "hash-2".to_string(),
                app_instance: "instance-2".to_string(),
            },
            None,
            None,
        ).await;

        // Expect Discovery failure (no server) — same as bootstrap_swift test.
        assert!(matches!(result, Err(CreateClusterError::Discovery(_))));
    }
```

- [ ] **Step 3: Verify + commit**

```bash
cargo test -p auki-domain-swift create_cluster_swift_swarm_construction_succeeds
git add bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain-swift): add create_cluster_swift + join_cluster_swift orchestrators"
```

---

### Task 23: Add `list_clusters` standalone helper

**Files:**
- Modify: `bindings/swift/auki-domain-swift/src/lib.rs`

`ClusterManager::list_clusters` is a static method. UniFFI 0.31 supports static methods on Objects via `#[uniffi::constructor]` (returns Arc<Self>) — but `list_clusters` returns `Vec<ClusterEntry>`, not Self. So expose as a free function instead.

- [ ] **Step 1: Add the free function**

```rust
/// List clusters from Discovery, sorted by created_ns desc. Mirrors
/// `auki-domain-py`'s static `ClusterManager.list_clusters`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn list_clusters(
    discovery_url: String,
) -> Result<Vec<ClusterEntry>, DiscoveryError> {
    ClusterManager::list_clusters(discovery_url).await
}
```

- [ ] **Step 2: Verify + test + commit**

```bash
cargo build -p auki-domain-swift
# No new test — function delegates entirely; tested indirectly by upstream + smoke.
git add bindings/swift/auki-domain-swift/src/lib.rs
git commit -m "feat(auki-domain-swift): add list_clusters free function"
```

---

### Task 24: Verify workspace-wide builds

**Files:** None — verification gate.

- [ ] **Step 1: Default `auki-domain` build**

```bash
cargo build -p auki-domain
cargo test -p auki-domain --lib
```

Expected: PASS (no swift-bindings).

- [ ] **Step 2: swift-bindings build + tests**

```bash
cargo build --features swift-bindings -p auki-domain
cargo test --features swift-bindings -p auki-domain --lib
```

Expected: PASS for both — all upstream tests should still pass; the annotations don't change behavior.

- [ ] **Step 3: `auki-domain-swift` build + tests**

```bash
cargo build -p auki-domain-swift
cargo test -p auki-domain-swift --lib
```

Expected: PASS.

- [ ] **Step 4: Full workspace**

```bash
cargo build --workspace --exclude browser_probe_listener
```

Expected: PASS. Pre-existing `browser_probe_listener` example bug stays excluded.

If `auki-network-py` or any other downstream binding fails because of an upstream signature change (e.g. the `impl Into<String>` → `String` changes in Task 18, or the `TransformEdgeResource::source` → `Option<String>` change in Task 6), adapt the failing call sites in place — same pattern as PR B's cascading fix for `auki-network-py`.

- [ ] **Step 5: No commit needed**

If any step fails, return to the offending task and fix in place before proceeding to Task 25.

---

### Task 25: Validate the iOS XCFramework build end-to-end

**Files:** None — runs the build script.

- [ ] **Step 1: Run the build**

```bash
bash bindings/swift/auki-domain-swift/build-xcframework.sh
```

Expected: success — produces an XCFramework at `bindings/swift/auki-domain-swift/target-xcframework/AukiDomain.xcframework/` containing:
- `ios-arm64/` (device slice)
- `ios-arm64_x86_64-simulator/` (fat simulator slice)
- `module.modulemap` aggregating four upstream namespaces (auki_identity, auki_network, auki_domain, auki_domain_swift)
- One `*.swift` file per UniFFI namespace

- [ ] **Step 2: Inspect the generated Swift surface**

```bash
ls bindings/swift/auki-domain-swift/target-xcframework/swift/
```

Verify the file list. Spot-check for:
- `class ClusterManager` (upstream Object)
- `class StreamSubscriptionAudio` (via auki-network)
- `protocol SensorCatalogProvider`, `protocol ResourceCatalogProvider` (callback interfaces)
- `enum ClusterTarget`
- `func bootstrapSwift(...)`, `func createClusterSwift(...)`, `func joinClusterSwift(...)`, `func listClusters(...)` (binding-crate free functions)
- `struct DaemonInfo`, `struct ClusterMember`, `struct ClusterMembership`
- `struct SensorRegistryEntry`, `enum SensorBody`, `enum ClockBody`, etc.
- `enum BootstrapError`, `enum CreateClusterError`, `enum JoinClusterError`

- [ ] **Step 3: No commit needed — integration validation**

If any expected Swift surface is missing, return to the relevant task to fix.

---

### Task 26: Write `auki-domain-swift` per-component docs

**Files:**
- Create: `bindings/swift/auki-domain-swift/README.md`
- Create: `bindings/swift/auki-domain-swift/parking_lot.md`
- Create: `bindings/swift/auki-domain-swift/changelog.md`
- Create: `bindings/swift/auki-domain-swift/src/readme.md`
- Create: `bindings/swift/auki-domain-swift/src/sprint.md`

- [ ] **Step 1: Create `README.md`**

Write to `bindings/swift/auki-domain-swift/README.md`:

```markdown
# auki-domain-swift

UniFFI Swift bindings for `auki-domain` — exposes the cluster Manager surface (bootstrap / membership / catalogs / streams / clock sync / diagnostics) to native iOS (Swift) peers. Aggregates the upstream-annotated types from `auki-identity` (PR A), `auki-network` (PR B), `auki-domain` (this PR), and `auki-time` under a single `AukiDomain.xcframework` umbrella.

## Scope (v0 — PR C)

- **Bootstrap orchestrators**: `bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift`, plus the static `list_clusters` helper. Each takes a 32-byte wallet seed + listen multiaddrs + agent version + DaemonInfo + optional SwiftStreamProvider. Mirrors `auki-domain-py`'s `ClusterManager.bootstrap` / `.create_cluster` / `.join_cluster`.
- **ClusterManager surface**: full method set — `cluster_name`, `local_peer_id_string`, `local_multiaddr_strings`, `manager_peer_id_string`, `is_manager`, `peer_count_u32`, `membership`, `participant_info`, `fetch_participant_info`, `admit_peer`, `fetch_*_catalog` (sensors + resources), `fetch_*_entry` (sensor + clock + frame + detector), the 5 typed `open_*_stream` methods, `set_sensor_catalog_provider`, `set_resource_catalog_provider`, `set_registry_app_root`, `clock_sync_estimate` / `clock_sync_estimates` / `domain_clock_estimate` / `domain_time_now`, `broadcast_diagnostic_message` / `drain_diagnostic_messages`, `shutdown`.
- **Callback interfaces**: `SensorCatalogProvider`, `ResourceCatalogProvider` (upstream traits, exported as Swift protocols).
- **Stream surface**: 5 typed `StreamSubscription*` Objects + the typed `open_*_stream` methods, re-exported from PR B's `auki-network-swift`. Producer side via PR B's `SwiftStreamProvider` two-call protocol.
- **Registry typed records**: full `SensorBody` / `ClockBody` / `FrameRegistryEntry` / `DetectorRegistryEntry` trees annotated as UniFFI Records / Enums — Swift consumers get typed structs, not opaque JSON strings.

## Out of scope

- iosapp integration code — that's Spec 2.
- A published SwiftPM package — distribution stays build-from-source via the iosapp sync script.
- `browser_session.rs` — browser-runtime feature; not native iOS.
- `elect_successor` — internal upstream utility.

## Build

XCFramework via `./build-xcframework.sh`. Validated against Xcode 26.3 + rustc 1.94. Produces a two-slice framework: `ios-arm64` (device) + `ios-arm64_x86_64-simulator` (fat sim).

iOS targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`.

## Consuming from Swift (sketch)

```swift
import AukiDomain  // hypothetical umbrella module

let manager = try await bootstrapSwift(
    target: .create(name: "demo-cluster"),
    walletSeed: Data(/* 32 bytes from Keychain */),
    discoveryUrl: "http://192.168.9.130:8080",
    listenAddresses: ["/ip4/0.0.0.0/tcp/0"],
    agentVersion: "iosapp/0.0",
    daemonInfo: DaemonInfo(...),
    streamProvider: nil,
    externalAddresses: nil
)

print("Local peer-id: \(manager.localPeerIdString())")
let membership = manager.membership()
print("Members: \(membership.peers.count)")

let sensors = try await manager.fetchSensorsCatalog(peerId: peerIdString)
```
```

- [ ] **Step 2: Create `parking_lot.md`**

```markdown
# auki-domain-swift parking lot

Open questions for this crate. Resolved items are deleted (per auki-sdk convention) and recorded in `changelog.md`.

## Open

- **`BootstrapSwiftError` vs. typed `*Error` mapping.** Swarm-build and 32-byte seed length failures inside the bootstrap orchestrators are currently folded into existing variants of `BootstrapError` / `CreateClusterError` / `JoinClusterError` (e.g. `Rejected(message)` or `Discovery(InvalidPeerId)`). Cleaner would be adding new variants upstream: `*Error::SwarmBuild { message: String }` and `*Error::InvalidSeed { actual: u32 }`. v0 ships with the fold mapping; future expansion adds the upstream variants.
- **`StreamSubscription*` cross-crate UniFFI propagation.** PR B's auki-network-swift defines the 5 subscription Objects. Plan C re-exports them via `pub use`; verify the Swift surface includes them under the `AukiDomain` xcframework's `auki_network.swift` namespace file.
- **`open_stream` generic resolver.** Python has a single `open_stream(peer_id, sensor_id)` that fetches the resource catalog, finds the payload kind, and dispatches to the typed opener. Skipped in Plan C — Swift consumers do this dispatch in Swift code by calling `fetch_resources_catalog` then the matching typed opener.
- **Heartbeat-detail variants of `SwiftPeerLivenessEvent`.** Inherited from PR B's parking lot — `HeartbeatReceived` / `HeartbeatNtpSampleObserved` upstream variants are dropped at v0.
- **`TransformEdgeResource::source` type change.** If Task 6 chose path (a) — change upstream from `Option<serde_json::Value>` to `Option<String>` — verify nothing in `auki-domain` or downstream consumers relied on programmatic JSON-value manipulation of this field. Audit periodically.
- **Single shared tokio runtime.** Three binding crates each drive their own. Consolidate if profiling shows pain.
```

- [ ] **Step 3: Create `changelog.md`**

```markdown
# Changelog — auki-domain-swift

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 22, HKT, 2026

Initial release. PR C of the Spec 1 SDK Swift binding expansion. Provides full Swift access to `auki-domain::ClusterManager` with parity to `auki-domain-py` plus the upstream-only methods explicitly included by user choice (clock sync, diagnostics). Architecture: upstream UniFFI annotations behind a new `swift-bindings` cargo feature on `auki-domain`; this binding crate is a thin scaffolding host with `PeerId` / `Multiaddr` custom-type registrations, `pub use` re-exports of upstream-annotated types, and three orchestrator functions (`bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift`) that hide libp2p swarm construction from Swift callers. Inherits the 5-payload stream surface + Swift callback-interface traits from `auki-network-swift` (PR B) via cross-crate dep + `pub use`. Registry-entry trees (`SensorBody`, `ClockBody`, `FrameRegistryEntry`, `DetectorRegistryEntry` + nested types) annotated as typed UniFFI records/enums per user choice.
```

- [ ] **Step 4: Create `src/readme.md`**

```markdown
# auki-domain-swift implementation

`src/lib.rs` hosts:

- `uniffi::setup_scaffolding!()` — the per-crate UniFFI metadata anchor.
- `uniffi::custom_type!(PeerId, String, { remote, ... })` and same for `Multiaddr` — the canonical string FFI seam for libp2p types.
- `pub use` re-exports of all upstream-annotated types from `auki-domain`, `auki-network`, `auki-time`, and `auki-network-swift` (for the binding-crate-side callback-interface traits + StreamSubscription* Swift glue).
- `BootstrapSwiftError` — typed error for the orchestrator pre-flight failures (seed length, swarm build, identity derivation).
- `build_swarm_and_identity` — internal helper that takes Swift-shaped inputs and returns `(PeerIdentity, Vec<Multiaddr>, Swarm<Behaviour>)`.
- `bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift` — three async orchestrators exposed via `#[uniffi::export(async_runtime = "tokio")]`. Each builds the swarm internally, optionally wraps a SwiftStreamProvider, and delegates to the upstream constructor.
- `list_clusters` — static-equivalent free function returning `Vec<ClusterEntry>`.

Upstream-side additions (in `crates/auki-domain` + `crates/auki-network` + `crates/auki-time`):

- `ClusterManager` (Object) + `ClusterTarget` (Enum) + provider traits as callback interfaces.
- ~30 annotated methods on `ClusterManager` — identity, membership, catalogs, registries, streams, clock sync, diagnostics.
- ~20 annotated value records (DaemonInfo, ClusterMember, ClusterMembership, ParticipantInfo, SensorEntry, registry entries + nested types, resource records + geometry, clock sync types, diagnostic types).
- 10+ annotated error enums (flat_error pattern for variants wrapping non-FFI inner errors).
```

- [ ] **Step 5: Create `src/sprint.md`**

```markdown
# auki-domain-swift sprint

## Current

PR C landed. The crate provides the full v0 cluster-orchestration surface iosapp's Spec 2 needs.

## Next

Spec 2 — iosapp consumer wiring + proof-of-load UI. Driven by `aukilabs/iosapp`'s own spec/plan cycle; this crate's API is stable for that work.
```

- [ ] **Step 6: Commit**

```bash
git add bindings/swift/auki-domain-swift/README.md bindings/swift/auki-domain-swift/parking_lot.md bindings/swift/auki-domain-swift/changelog.md bindings/swift/auki-domain-swift/src/readme.md bindings/swift/auki-domain-swift/src/sprint.md
git commit -m "docs(auki-domain-swift): per-component docs for PR C"
```

---

### Task 27: Update `bindings/swift/` indices

**Files:**
- Modify: `bindings/swift/README.md`
- Modify: `bindings/swift/parking_lot.md`
- Modify: `bindings/swift/changelog.md`

- [ ] **Step 1: Update `bindings/swift/README.md`**

Add an `auki-domain-swift` row to the per-crate table:

```
| `auki-domain-swift` | ClusterManager (bootstrap / membership / catalogs / streams / clock sync / diagnostics) — full `auki-domain-py` parity + extras. PR C. |
```

- [ ] **Step 2: Update `bindings/swift/parking_lot.md`**

Add a structured per-package summary:

```
- [`auki-domain-swift/parking_lot.md`](auki-domain-swift/parking_lot.md): 6 open items (BootstrapSwiftError variant mapping, cross-crate stream subscription propagation verification, deferred generic open_stream resolver, heartbeat-detail variant inheritance, TransformEdgeResource::source change audit, shared tokio runtime).
```

- [ ] **Step 3: Update `bindings/swift/changelog.md`**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

`auki-domain-swift` PR C landed: the third and final Swift binding crate of Spec 1's SDK expansion. Full ClusterManager surface with `auki-domain-py` parity + upstream-only methods (clock sync, diagnostics) per user scope choice. Aggregates PR A (auki-identity-swift) + PR B (auki-network-swift) + this crate's additions into a single `AukiDomain.xcframework` umbrella. Registry entry trees annotated as typed records (not canonical JSON strings). See [`auki-domain-swift/changelog.md`](auki-domain-swift/changelog.md) for the crate-level entry.
```

- [ ] **Step 4: Commit**

```bash
git add bindings/swift/README.md bindings/swift/parking_lot.md bindings/swift/changelog.md
git commit -m "docs(bindings/swift): propagate PR C summary to indices"
```

---

### Task 28: Propagate to upstream + bindings + root + docs changelogs

**Files:**
- Modify: `crates/auki-domain/parking_lot.md`
- Modify: `crates/auki-domain/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `bindings/changelog.md`
- Modify: `changelog.md` (root)
- Modify: `docs/changelog.md`

- [ ] **Step 1: Update `crates/auki-domain/changelog.md`**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

PR C — added new `swift-bindings` cargo feature gating UniFFI proc-macros across the public surface: `ClusterManager` (Object), `ClusterTarget` (Enum), `DaemonInfo` + cluster_membership + stream_manifest records, all 10+ error enums (flat_error pattern), and the two catalog-provider traits as UniFFI callback interfaces. ~30 methods exposed on ClusterManager (identity, membership, catalogs, registries, streams, clock sync, diagnostics, shutdown). The 5 per-payload `open_*_stream` methods delegate to `auki-network`'s annotated NetworkRuntime methods (PR B). `impl Into<String>` parameters changed to `String` where the methods are FFI-exposed; this is additive for non-FFI callers but worth noting in case of compilation cascades. Bootstrap constructors stay un-annotated upstream; Swift consumers go through `bindings/swift/auki-domain-swift::bootstrap_swift` / `create_cluster_swift` / `join_cluster_swift` orchestrators.
```

- [ ] **Step 2: Update `crates/auki-domain/parking_lot.md`**

Prepend a swift-bindings entry:

```
- **`swift-bindings` feature**: gates UniFFI proc-macros on the full public ClusterManager surface + provider trait callback interfaces. Propagates to `auki-identity/swift-bindings`, `auki-network/swift-bindings`, and `auki-time/swift-bindings`. No behavior change with the feature off (default).
```

- [ ] **Step 3: Update `crates/changelog.md`**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

`auki-domain` — UniFFI-annotated the full PR C surface (ClusterManager + ClusterTarget + provider traits + errors + records). See [`auki-domain/changelog.md`](auki-domain/changelog.md). Also annotates registry/resource/sensors/diagnostic types in `auki-network` and clock-sync types in `auki-time` to support the full ClusterManager method exposure.
```

- [ ] **Step 4: Update `bindings/changelog.md`**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

`bindings/swift/auki-domain-swift` PR C landed: the third and final binding crate of Spec 1's SDK Swift expansion. Full ClusterManager surface (bootstrap / membership / catalogs / streams / clock sync / diagnostics) plus 3 orchestrator functions that hide libp2p swarm construction. Aggregates PR A + PR B into one `AukiDomain.xcframework` umbrella. See [`swift/auki-domain-swift/changelog.md`](swift/auki-domain-swift/changelog.md).
```

- [ ] **Step 5: Update `changelog.md` (root)**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

Spec 1 PR C landed: `bindings/swift/auki-domain-swift`, the final Swift binding crate of the SDK expansion. Provides full `ClusterManager` access to native iOS — bootstrap / create / join orchestrators, membership snapshots, sensor + resource + registry catalog fetches, 5 typed payload stream openers, clock sync, diagnostics. Aggregates the auki-identity (PR A), auki-network (PR B), and auki-domain (this PR) UniFFI surfaces into one `AukiDomain.xcframework`. Unblocks Spec 2 (iosapp wiring + proof-of-load UI).
```

- [ ] **Step 6: Update `docs/changelog.md`**

Prepend:

```
### Nils's claude · May 22, HKT, 2026

Added the [Spec 1 PR C implementation plan](superpowers/plans/2026-05-22-spec1-prc-auki-domain-swift.md) for `auki-domain-swift`. 28 tasks covering: new `swift-bindings` feature on auki-domain (propagated to auki-time + extending auki-network + auki-identity), annotation of ~50 records/enums/errors across auki-domain + auki-network + auki-time, ClusterManager Object + ~30 methods, 3 bootstrap orchestrators in the binding crate, registry-tree typed records (per user choice over canonical JSON shortcut), full XCFramework validation. With PR C merged, Spec 1's three-PR arc is complete; iosapp Spec 2 work can begin.
```

- [ ] **Step 7: Commit**

```bash
git add crates/auki-domain/parking_lot.md crates/auki-domain/changelog.md crates/changelog.md bindings/changelog.md changelog.md docs/changelog.md
git commit -m "docs: propagate PR C summary across hierarchy"
```

---

## Implementation order

Tasks are listed in dependency order. Critical sub-orderings within each phase:

1. **Setup (Tasks 1-4)** — feature + scaffolding + new crate skeleton. Required before any annotation work.
2. **Annotations (Tasks 5-15)** — records and enums first; errors next; callback-interface traits; finally `ClusterTarget`. Bottom-up: leaf records before composite enums.
3. **ClusterManager (Tasks 17-19)** — Object first, then methods grouped by sync-ness and concern.
4. **Re-exports + orchestrators (Tasks 20-23)** — only possible after upstream annotations exist.
5. **Validation (Tasks 24-25)** — workspace build then xcframework.
6. **Docs (Tasks 26-28)** — propagation.

## Stop gates

- After Task 19: ClusterManager fully annotated upstream. If the orchestrator design (Tasks 21-22) needs to be deferred, downstream consumers can still construct the manager via the raw `ClusterManager::bootstrap` call (un-FFI). Spec 2 work can begin probing the surface.
- After Task 23: All FFI-exposed functions exist. XCFramework validation in Task 25 confirms the full surface lands.

## Open design items addressed during execution

1. **`Self::method(self, ...)` indirection vs. method rename** — Task 17/18 has implementer choose during execution.
2. **`TransformEdgeResource::source` field shape** — Task 6 picks between upstream change (preferred) and binding-crate shim.
3. **`auki_network_swift::swift_provider_to_upstream` visibility** — Task 21 changes from `pub(crate)` to `pub` for cross-crate reuse.
4. **`use_remote_type!` vs `pub use`** — Task 20's verification step. Most types should work via `pub use`; some may need explicit `use_remote_type!`.
