# Auki SDK

The on-device SDK for the Auki **real world web** — a collaborative spatial computing protocol that lets devices, robots, and services share, locate, and reason about the physical world.

> *The internet made information browsable. The Auki network makes the physical world browsable.*

---

## What this SDK is for

Each physical space — a warehouse, a hospital ward, a retail floor — gets a **Domain**: a unique tag applied to data, asserting that the data is *about* that space. Domains are to the real world what websites are to the internet — privately owned, independently operated, linked by a shared protocol. A Domain has one or more **scenegraphs** — structured representations of typed nodes (frames, sensors, clocks) connected by transform edges. The Domain Owner designates one as the canonical **Map**, served by default when a peer asks for "the map" without naming a scenegraph. See [Glossary.md](Glossary.md) for the full term list, including the distinction between Domain ID, Scenegraph ID, and Session ID.

The Auki protocol exists so any node — a phone, a robot, a cloud server, a browser tab — can answer four questions about any other node:

- **Where is this?**
- **When was this?**
- **How can I talk to you?**
- **How can I compensate you?**

This SDK is the on-device library that lets a node participate. It captures sensor data with the right identity and timing metadata, maintains a local scene graph, and (in time) crosses domain boundaries through the protocol's transform-composition operations.

---

## Spatio-temporal reasoning

A node's local scene graph is the combined output of **four append-only logs** and **three registries**. Together they answer "where was X at time *t*" through composition along a transform path:

```
T_X_session(t) = T_body_session(t) ∘ T_X_body(t)
```

Each transform edge is looked up or interpolated at time *t* in a Pose Log; the chain is composed into the answer the caller asked for.

### The four logs

| Log | What it stores |
|---|---|
| **Pose Log** | Timestamped `from → to` transforms — answers "where was this frame over time" |
| **Sensor Log** | Per-frame sensor payloads (camera frames, point clouds, IMU samples), keyed to a sensor and a clock |
| **Detection Log** | Per-frame detection outputs from extractors |
| **TimeTransform Log** | Sampled offsets between two clocks — answers "given a timestamp on clock A, what's the equivalent on clock B" |

### The three registries

| Registry | What it holds |
|---|---|
| **Frame Registry** | Coordinate convention metadata for named frames (handedness, axes, units, rotation semantics) |
| **Sensor Registry** | Per-sensor identity and interpretation metadata (type, dimensions, format, sample rate) |
| **Clock Registry** | Per-clock identity and semantics (monotonic vs UTC, scope, epoch) |

Logs reference registries by **content-addressed hash**: the hash IS the version. Refining a registry entry is a sibling-write under the same id; downstream consumers pin a specific hash deliberately.

---

## What's implemented today

This repo is in early development. The crates here implement a foundational subset of the architecture:

| Crate | Status |
|---|---|
| [`auki-logs`](crates/auki-logs) | ✓ Generic segmented ring-buffer log primitive — manifest + segment files + retention eviction |
| [`auki-registry`](crates/auki-registry) | ✓ Sensor + Clock registry types and IO; Sensor Log + Point Cloud Log + Audio Log + Pose Log payload schemas (capture-only for Pose Log; `convert_pose` pending). Frame Registry pending |
| [`auki-jcs`](crates/auki-jcs) | ✓ RFC 8785 JSON canonicalization (used for stable hashing of registry entries) |
| [`auki-hash`](crates/auki-hash) | ✓ XXH3-128 wrapper used for registry content-addressing |
| [`auki-time-transforms`](crates/auki-time-transforms) | ✓ Clock sampler primitives for the TimeTransform Log |
| [`auki-session`](crates/auki-session) | ✓ Path helpers for the on-disk session shape — single source of truth for app/session/recording layout |
| [`auki-identity`](crates/auki-identity) | ✓ Wallet primitive: ed25519 keypairs, deterministic child derivation, signed creation certs. WASM-friendly |
| [`auki-identity-py`](crates/auki-identity-py) | ✓ PyO3 bindings for a tiny slice of the SDK — `load_or_mint_seed`, `Wallet.from_seed/derive_child/peer_id`, `app_instance.derive`. Stepping-stone Python package for Boosterapp's `/api/info` v0.0.11 sidecar; the full `auki-py` MVP (Swarm + async) is a separate, larger track |
| [`auki-network`](crates/auki-network) | ✓ M0 + M1: peer identity (derived from wallet via `derive_child("peer/v1")`), reachability records, named capabilities, plus a libp2p `Swarm` builder behind the `swarm` feature (TCP/QUIC + Noise + Yamux + Circuit Relay v2 + mDNS + identify + ping; dial-by-peer-id helper for circuit-relay multiaddrs). All Reid M2 networking parking-lot questions resolved |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ⚠ Generic ROS2 → SDK glue: `CameraInfo`/`Image` and `PointCloud2` translation, with RGB/RGBA normalization for point clouds. Currently broken at the transport layer: `r2r` 0.9.5's compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout some camera drivers publish. Fix in flight |

