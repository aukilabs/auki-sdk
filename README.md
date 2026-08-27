# Auki SDK

On-device SDK for the Auki spatial-computing protocol — a Cargo workspace of Rust crates plus per-language bindings (Python via PyO3, Swift via UniFFI, TypeScript for the browser).

See [VISION.md](VISION.md) for the aspirational spec; this file describes what's actually in the repo today. [GLOSSARY.md](GLOSSARY.md) defines the domain terms. Existing Manager-era consumers should start with the [authenticated Domain migration guide](docs/authenticated-domain-migration.md).

## Stage 1 release line

The authenticated native/Python release candidate is the coordinated `0.1.0`
line. The active Cargo workspace declares an MSRV of Rust `1.89.0`, inherited
by every workspace package and checked at that exact toolchain before release.

| Artifact | Stage 1 version/distribution |
|---|---|
| SDK source | coordinated Git tag `v0.1.0` after the release gate |
| `auki-p2p` | crate `0.1.0`; publish before dependent crates |
| `auki-p2p-dataset` | crate `0.1.0`; publish after `auki-p2p 0.1.0` |
| Rust `auki-domain` | crate version `0.1.0`, consumed from the coordinated Git tag in Stage 1 |
| Python Domain/Session | exact paired wheels `auki-domain-py==0.1.0` and `auki-session-py==0.1.0` from one build |
| Swift and browser | prior Manager-era source tag `v0.0.60`; not wire-compatible with Stage 1 |

No `v0.1.0` tag or registry package is published by this source checkout.
Publishing is a separate release action after the local gate; unsupported
platforms remain retrievable at `v0.0.60` until their later breaking stages.

## What this SDK does today

The Auki protocol is built around five questions any node should be able to answer about any other node — **Identity (who am I?)**, **Spatial (where did this happen?)**, **Temporal (when did this happen?)**, **Networking (how do I talk to you?)**, and **Tokenomics (how do I compensate you?)**. The SDK today implements the foundations for each.

### Identity

- **`Wallet`** — ed25519 keypair with deterministic child derivation. Mints libp2p peer identities via `Wallet::derive_child("peer/v1")` and signs creation certs.
- **`auki-jcs` + `auki-hash`** — RFC 8785 JSON canonicalization + XXH3-128 content-addressing. The hash IS the version; refining an entry is a sibling-write under the same id.
- **Sensor / Clock / Frame Registries** — content-addressed catalogs for every entity referenced by the logs. Logs pin their `sensor_hash` / `clock_hash` / `frame_hash`; spatial sensors pin an exact `frame_id` + `frame_hash`.

### Spatial

- **Pose Logs** — segmented `from → to` transforms keyed per ordered frame pair. The future `convert_pose` composition along a transform path is pending.
- **Sensor + Detection Logs** — per-frame sensor payloads (camera, point cloud, joint encoders, audio) and detector outputs.
- **Maps + Mappers** — content-addressed Map resources, durable Map Logs, and SDK-native producers that transform sensor/pose streams into mergeable Map Updates.
- **Application-controlled components** — bring-your-own Detectors and Mappers run only when an application starts an instance. Live runners isolate blocking work behind bounded latest-wins queues; local Sensor Log detector replay remains ordered and exhaustive. Detection Logs and Map Logs, rather than component instances, are the Resources peers consume.
- **`auki-geometry`** — convention conversion for points, vectors, directions, and `SpatialTransform` poses (the convention-only layer underneath the future full `convert_pose`).

### Temporal

- **TimeTransform Logs** — segmented, application-produced offsets between two
  explicitly identified clocks. They preserve recorded temporal lineage
  without electing a Domain clock or deriving time authority from networking.
- **`auki-time`** — `SessionClock` and pure affine `TimeTransform` math. The
  `convert_time` operation that consumes recorded transforms is pending.

### Networking

- **`auki-p2p` authenticated transport** owns the stable libp2p identity,
  DDS-signed Domain credentials, mutual authentication, explicit direct/relay
  routes, relay reservations, and live authenticated-peer observations.
- **Authenticated application protocols** use `/auki/auth/1/...` IDs for info,
  resource catalogs v0.2/v0.3/v0.4, registries v0.2/v0.3, blobs, messages,
  and typed streams. `auki-network` retains their bounded codecs and plain
  application types; it owns no swarm or control plane.
- **`Peer` / `Session` / `Domain`**: a long-lived `Peer` owns identity and
  registries; a `Session` owns one recording timeline and its logs; `Domain`
  owns one authenticated P2P node for one exact DDS Domain UUID. The host
  supplies credentials and explicit routes. There is no Manager, election,
  membership roster, Discovery HTTP dependency, or hidden topology policy.
