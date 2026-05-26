# Auki SDK

On-device SDK for the Auki spatial-computing protocol — a Cargo workspace of Rust crates plus per-language bindings (Python via PyO3, Swift via UniFFI, TypeScript for the browser).

See [Vision.md](Vision.md) for the aspirational spec; this file describes what's actually in the repo today. [Glossary.md](Glossary.md) defines the domain terms.

## What this SDK does today

The Auki protocol is built around five questions any node should be able to answer about any other node — **Identity (who am I?)**, **Spatial (where did this happen?)**, **Temporal (when did this happen?)**, **Networking (how do I talk to you?)**, and **Tokenomics (how do I compensate you?)**. The SDK today implements the foundations for each.

### Identity

- **`Wallet`** — ed25519 keypair with deterministic child derivation. Mints libp2p peer identities via `Wallet::derive_child("peer/v1")` and signs creation certs.
- **`auki-jcs` + `auki-hash`** — RFC 8785 JSON canonicalization + XXH3-128 content-addressing. The hash IS the version; refining an entry is a sibling-write under the same id.
- **Sensor / Clock / Frame Registries** — content-addressed catalogs for every entity referenced by the logs. Logs pin their `sensor_hash` / `clock_hash` / `frame_hash`; spatial sensors pin an exact `frame_id` + `frame_hash`.

### Spatial

- **Pose Logs** — segmented `from → to` transforms keyed per ordered frame pair. The future `convert_pose` composition along a transform path is pending.
- **Sensor + Detection Logs** — per-frame sensor payloads (camera, point cloud, joint encoders, audio) and detector outputs.
- **`auki-geometry`** — convention conversion for points, vectors, directions, and `SpatialTransform` poses (the convention-only layer underneath the future full `convert_pose`).

### Temporal

- **TimeTransform Logs** — segmented sampled offsets between two clocks. Combined with NTP-style exchanges between peers, they let a cluster converge on a shared **domain time** so events recorded against different local clocks can be aligned.
- **`auki-time`** — `SessionClock`, pure `TimeTransform` math, NTP-style offset samples (`NtpExchange`, `NtpSample`, `compute_ntp_sample`, `select_best_ntp_sample`), and the 1 Hz `local_clock_read` sampler that writes the TimeTransform Log. The `convert_time` operation that consumes the log is pending.

### Networking

- **libp2p substrate** (TCP/QUIC, Noise, Yamux, Circuit Relay v2) with typed `/auki/stream/0.1.0` streams for camera, point-cloud, joint-encoder, and audio payloads. Native Managers can reserve a relay-mediated circuit address through a Domain Relay and publish the relay base metadata through Discovery for browser peers.
- **Peer protocols**: `/auki/join`, `/auki/heartbeat`, `/auki/membership`, `/auki/info`, `/auki/resources`, `/auki/sensors`, `/auki/registries`.
- **`ClusterManager`** — single app-facing entry point for Discovery + cluster bootstrap, membership, Manager election, relay hint preservation, resource catalogs, registry fetch, and stream open.
- **RFC-first networking path** — `auki-protocol` owns the pure v1 protocol
  types and `auki-p2p` is the new clean libp2p runtime for configured peers,
  lifecycle authorization, offer loading, Get/Subscribe, local Get providers,
  and status snapshots. It is intentionally separate from the shipped
  `auki-network` / `ClusterManager` path while it matures.
- **HTTP control API** for daemons that produce SDK sessions — see [`docs/control-api.md`](docs/control-api.md).

### Tokenomics

- Not implemented. The `Wallet` under Identity is the on-device primitive future payment / billing rails will bind to.

Cross-cutting gaps not in any of the five buckets above: a `Session` abstraction tying clock + sensor-id minting + recording lifecycle together, the Detector binding API, and live pose-stream / detection-resource rows in `/auki/resources/0.0.1`. Tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

## Crate map

### `crates/` — Rust workspace (plus one TypeScript browser package)