**Not yet implemented:**

- `convert_pose` (the Pose Log primitives — `PoseLogEntry`, `PoseSource`, `build_pose_log_manifest`, `poselog_path` — are in place for capture and read; the `convert_pose` operation that composes pose paths is pending)
- Detection Log
- Frame Registry
- `convert_time` (the TimeTransform Log primitives exist; the `convert_time` operation that consumes them does not yet)
- Domain registration / clustering / discovery (these live in higher layers; the SDK provides primitives, not the network protocol itself)
- The full `auki-py` MVP — libp2p Swarm + async / Tokio integration. The minimal `auki-identity-py` slice (`load_or_mint_seed`, `Wallet.from_seed/derive_child/peer_id`, `app_instance.derive`) ships today as a stepping stone for Boosterapp's `/api/info` v0.0.11 sidecar

---

## On-disk format

Logs and registries write to a documented binary + JSON format. Each format spec lives with the crate that owns it — they're the source of truth for any reader, in any language:

- [`auki-logs`](crates/auki-logs/README.md) — segmented ring-buffer log layout (used by both Sensor and TimeTransform Logs)
- [`auki-registry`](crates/auki-registry/README.md) — registry entry storage layout, plus the Sensor Log payload schema and the ROS2 → SDK field mapping
- [`auki-time-transforms`](crates/auki-time-transforms/README.md) — TimeTransform Log payload schema and sampling protocol
- [`auki-session`](crates/auki-session/README.md) — the path layout and helpers below

Files within an app:

```
<app_root>/
├── registries/
│   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json     ← coming
└── <session_id>/
    ├── timetransform_logs/<from_id>__<to_id>/
    │   ├── manifest.json
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    ├── sensorlogs/
    │   ├── <recording_uuid_1>/            ← one sensor stream per recording
    │   │   ├── manifest.json
    │   │   └── segments/<padded-ns>.seg
    │   ├── <recording_uuid_2>/
    │   │   └── ...
    │   └── <recording_uuid_3>/
    └── poselogs/
        ├── <recording_uuid_1>/            ← one pose source per recording
        │   ├── manifest.json
        │   └── segments/<padded-ns>.seg
        └── <recording_uuid_2>/
```

`<app_root>` is chosen by the integrator (boosterapp uses `/home/booster/auki/boosterapp/`); the SDK doesn't enforce structure above the registries. Registries live at the app root because hash-keyed writes are idempotent — a sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session, so per-session copies would be wasted work.

**A recording is one sensor stream.** Each `<recording_uuid>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor. Multi-sensor capture means multiple parallel recordings sharing a session, not a multi-sensor recording. The auto-started ring buffer is just a recording with `retention_ns: 30s`; intent captures are recordings with `retention_ns: 0`. The sensor identity lives in the log's manifest (`sensor_id` + `sensor_hash`), not in the path.

This shape is **breaking from v0.0.6** (the inner `<sensor_id>/` layer is gone).

---

## API surface

The SDK exposes four distinct API surfaces. They serve different audiences and live in different layers of the stack.

### 1. Rust crate APIs

The on-device library, organized as a Cargo workspace. Each crate is independently versioned via the repo's Git tags; pull the ones you need.

| Crate | Public surface (top level) |
|---|---|
| [`auki-hash`](crates/auki-hash) | `hash_jcs_bytes(bytes) -> String` (XXH3-128) |
| [`auki-jcs`](crates/auki-jcs) | `canonicalize(value) -> Vec<u8>` (RFC 8785) |
| [`auki-identity`](crates/auki-identity) | `Wallet`, `PublicKey`, `WalletId`, `Signature`, `CreationCert`, `verify(...)`, `load_or_mint_seed(...)` |
| [`auki-logs`](crates/auki-logs) | `Log<T>`, `LogReader<T>`, `Entry<T>`, `Error` |
| [`auki-registry`](crates/auki-registry) | `SensorRegistryEntry` / `SensorBody` (`RgbCamera`, `PointCloud`, `Microphone`), `ClockRegistryEntry`, `FrameRegistryEntry`, `SensorLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`, `PoseLogEntry`, `PoseSource`, `TransformSample`, `write_sensor` / `read_sensor`, `write_clock` / `read_clock`, `write_frame` / `read_frame`, `build_sensor_log_manifest`, `build_pose_log_manifest` |
| [`auki-session`](crates/auki-session) | `registries_root`, `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `timetransform_log_path`, `sensorlog_path`, `poselog_path`, `id_to_segment` |
| [`auki-time-transforms`](crates/auki-time-transforms) | `Clock` (trait), `SystemClock`, `Sampler`, `SamplerState`, `tick(...)`, `build_manifest(...)`, `TimeTransformEntry`, `TimeTransformSource` |
| [`auki-network`](crates/auki-network) | `PeerIdentity`, `ParticipantInfo`, `ReachabilityRecord`, `Capability`, plus modules `cluster_doc`, `swarm`, `cluster_protocol`, `cluster_runtime`, `stream_protocol`, `stream_runtime`, `app_instance`, `discovery_client`. Constant `PEER_DERIVATION_LABEL = "peer/v1"` |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ROS2 message structs (`StampMsg`, `CameraInfoMsg`, `ImageMsg`, `PointCloud2Msg`, `PointFieldMsg`); builders (`build_rgb_camera_registry_entry`, `build_sensor_log_entry`, `build_point_cloud_registry_entry`, `build_point_cloud_log_entry`); `CameraSubscriber` / `PointCloudSubscriber` traits + mocks; `r2r_subscriber` module |