- The resource catalog exposes rows discriminated by `variant`
  (`sensor_log` | `pose_log` | `time_transform_log` | `detection_log`) and is
  sampled live from the provider the Domain serves. Registry/blob reads are
  content-pinned, messages are receiver-owned, and typed streams bind the
  expected authenticated producer before application bytes flow.
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
| [`auki-time`](crates/auki-time) | `SessionClock`, fixed affine `TimeTransform` math, clock traits, and local sampling primitives; no network-derived Domain clock | ✓ |
| [`auki-logs`](crates/auki-logs) | Generic segmented append-only log primitive | ✓ |
| [`auki-registry`](crates/auki-registry) | Sensor / Clock / Frame / Map identity catalogs + IO | ✓ |
| [`auki-datatypes`](crates/auki-datatypes) | Shared protobuf segment + wire payload schemas | ✓ |
| [`auki-manifests`](crates/auki-manifests) | JCS-JSON log-manifest builders (sensor / pose / TT / detection / map) | ✓ |
| [`auki-layout`](crates/auki-layout) | On-disk path helpers for session/log layout | ✓ |
| [`auki-geometry`](crates/auki-geometry) | Convention conversion for points / vectors / poses | ✓ |
| [`auki-maps`](crates/auki-maps) | Deterministic voxel Map accumulation + renderer-neutral chunk updates | ✓ |
| [`auki-mappers`](crates/auki-mappers) | SDK-native Map producers; point-cloud + pose voxel Mapper | ✓ |
| [`auki-p2p`](crates/auki-p2p) | Authenticated libp2p runtime, stable identity, explicit direct/relay routes, relay reservations, and peer observations | ✓ |
| [`auki-p2p-dataset`](crates/auki-p2p-dataset) | Authenticated content-addressed dataset transfer protocol | ✓ |
| [`auki-network`](crates/auki-network) | Bounded authenticated-protocol codecs and plain networking/application types; no swarm owner | ✓ |
| [`auki-session`](crates/auki-session) | Declarative app API: `Peer` (identity + registries) + `Session` (clocks + log registration); network-free | ✓ |
| [`auki-domain`](crates/auki-domain) | Public authenticated `Domain` lifecycle over one DDS Domain UUID, with explicit authority/routes and retained catalogs, registries, blobs, messages, and streams | ✓ |
| [`auki-domain-relay`](crates/auki-domain-relay) | Domain Relay capability for browser-compatible reachability | WIP (v0.0.0) |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ROS2 → SDK glue for `Image` / `CameraInfo` / `PointCloud2` | ⚠ broken at the `r2r 0.9.5` transport layer |
| [`auki-network-browser-wasm`](crates/auki-network-browser-wasm) | Prior Rust/WASM runtime, retained outside the active workspace until the browser stage replaces it | Legacy line |
| [`auki-domain-browser`](crates/auki-domain-browser) | TypeScript browser facade; authenticated-engine migration is a later stage | WIP (v0.0.0) |

### `bindings/python/` — PyO3 / betterproto

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-py`](bindings/python/auki-identity-py) | Wallet + per-machine identity | ✓ |
| [`auki-datatypes-py`](bindings/python/auki-datatypes-py) | betterproto dataclasses for the shared protobuf types | ✓ |
| [`auki-logs-py`](bindings/python/auki-logs-py) | `Log<T>` with opaque-bytes payload | ✓ |
| [`auki-registry-py`](bindings/python/auki-registry-py) | Sensor / Clock / Frame registry IO | ✓ |
| [`auki-manifests-py`](bindings/python/auki-manifests-py) | Log-manifest builders | ✓ |
| [`auki-layout-py`](bindings/python/auki-layout-py) | On-disk path helpers | ✓ |
| `auki-network-py` | Removed Manager-era networking binding; use `auki-domain-py` | Removed |
| [`auki-domain-py`](bindings/python/auki-domain-py) | Python binding over the same authenticated `Domain` owner as Rust | ✓ |
| [`auki-mappers-py`](bindings/python/auki-mappers-py) | Python boundary for SDK-native Mappers; normalized point cloud + registry + pose to `MapUpdate` | ✓ |
| [`auki-session-py`](bindings/python/auki-session-py) | Python binding for `auki-session` — `Peer`, `Session`, registries, log specs, and handles | ✓ |

### `bindings/swift/` — UniFFI

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-swift`](bindings/swift/auki-identity-swift) | `Wallet` + `PeerIdentity` | ✓ |
| [`auki-network-swift`](bindings/swift/auki-network-swift) | Prior Manager-compatible package line, retained outside the active workspace until the Swift stage | Legacy line |

Each package's own `README.md` documents its current state, public surface, and dependencies.

## Contributing & license

Work is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). See [CONTRIBUTING.md](CONTRIBUTING.md) for the folder convention, board flow, and git hygiene rules; [CLAUDE.md](CLAUDE.md) is the equivalent for AI agents.

MIT — see [LICENSE](LICENSE). Copyright © 2026 Auki Labs Limited.
