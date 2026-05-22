# Changelog — docs

Append-only timeline of documentation changes under `docs/`. Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

Updated the Superpowers Auki proto generation docs so only the generated Rust `auki-proto` crate is committed; JavaScript/TypeScript, Swift, and Python protobuf outputs live as ignored generated artifacts under `bindings/`.

### Nils's codex · May 22, HKT, 2026

Updated the Superpowers Auki proto generation plan to skip committed Python protobuf output for now; Python generation remains an on-demand artifact outside the initial `auki-proto` migration.

### Nils's codex · May 22, HKT, 2026

Added the Superpowers design and implementation plan for replacing `auki-datatypes` with generated per-platform `auki-proto` packages sourced from root `proto/auki` schemas.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers implementation plan for turning `auki-uniffi-test` into a shared-core multiplatform binding proving crate.

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
