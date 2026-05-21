# Changelog — docs

Append-only timeline of documentation changes under `docs/`. Latest entry on top.

---

### Nils's claude · May 21, 14:10 HKT, 2026

Added the [Spec 1 PR A implementation plan](superpowers/plans/2026-05-20-spec1-pra-auki-identity-swift.md) for `auki-identity-swift`. Eleven tasks covering: optional `swift-bindings` cargo feature on `crates/auki-identity` + `crates/auki-network`, UniFFI proc-macros on `Wallet` and `PeerIdentity` (with small `wallet_id_str` / `peer_id_string` helpers), new `bindings/swift/auki-identity-swift/` scaffolding host, per-component doc files, indices + changelog propagation, end-to-end iOS XCFramework validation. Plans for PR B (network expansion) and PR C (auki-domain-swift) follow once PR A lands.

### Nils's claude · May 21, 13:38 HKT, 2026

Rewrote the [SDK Swift binding expansion design](superpowers/specs/2026-05-20-sdk-swift-binding-expansion-design.md) (revision 2) — pivoted to upstream UniFFI proc-macros under a new `swift-bindings` cargo feature on each of `crates/auki-{identity,network,domain}`. Binding crates under `bindings/swift/` become thin scaffolding hosts; UniFFI introspects upstream types directly, hand-wrapping near-eliminated. See [`superpowers/specs/changelog.md`](superpowers/specs/changelog.md).

### Nils's claude · May 21, 13:24 HKT, 2026

Added the [SDK Swift binding expansion design](superpowers/specs/2026-05-20-sdk-swift-binding-expansion-design.md): three new/expanded binding crates under `bindings/swift/` (`auki-identity-swift`, `auki-network-swift` expansion, `auki-domain-swift`) covering the SDK surface that `aukilabs/iosapp`'s proof-of-load demo needs. Blocks Spec 2 (iosapp wiring). See [`superpowers/specs/changelog.md`](superpowers/specs/changelog.md) for the spec-level entry.

### Nils's codex · May 21, HKT, 2026

Refined the Superpowers stream naming cleanup docs after implementation so they describe the final vocabulary directly.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers implementation plan for the SDK-wide stream naming cleanup.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers stream naming cleanup design spec for the SDK-wide full rename of camera, detection, and camera registry vocabulary.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter planning docs so Park and the browser SDK package use the same current SDK vocabulary: `audio`, `camera`, `point_cloud`, `joint_encoders`, `detection`, plus UI stream states `declined` and `error`.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to use `DomainClockSource.backing_peer_id` for domain-clock source provenance instead of naming the field after the Manager role.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to use `DomainClockSource.cluster_name` instead of introducing a separate domain id concept.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so `DomainClockSource` carries an explicit `domain_id` alongside the derived domain clock id.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so `NetworkRuntime::spawn(...)` requires SDK-owned heartbeat timestamps directly as part of the default runtime contract.

### Nils's codex · May 20, HKT, 2026

Walked back the heartbeat time-sync planning docs so heartbeat frames stay sender-clock-only; domain-clock identity remains in `DomainClockSource` and future TimeTransform manifests.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to keep NTP estimation in `auki-time` and limit `ClusterManager` to domain-clock source authority.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so future heartbeat timestamp wiring consumes `SessionClock` as the single source of clock identity and readings.

### Nils's codex · May 20, HKT, 2026

Updated Superpowers timekeeping docs to target the renamed `auki-time` crate instead of `auki-time-transforms`.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain plan to prevent browser/native SDK drift: browser crates are now bindings/facades over shared Rust `auki-network` and `auki-domain` logic, with runtime-specific code limited to concrete wasm/browser constraints.

### Nils's codex · May 20, HKT, 2026

Rewrote the browser Domain WebRTC plan around true peer symmetry: browser peers can be Managers, Discovery records PeerIds rather than platform classes, and reachability is an SDK transport concern.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers SDK timekeeping foundation plan and marked heartbeat time sync as dependent on a reusable `SessionClock` primitive.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers implementation plan for syncing cluster peers to a Manager-authored domain clock over heartbeat-carried TimeTransform samples.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers native pointcloud design spec and propagated the docs-level changelog chain for the SDK pointcloud refactor planning artifact.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain WebRTC join implementation plan after the browser probe smoke passed, covering production Manager WebRTC advertisement, browser wasm raw SDK substreams, and `auki-domain-browser` join wiring.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser WebRTC probe stream implementation plan and propagated the docs-level changelog chain for the native listener plus browser wasm dial proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport compile-probe implementation plan and propagated the docs-level changelog chain for the first SDK browser networking spike slice.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport spike spec and propagated the docs-level changelog chain for the SDK-owned browser networking proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter implementation plan and propagated the docs-level changelog chain for the first SDK package tranche.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter design spec and propagated the docs-level changelog chain for the SDK package Park needs to load real browser peers.
