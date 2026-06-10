# Auki SDK

On-device SDK for the Auki spatial-computing protocol — a Cargo workspace of Rust crates plus per-language bindings (Python via PyO3, Swift via UniFFI, TypeScript for the browser).

See [VISION.md](VISION.md) for the aspirational spec; this file describes what's actually in the repo today. [Glossary.md](Glossary.md) defines the domain terms.

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

- **libp2p substrate** (TCP/QUIC, Noise, Yamux, Circuit Relay v2) with typed `/auki/stream/0.2.0` streams for camera, point-cloud, joint-encoder, audio, and live pose `SpatialTransform` payloads. Native Managers can reserve a relay-mediated circuit address through a Domain Relay and publish the relay base metadata through Discovery for browser peers.
- **Peer protocols**: `/auki/join`, `/auki/heartbeat`, `/auki/membership`, `/auki/info`, `/auki/resources/0.2.0`, `/auki/registries/0.2.0`.
- **`Peer` / `Session` / `Domain`** — the app-facing API split (#282): a long-lived `Peer` (in `auki-session`) owns identity and the sensor / frame / detector registries and mints `Session`s — one timeline each, with a ULID session id and auto-registered monotonic + UTC clocks (#284) — which register the logs they write. `auki-domain`'s `Domain::join(&peer, &session, config)` puts the pair on the network and serves the resource catalog; `ClusterManager` is the engine `Domain` constructs and owns.
- The resource catalog (`/auki/resources/0.2.0`) exposes rows discriminated by a `variant` field (`sensor_log` | `pose_log` | `time_transform_log` | `detection_log`), replacing the old `sensor_stream` / `transform_edge` / `pose_stream` row types.
- `/auki/resources/0.2.0` is a live, pollable snapshot of resources that can currently accept stream opens. Peers may join before resources are ready; consumers such as Park poll and reconcile additions/removals; producers omit unavailable resources and re-add them later with the same stable `resource_id`.
- **HTTP control API** for daemons that produce SDK sessions — see [`docs/control-api.md`](docs/control-api.md).

### Tokenomics

- Not implemented. The `Wallet` under Identity is the on-device primitive future payment / billing rails will bind to.

The first live pose-stream hardware target is Galbot G1 using RoboStreamer to publish `base_link -> head_left_rgb_optical` pose logs into Park. Tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

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
| [`auki-network`](crates/auki-network) | libp2p substrate, typed camera/point-cloud/joint-encoder/audio/pose streams, Discovery HTTP client with Manager and relay address hints, peer protocols | ✓ |
| [`auki-session`](crates/auki-session) | Declarative app API: `Peer` (identity + registries) + `Session` (clocks + log registration); network-free | ✓ |
| [`auki-domain`](crates/auki-domain) | `Domain::join(&peer, &session, config)` — app-facing network presence; owns `ClusterManager`, the cluster lifecycle engine | ✓ |
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
| [`auki-network-py`](bindings/python/auki-network-py) | Discovery client with relay hints + shared stream pyclasses, including `SpatialTransformFrame` | ✓ |
| [`auki-domain-py`](bindings/python/auki-domain-py) | `ClusterManager` Python facade with `ResourceEntry`, resource-catalog fetch, and typed stream openers including pose | ✓ |
| [`auki-session-py`](bindings/python/auki-session-py) | Python binding for `auki-session` — `Session`, register_*, log specs/handles, `catalog()` | ✓ |

### `bindings/swift/` — UniFFI

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-swift`](bindings/swift/auki-identity-swift) | `Wallet` + `PeerIdentity` | ✓ |
| [`auki-network-swift`](bindings/swift/auki-network-swift) | Discovery client + `NetworkRuntime` + 5-payload streams | ✓ |

Each package's own `README.md` documents its current state, public surface, and dependencies.

## Contributing & license

Work is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). See [CONTRIBUTING.md](CONTRIBUTING.md) for the folder convention, board flow, and git hygiene rules; [CLAUDE.md](CLAUDE.md) is the equivalent for AI agents.

MIT — see [LICENSE](LICENSE). Copyright © 2026 Auki Labs Limited.
