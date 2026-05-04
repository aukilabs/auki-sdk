# Parking lot — crates

Cross-crate questions, plus a topic summary of per-crate parking lots.

---

## Schema versioning coordination

Each crate that owns a wire format pins its own version: `auki-logs` segment format v1, `auki-registry` entry schema v1, `auki-time-transforms` payload v1. They're independent today. When any one bumps to v2, what's the coordination story for consumers? Does the manifest need a per-log version field separate from the entry schema, or is the segment-format version the single source of truth?

## src/sprint.md per-crate scaffolding missing

The convention specifies `src/sprint.md` per crate (current work + next steps). None of the eight crates have one. They'd need seeding before the convention is fully realized.

---

## Per-crate parking lots

- [`auki-hash/`](auki-hash/parking_lot.md) — cryptographic strength upgrade path
- [`auki-identity/`](auki-identity/parking_lot.md) — BIP32-vs-labeled-hash derivation; encrypted-at-rest format; BIP39 mnemonics; signing-scheme v2 shape
- [`auki-jcs/`](auki-jcs/parking_lot.md) — `serde_jcs` upstream vendoring strategy
- [`auki-logs/`](auki-logs/parking_lot.md) — per-entry checksums; reader streaming for unbounded captures
- [`auki-network/`](auki-network/parking_lot.md) — peer-derivation label evolution; `ReachabilityRecord` extensibility; `SwarmConfig` knob minimalism; `BuildError::Transport` structure; loopback `add_external_address` workaround note; DCUtR/hole-punching as future work; Cluster Registry primitive evolution (does `cluster.json` graduate?); `cluster.json` signing; operator UX for peer-id discovery; `app_instance` container/Docker handling, multi-NIC tiebreaker, eventual stable-id options. (3 Reid M2 questions resolved + propagated 2026-05-02 in M1b code.)
- [`auki-registry/`](auki-registry/parking_lot.md) — Frame Registry shape; UTC clock epoch format; sensor_id naming convention formalization; atomic-write tmp cleanup
- [`auki-ros-adapter/`](auki-ros-adapter/parking_lot.md) — `r2r` typesupport blocker
- [`auki-session/`](auki-session/parking_lot.md) — TimeTransform log path encoding ambiguity
- [`auki-time-transforms/`](auki-time-transforms/parking_lot.md) — future `TimeTransformSource` variants
