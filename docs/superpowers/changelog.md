# Changelog — docs/superpowers

Append-only timeline of Superpowers design artifacts. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Refined the stream naming cleanup Superpowers artifacts so active plans/specs describe the final `CameraFrame`, `DetectionFrame`, and `Camera` vocabulary without mechanical self-renames.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup implementation plan under [`plans/`](plans/2026-05-21-stream-naming-cleanup.md), sequencing the full no-compatibility rename across datatypes, registry, network, bindings, domain, ROS adapter, docs, and Park.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup design under [`specs/`](specs/2026-05-21-stream-naming-cleanup-design.md), specifying the no-compatibility rename to `CameraFrame`, `DetectionFrame`, and `SensorBody::Camera`.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter plan/spec under [`plans/`](plans/changelog.md) and [`specs/`](specs/changelog.md) so the browser contract cannot drift from the current SDK sensor-kind and stream-state vocabulary.

### Nils's codex · May 20, HKT, 2026

Renamed the heartbeat time-sync plan's `DomainClockSource.manager_peer_id` field to `backing_peer_id`, making the source record describe clock provenance instead of Manager role.

### Nils's codex · May 20, HKT, 2026

Replaced the proposed `DomainClockSource.domain_id` field with the existing `cluster_name` concept in the heartbeat time-sync plan; the domain clock id derives from `<cluster-name>/domain-clock`.

### Nils's codex · May 20, HKT, 2026

Made `DomainClockSource.domain_id` explicit in the heartbeat time-sync plan; the domain clock id now derives from `<domain-id>/domain-clock` instead of requiring consumers to parse `clock_id`.

### Nils's codex · May 20, HKT, 2026

Corrected the heartbeat time-sync plan so `NetworkRuntime::spawn(...)` requires `HeartbeatTimestampSource` directly as the default heartbeat timestamp path.

### Nils's codex · May 20, HKT, 2026

Walked back the heartbeat time-sync plan so `/auki/heartbeat/0.0.1` carries only sender timestamp clock identity plus NTP echo fields; the domain-clock id/hash now live in `DomainClockSource` and future TimeTransform manifests. Also refreshed Python binding paths to `bindings/python/auki-domain-py`.

### Nils's codex · May 20, HKT, 2026

Revised the heartbeat time-sync plan so NTP math, filtering, and local TimeTransform production live in `auki-time`; `ClusterManager` remains only the domain-clock source authority.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync plan to depend on the implemented `SessionClock` foundation and to describe heartbeat timestamps as sourced from `SessionClock`, not `DaemonInfo` clock fields.

### Nils's codex · May 20, HKT, 2026

Renamed the SDK timekeeping foundation plan's target crate from `auki-time-transforms` to `auki-time`, matching the crate rename and the broader timekeeping responsibility.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain peer symmetry plan under [`plans/`](plans/changelog.md) so `auki-network-browser-wasm` and `auki-domain-browser` are bindings/facades over shared Rust SDK logic rather than parallel browser implementations.

### Nils's codex · May 20, HKT, 2026

Reframed the browser Domain WebRTC implementation plan under [`plans/`](plans/changelog.md) so browser peers are full role-symmetric Domain peers, including Manager eligibility, with reachability handled as SDK transport state.

### Nils's codex · May 19, HKT, 2026

Added the SDK timekeeping foundation implementation plan under [`plans/`](plans/2026-05-19-sdk-timekeeping-foundation.md), making heartbeat time sync depend on a reusable `SessionClock` primitive first.

### Nils's codex · May 19, HKT, 2026

Added the domain-clock heartbeat time-sync implementation plan under [`plans/`](plans/2026-05-19-domain-clock-heartbeat-time-sync.md), scoping Manager-authored domain-clock transforms over `/auki/heartbeat/0.0.1`.

### Nils's codex · May 19, HKT, 2026

Added the native pointcloud SDK refactor design under [`specs/`](specs/changelog.md), documenting the approved breaking pointcloud contract for implementation planning.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain WebRTC join implementation plan under [`plans/`](plans/changelog.md), following the passing browser-to-native probe with production Manager WebRTC advertisement and browser join/info wiring.

### Nils's codex · May 19, HKT, 2026

Added the browser WebRTC probe stream implementation plan under [`plans/`](plans/changelog.md), sequencing the first browser-to-native SDK-owned protocol stream after the wasm libp2p feature compile probe passed.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport compile-probe implementation plan under [`plans/`](plans/changelog.md), sequencing the first measurable SDK browser networking spike before native dial work.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport spike spec under [`specs/`](specs/changelog.md), turning the browser transport question into a rust-libp2p Wasm proof plan before Domain join/audio.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter implementation plan under [`plans/`](plans/changelog.md), keeping first-tranche SDK package work separate from the later browser transport/audio plan.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter spec under [`specs/`](specs/changelog.md), documenting the SDK-side package and transport work needed for Park's browser-peer Milestone 0.
