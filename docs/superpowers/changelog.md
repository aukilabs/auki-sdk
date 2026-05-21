# Changelog — docs/superpowers

Append-only timeline of Superpowers design artifacts. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup implementation plan under [`plans/`](plans/2026-05-21-stream-naming-cleanup.md), sequencing the full no-compatibility rename across datatypes, registry, network, bindings, domain, ROS adapter, docs, and Park.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup design under [`specs/`](specs/2026-05-21-stream-naming-cleanup-design.md), specifying the no-compatibility rename to `CameraFrame`, `DetectionFrame`, and `SensorBody::Camera`.

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

### Nils's codex · May 19, HKT, 2026

Added the SDK timekeeping foundation implementation plan under [`plans/`](plans/2026-05-19-sdk-timekeeping-foundation.md), making heartbeat time sync depend on a reusable `SessionClock` primitive first.

### Nils's codex · May 19, HKT, 2026

Added the domain-clock heartbeat time-sync implementation plan under [`plans/`](plans/2026-05-19-domain-clock-heartbeat-time-sync.md), scoping Manager-authored domain-clock transforms over `/auki/heartbeat/0.0.1`.

### Nils's codex · May 19, HKT, 2026

Added the native pointcloud SDK refactor design under [`specs/`](specs/changelog.md), documenting the approved breaking pointcloud contract for implementation planning.
