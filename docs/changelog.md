# Changelog — docs

Append-only timeline of documentation changes under `docs/`. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Added the Superpowers stream naming cleanup design spec for the SDK-wide full rename of camera, detection, and camera registry vocabulary.

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

### Nils's codex · May 19, HKT, 2026

Added the Superpowers SDK timekeeping foundation plan and marked heartbeat time sync as dependent on a reusable `SessionClock` primitive.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers implementation plan for syncing cluster peers to a Manager-authored domain clock over heartbeat-carried TimeTransform samples.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers native pointcloud design spec and propagated the docs-level changelog chain for the SDK pointcloud refactor planning artifact.
