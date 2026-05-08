# Parking lot — crates

Cross-crate questions, plus a topic summary of per-crate parking lots.

---

## Schema versioning coordination

Each crate that owns a wire format pins its own version: `auki-logs` segment format v1, `auki-registry` entry schema v1, `auki-time-transforms` payload v1. They're independent today. When any one bumps to v2, what's the coordination story for consumers? Does the manifest need a per-log version field separate from the entry schema, or is the segment-format version the single source of truth?

## src/sprint.md per-crate scaffolding missing

The convention specifies `src/sprint.md` per crate (current work + next steps). None of the eight crates have one. They'd need seeding before the convention is fully realized.

---

## Rust vs Python surface namespacing — pick one and converge _(filed by Dobby, 2026-05-08)_

The [`auki-network-py`](auki-network-py) Python surface is namespaced into submodules: `auki_network.cluster.*` (`ClusterRuntime`, `ParticipantInfo`, `JpegFrame`, `PointCloudFrame`, `StreamDispatch`, …) and `auki_network.discovery.*` (`DiscoveryClient`). The corresponding Rust [`auki-network`](auki-network) surface is **flat** — every submodule (`cluster_doc`, `cluster_protocol`, `cluster_runtime`, `stream_protocol`, `stream_runtime`, `participant`, `swarm`, `app_instance`, `discovery_client`) is `pub mod`-published at the crate root, but the *types* themselves (`ParticipantInfo`, `Capability`, `PeerIdentity`, …) are re-exported flat at the crate root via `pub use`.

A consumer reading the two side-by-side gets two different mental models of the same surface. `cluster_doc::ClusterDoc` in Rust vs `auki_network.cluster.ClusterDoc` in Python — same type, different access path. Doc effort is duplicated; consumer learning doesn't transfer. The May 2026 [Python bindings strategy decision](../parking_lot.md#python-bindings-strategy) (per-component naming, `auki-network-py` over umbrella `auki-py`) commits the workspace to multiple `*-py` crates over time — `auki-logs-py`, `auki-session-py`, `auki-registry-py`, etc. The longer this divergence sits, the more bindings will replicate the mismatch.

Two forward paths:

1. **Mirror Python's namespacing in Rust.** Introduce real submodules at the Rust crate root: `auki_network::cluster::{ParticipantInfo, ClusterRuntime, ClusterDoc, …}`, `auki_network::discovery::DiscoveryClient`, possibly `auki_network::stream::{StreamProvider, StreamDispatch, JpegFrame, PointCloudFrame, …}`. Drop the flat `pub use` re-exports (or keep them with `#[deprecated]` for one release). Pre-1.0; non-breaking is cheap right now.
2. **Flatten Python to match Rust.** Less likely the right move — Python's submodules are good (small surface per module, easy to reason about), and namespace pollution at the package root is worse-than-Rust ergonomics in Python.

Lean: (1). Aligning Rust submodules to Python namespacing is the cheaper convergence — Rust gets clearer organization at no cost while still pre-1.0; Python keeps its current shape; the future Rust runtime `Session` (now reserved at `auki-session`, separate from the layout-only [`auki-layout`](auki-layout) crate) would naturally pair with `auki_session_py.session.Session` under this convention. Apply the same convention to future `*-py` crates as they land.

Pin before the next public-API touch on `auki-network`. Surfacing for editorial pass; not gating in-flight work.

---

## Per-crate parking lots

- [`auki-hash/`](auki-hash/parking_lot.md) — cryptographic strength upgrade path
- [`auki-identity/`](auki-identity/parking_lot.md) — BIP32-vs-labeled-hash derivation; encrypted-at-rest format; BIP39 mnemonics; signing-scheme v2 shape; **missing `Result<T>` aliases** (filed 2026-05-08)
- [`auki-identity-py/`](auki-identity-py/parking_lot.md) — PyPI distribution policy; locked-`peer_id`-vector regeneration follow-up; Python version floor (the once-deferred async/Swarm bindings shipped as `auki-network-py` per the per-component naming decision)
- [`auki-network-py/`](auki-network-py/parking_lot.md) — `pyo3-log` routing (deferred — stderr → journald is fine for K1 today); two-runtime E2E test uses fixed loopback ports (no introspection API for OS-chosen addresses); PyPI distribution policy (alongside `auki-identity-py`'s); type stubs (`auki_network.pyi`); single-task tokio runtime sizing; Python version floor
- [`auki-jcs/`](auki-jcs/parking_lot.md) — `serde_jcs` upstream vendoring strategy
- [`auki-logs/`](auki-logs/parking_lot.md) — per-entry checksums; reader streaming for unbounded captures. (Encoder-aware vs encoder-agnostic `Log<T>` resolved 2026-05-08 in Step 1 of the auki-datatypes migration — encoding-agnostic via `LogPayload` trait.)
- [`auki-network/`](auki-network/parking_lot.md) — **Vinland `discovery_client` `DiscoveryRuntime`** (re-register / poll loop; deferred until Discovery v2 grows TTL or push); **Vinland `discovery_client` TLS knobs** (custom roots / proxy via `with_http` escape hatch today; first-class kwargs deferred); peer-derivation label evolution; `ReachabilityRecord` extensibility; `SwarmConfig` knob minimalism; `BuildError::Transport` structure; loopback `add_external_address` workaround note; DCUtR/hole-punching as future work; Cluster Registry primitive evolution (does `cluster.json` graduate?); `cluster.json` signing; operator UX for peer-id discovery; `app_instance` container/Docker handling, multi-NIC tiebreaker, eventual stable-id options; **grimsby `stream_protocol` JSON-of-binary encoding inefficiency** (RESOLVED 2026-05-06 for Dagaz's new `PointCloudFrame` via `#[serde(with = "base64_bytes")]` adapter; still deferred for `JpegFrame` since the swap would re-renegotiate every grimsby v1 consumer); **`libp2p-stream` 0.4.0-alpha exact pin** (libp2p umbrella in 0.56 doesn't expose `stream` as a feature; relax to `^X.Y` once upstream stabilizes); **`Capability(pub String)` open-string vs typed enum** (filed 2026-05-08); **`PEER_DERIVATION_LABEL` constant in wrong crate** (filed 2026-05-08); **`StreamDispatch` streaming-stability lever README disclosure** (filed 2026-05-08). (3 Reid M2 questions resolved + propagated 2026-05-02 in M1b code.)
- [`auki-registry/`](auki-registry/parking_lot.md) — UTC clock epoch format; sensor_id naming convention formalization (**raised 2026-05-08 to "load-bearing for cross-peer recording provenance"** per the [root subscription-as-materialization keystone](../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08)); atomic-write tmp cleanup; log-payload sections in README during migration (Frame Registry shape RESOLVED 2026-05-07 — `FrameRegistryEntry { handedness, axes, units }` + four preset constructors landed in v0.0.22, no `parent_frame` on the entry, no `label`)
- [`auki-ros-adapter/`](auki-ros-adapter/parking_lot.md) — `r2r` typesupport blocker
- [`auki-layout/`](auki-layout/parking_lot.md) — TimeTransform log path encoding ambiguity; **crate renamed `auki-session` → `auki-layout` 2026-05-08** (resolved)
- [`auki-time-transforms/`](auki-time-transforms/parking_lot.md) — future `TimeTransformSource` variants