Each crate's own README documents the public types in detail and pins the on-disk format where applicable.

### 2. Python (PyO3) bindings

PyO3 wrappers shipped as separate crates, one per Rust component. The pattern is **per-component naming** (no umbrella `auki-py` package).

[`auki-identity-py`](crates/auki-identity-py) — wallet primitives + per-machine identity:

```python
import auki_identity
seed   = auki_identity.load_or_mint_seed(path)        # bytes
wallet = auki_identity.Wallet.from_seed(seed)
peer_id = wallet.derive_child("peer/v1").peer_id()    # libp2p PeerId string
mac_id  = auki_identity.app_instance.derive()         # MAC-derived per-machine ID
```

[`auki-network-py`](crates/auki-network-py) — peer cluster runtime, live streams, Discovery client. Two submodules:

| Submodule | Public surface |
|---|---|
| `auki_network.cluster` | `ParticipantInfo`, `PeerSnapshot`, `ClusterDoc`, `ClusterRuntime`, `StreamRequest`, `AcceptInfo`, `JpegFrame`, `PointCloudFrame`, `DeclineReason`, `EndReason`, `ProducerFrame`, `ConsumerFrame`, `StreamDecision`, `StreamSubscription`, `FrameIterator`, plus `load_doc(...)` and `spawn(...)` |
| `auki_network.discovery` | `DiscoveryClient` — `register / fetch / deregister` |