| Crate | Purpose | Status |
|---|---|---|
| [`auki-hash`](crates/auki-hash) | XXH3-128 wrapper for content-addressing | ✓ |
| [`auki-jcs`](crates/auki-jcs) | RFC 8785 JSON canonicalization | ✓ |
| [`auki-identity`](crates/auki-identity) | ed25519 wallet + child derivation + signed creation certs | ✓ |
| [`auki-time`](crates/auki-time) | `SessionClock`, `TimeTransform` math, NTP-style sampler | ✓ |
| [`auki-logs`](crates/auki-logs) | Generic segmented append-only log primitive | ✓ |
| [`auki-registry`](crates/auki-registry) | Sensor / Clock / Frame identity catalogs + IO | ✓ |
| [`auki-datatypes`](crates/auki-datatypes) | Shared protobuf segment + wire payload schemas | ✓ |
| [`auki-manifests`](crates/auki-manifests) | JCS-JSON log-manifest builders (sensor / pose / TT / detection) | ✓ |
| [`auki-layout`](crates/auki-layout) | On-disk path helpers for session/log layout | ✓ |
| [`auki-geometry`](crates/auki-geometry) | Convention conversion for points / vectors / poses | ✓ |
| [`auki-network`](crates/auki-network) | libp2p substrate, typed streams, Discovery HTTP client with Manager and relay address hints, peer protocols | ✓ |
| [`auki-protocol`](crates/auki-protocol) | RFC-first v1 protocol types, frames, signed authority objects, lifecycle, offers, Get, Subscribe, status | WIP (v0.0.0) |
| [`auki-p2p`](crates/auki-p2p) | Clean RFC-first libp2p runtime with configured peers, lifecycle authorization, offer loading, Get/Subscribe, Get providers, status snapshots | WIP (v0.0.0) |
| [`auki-domain`](crates/auki-domain) | `ClusterManager` — app-facing cluster lifecycle facade with relay hint preservation | ✓ |
| [`auki-domain-relay`](crates/auki-domain-relay) | Domain Relay capability for browser-compatible reachability | WIP (v0.0.0) |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ROS2 → SDK glue for `Image` / `CameraInfo` / `PointCloud2` | ⚠ broken at the `r2r 0.9.5` transport layer |
| [`auki-network-browser-wasm`](crates/auki-network-browser-wasm) | Browser/WASM libp2p transport probe | WIP (v0.0.0) |
| [`auki-domain-browser`](crates/auki-domain-browser) | TypeScript browser `Peer` contract types | WIP (v0.0.0) |

### `bindings/python/` — PyO3 / betterproto

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-py`](bindings/python/auki-identity-py) | Wallet + per-machine identity | ✓ |
| [`auki-datatypes-py`](bindings/python/auki-datatypes-py) | betterproto dataclasses for the shared protobuf types | ✓ |
| [`auki-logs-py`](bindings/python/auki-logs-py) | `Log<T>` with opaque-bytes payload | ✓ |
| [`auki-registry-py`](bindings/python/auki-registry-py) | Sensor / Clock / Frame registry IO | ✓ |
| [`auki-manifests-py`](bindings/python/auki-manifests-py) | Log-manifest builders | ✓ |
| [`auki-layout-py`](bindings/python/auki-layout-py) | On-disk path helpers | ✓ |
| [`auki-network-py`](bindings/python/auki-network-py) | Discovery client with relay hints + shared stream pyclasses | ✓ |
| [`auki-domain-py`](bindings/python/auki-domain-py) | `ClusterManager` Python facade | ✓ |
| [`auki-session-py`](bindings/python/auki-session-py) | Source-of-truth Python control-plane surface | WIP (scaffolding) |

### `bindings/swift/` — UniFFI

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-swift`](bindings/swift/auki-identity-swift) | `Wallet` + `PeerIdentity` | ✓ |
| [`auki-network-swift`](bindings/swift/auki-network-swift) | Discovery client + `NetworkRuntime` + 5-payload streams | ✓ |

Each package's own `README.md` documents its current state, public surface, and dependencies.

## Contributing & license

Work is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). See [CONTRIBUTING.md](CONTRIBUTING.md) for the folder convention, board flow, and git hygiene rules; [CLAUDE.md](CLAUDE.md) is the equivalent for AI agents.

MIT — see [LICENSE](LICENSE). Copyright © 2026 Auki Labs Limited.