These are what consumer apps written in Python (e.g. BoosterApp's sidecar) import to participate as a peer, register live sensor streams, and join a discovery service — without reimplementing libp2p in Python.

### 3. HTTP control API (cross-app operator surface)

Daemons that produce SDK sessions (BoosterApp, Sentinel, future) implement a uniform HTTP control surface so any UI — primarily [Park](https://github.com/aukilabs/park) — can drive any of them through one contract. The SDK **specifies** the contract; consumer apps implement it.

Specified in [`docs/control-api.md`](docs/control-api.md). All endpoints under `/api/`, JSON over HTTP, daemons bind `0.0.0.0:<port>`, trusted-LAN assumption (no auth in v1).

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/info` | Session-scoped identity: `app`, `name`, `session_id`, `session_clock_id` + `session_clock_hash`, `session_now_ns`, `cluster_joined_at_ns`, `peer_id`, `app_instance`. Same payload as the libp2p `/auki/cluster/1.0.0` protocol. |
| `GET` | `/api/state` | `{session_uuid, recordings: [...]}` — every recording in the session, active or stopped, with full identity (`sensor_id` + `sensor_hash`, `clock_id` + `clock_hash`) and lifecycle fields (`started_at_ns`, `stopped_at_ns`, `duration_ns`, `frame_count`, `retention_ns`). |
| `GET` | `/api/registries/sensors/<sensor_id>/<sensor_hash>` | Hash-pinned Sensor Registry entry, served verbatim, immutable. |
| `GET` | `/api/registries/clocks/<clock_id>/<clock_hash>` | Hash-pinned Clock Registry entry, same semantics. |
| `GET` | `/api/preview/latest.jpg` | Latest captured frame as JPEG (poll-based; `503` if none yet). |
| `POST` | `/api/recordings` | Start an unbounded intent recording. Returns `201 {"recording_id": "..."}`. |
| `DELETE` | `/api/recordings/<id>` | Stop a recording — sets `stopped_at_ns`, freezes `duration_ns`, keeps the entry in `/api/state`. |
| `PATCH` | `/api/buffer` | `{"retention_ns": <i64>}` — change the auto-started buffer's retention at runtime. |
| `POST` | `/api/quit` | Clean shutdown — flushes logs, closes mDNS, exits. Responds `200` *before* teardown. |

Plus an **mDNS service-discovery convention** on `_auki._tcp.local.` with TXT records `name` and `app` so consumers find daemons on the LAN automatically. See [`docs/control-api.md`](docs/control-api.md) for the full conformance checklist.

### 4. libp2p wire protocols

For peer-to-peer participation. Not REST-shaped, but they are public protocols the SDK defines.

| Protocol ID | Purpose |
|---|---|
| `/auki/cluster/1.0.0` | Peer-to-peer `ParticipantInfo` exchange. Carries the same payload as HTTP `/api/info`. |
| `/auki/stream/1.0.0` | Live sensor streaming. Two payload kinds today: JPEG frames (RGB cameras) and CDR-encoded `PointCloud2` (stereo / depth point clouds). |

Both are exposed from Python via `ClusterRuntime.open_stream` / `ClusterRuntime.open_pointcloud_stream` in `auki-network-py`, and from Rust via the `stream_protocol` / `stream_runtime` modules in `auki-network`.

---

## Quickstart

Add the SDK crates as Git dependencies in your `Cargo.toml`:

```toml
[dependencies]
auki-logs     = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.5" }
auki-registry = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.5" }
auki-session  = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.5" }
```

Open a sensor log for one recording:

```rust
use std::path::Path;
use auki_logs::Log;
use auki_session::{session_root, sensorlog_path};

let app_root  = Path::new("/home/booster/auki/boosterapp");
let session   = session_root(app_root, "session-uuid");
let log_root  = sensorlog_path(&session, "recording-uuid");

let manifest = serde_json::json!({
    "segment_duration_ns": 1_000_000_000_i64,
    "retention_ns":        30_000_000_000_i64,
    "sensor_id":           "K1-AABBCCDDEEFF/head_left_cam",
    "sensor_hash":         "abc...",
    "clock_id":            "K1-AABBCCDDEEFF/utc",
    "clock_hash":          "def...",
});

let mut log: Log<MyPayload> = Log::open(&log_root, manifest)?;
log.append(timestamp_ns, &payload)?;
```

See each crate's `README.md` for the contract spec, and `crates/<name>/src/README.md` for the Rust-side implementation status.

---

## Cross-language conformance vectors

Critical wire-format and derivation chains are pinned by **locked test vectors** — exact-bytes/exact-strings the Rust crate produces from a fixed input. Any reimplementation in another language (Python, Go, browser JS) is correct only if it reproduces the same outputs from the same inputs. If a vector ever drifts, every consumer in the wild drifts with it; updates require a coordinated version bump.

| Crate | Test | What it locks |
|---|---|---|
| [`auki-hash`](crates/auki-hash) | `tests::locked_*` (existing) | XXH3-128 byte vectors (used for content-addressed registry hashes) |
| [`auki-identity`](crates/auki-identity) | `tests::locked_derive_child_peer_v1_pubkey_vector` | `Wallet::from_seed([3u8; 32]).derive_child("peer/v1").public_key()` → 32-byte ed25519 pubkey |
| [`auki-network`](crates/auki-network) | `tests::locked_seed_to_peer_id_vector` | `PeerIdentity::from_wallet(Wallet::from_seed([3u8; 32])).peer_id()` → canonical `12D3KooW…` libp2p PeerId string |

The two-stage chain in `auki-identity` + `auki-network` together pin the `Wallet → libp2p PeerId` derivation that `cluster.json` depends on. They use the same `[3u8; 32]` seed so the locked pubkey can be inspected as the intermediate value for the locked PeerId.

---

## Design and discussion

The SDK's design is being worked through across two spaces:

- **Long-form design docs** live in Notion under [About the real world web](https://www.notion.so/3495c8e9659280b38cc9ce1540f72a29). These cover the broader Auki protocol, the four questions, the registries and logs, and the Domain/Cluster/Map architecture.
- **Open architectural questions** live in the [SDK Parking lot](https://www.notion.so/3495c8e9659280f5b0eafc6e4f70898b) — registry vs. manifest, frame pairing, schema versioning, etc.

Notion is the source of truth for design conversations; this repo is the source of truth for the implementation.

---

## Status

The SDK is **0.x and changing**. Schema and API are pre-stable; breaking changes are expected and will tick the minor version pre-1.0. Stable points are tagged on Git (`v0.0.1`, `v0.0.2`, …); downstream consumers should pin a tag.

The first downstream consumers are:

- **`booster-app`** — the on-robot application running the SDK on a Booster K1 humanoid
- **`park`** — an open-source visualization tool for Auki sessions, eventually a real-time domain viewer

---

## Contributing

Issues, design questions, and contributions are welcome via [GitHub issues](https://github.com/aukilabs/auki-sdk/issues). Bigger architectural questions are best raised in the Notion [SDK Parking lot](https://www.notion.so/3495c8e9659280f5b0eafc6e4f70898b) so they're visible to the whole team.

---

## License

MIT. See [LICENSE](LICENSE).

Copyright © 2026 Auki Labs Limited.
