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
| [`auki-logs`](crates/auki-logs) | ✓ Generic segmented ring-buffer log primitive — manifest + segment files + retention eviction. Encoder-agnostic via the `LogPayload` trait (Step 1, 2026-05-08); consumers pick prost / ciborium / their own. |
| [`auki-registry`](crates/auki-registry) | ✓ Sensor + Clock + Frame registry types and IO; Point Cloud Log + Audio Log + Pose Log payload schemas (capture-only for Pose Log; `convert_pose` pending). Camera log payload (`PinholeCameraLogEntry` + `DynamicIntrinsics`) departed at Step 1 (2026-05-08) into [`auki-datatypes`](crates/auki-datatypes). Frame Registry shipped in v0.0.22 with four preset constructors (`ros_body` / `ros_optical` / `opengl` / `unity`) and `frame_id` references on `RgbCamera` + `PointCloud`. |
| [`auki-datatypes`](crates/auki-datatypes) | ✓ Single source of truth for shared cross-language segment payload shapes (`.proto` schemas + prost-generated Rust). Ships `auki.camera` (`PinholeCameraLogEntry` + `DynamicIntrinsics`, Step 1), `auki.point_cloud` (`PointCloudLogEntry`, opaque-bytes-only, Step 3), and the libp2p stream wire types `auki.frame_stream` (`JpegFrame`) + `auki.point_cloud_stream` (`PointCloudFrame`) + `auki.stream` envelope (Step 2). Three more on-disk packages migrate one step at a time per [`auki-datatypes/src/sprint.md`](crates/auki-datatypes/src/sprint.md). Encoding is protobuf via prost; the name names the responsibility. |
| [`auki-manifests`](crates/auki-manifests) | ✓ Single source of truth for log manifest shapes — JCS-canonical UTF-8 JSON via `auki-jcs`. `build_sensor_log_manifest` / `build_pose_log_manifest` / `build_time_transform_log_manifest` plus the inline `PoseSource` tagged enum. Symmetric with `auki-datatypes`: that crate owns segment payload shapes, this one owns per-recording manifest shapes. |
| [`auki-jcs`](crates/auki-jcs) | ✓ RFC 8785 JSON canonicalization (used for stable hashing of registry entries) |
| [`auki-hash`](crates/auki-hash) | ✓ XXH3-128 wrapper used for registry content-addressing |
| [`auki-time-transforms`](crates/auki-time-transforms) | ✓ Clock sampler primitives for the TimeTransform Log |
| [`auki-layout`](crates/auki-layout) | ✓ Path helpers for the on-disk session shape — single source of truth for app/session/recording layout (renamed from `auki-session` 2026-05-08; the old name now reserved for the future runtime `Session` abstraction) |
| [`auki-identity`](crates/auki-identity) | ✓ Wallet primitive: ed25519 keypairs, deterministic child derivation, signed creation certs. WASM-friendly |
| [`auki-identity-py`](crates/auki-identity-py) | ✓ PyO3 bindings for the identity primitives BoosterApp's Python sidecar consumes — `load_or_mint_seed`, `Wallet.from_seed/derive_child/peer_id/seed`, `app_instance.derive` |
| [`auki-network`](crates/auki-network) | ✓ libp2p substrate (TCP/QUIC, Noise, Yamux, Circuit Relay v2, mDNS, identify, ping) behind the `swarm` feature; peer identity from `Wallet::derive_child("peer/v1")`; `cluster.json` loader + opaque `ClusterRuntime` driving `/auki/cluster/0.0.1` (participant exchange) and `/auki/stream/0.1.0` (typed `Stream<T>` for `JpegFrame` and `PointCloudFrame`, prost-encoded `StreamMessage` envelope from `auki-datatypes` since Step 2 (2026-05-08), dispatched by `sensor_id` via the closed `StreamDispatch` enum). REST `discovery_client` (Vinland) for register/fetch/deregister against a Discovery server, behind the `discovery_client` feature. MAC-derived `app_instance` behind its own feature |
| [`auki-network-py`](crates/auki-network-py) | ✓ PyO3 wrapper around `ClusterRuntime` + `Stream<T>` + `discovery_client`. `cluster.spawn(stream_provider=...)` accepting Python `async def` generators (Pattern A — SDK owns the asyncio loop on its tokio worker; sidecars stay sync-shaped); `runtime.open_stream(peer_id, sensor_id)` (JPEG) + `runtime.open_pointcloud_stream(peer_id, sensor_id)` (CDR `PointCloud2` bytes); `auki_network.discovery.DiscoveryClient` with sync `register` / `fetch` / `deregister` |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ⚠ Generic ROS2 → SDK glue: `CameraInfo`/`Image` and `PointCloud2` translation, with RGB/RGBA normalization for point clouds. `frame_id` threaded through both builders for v0.0.22's Frame Registry rollout. Currently broken at the transport layer: `r2r` 0.9.5's compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout some camera drivers publish. Fix in flight |

**Not yet implemented:**

- `convert_pose` (the Pose Log primitives — `PoseLogEntry` + `TransformSample` in [`auki-registry`](crates/auki-registry), `PoseSource` + `build_pose_log_manifest` in [`auki-manifests`](crates/auki-manifests), `poselog_path` in [`auki-layout`](crates/auki-layout) — are in place for capture and read; the `convert_pose` operation that composes pose paths is pending)
- Detection Log
- `convert_time` (the TimeTransform Log primitives exist; the `convert_time` operation that consumes them does not yet)
- A `Session` abstraction tying clock + sensor-id minting + recording lifecycle together (today daemons construct sessions by convention)
- Layer 2 capability advertisement (`/auki/capabilities/1.0.0`) — querying what topics a peer offers at runtime, and cross-peer Sensor / Clock / Frame Registry sync. Today registries are local-disk only; cross-peer convention agreement is by configuration

---

## On-disk format

Logs and registries write to a documented binary + JSON format. Each format spec lives with the crate that owns it — they're the source of truth for any reader, in any language:

- [`auki-logs`](crates/auki-logs/README.md) — segmented ring-buffer log layout (used by both Sensor and TimeTransform Logs)
- [`auki-registry`](crates/auki-registry/README.md) — registry entry storage layout (Sensor / Clock / Frame); Point Cloud + Audio + Pose Log payload schemas (mid-migration)
- [`auki-datatypes`](crates/auki-datatypes/README.md) — Sensor Log payload schemas for Pinhole cameras and Point Clouds, post-migration; the `.proto` files are the cross-language contract
- [`auki-manifests`](crates/auki-manifests/README.md) — Sensor / Pose / TimeTransform Log manifest shapes (JCS-JSON)
- [`auki-time-transforms`](crates/auki-time-transforms/README.md) — TimeTransform Log payload schema and sampling protocol
- [`auki-layout`](crates/auki-layout/README.md) — the path layout and helpers below

Files within an app:

```
<app_root>/
├── registries/
│   ├── sensors/<sensor_id>/<hash>.json   ← shared across all sessions of this app
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json
└── <session_id>/
    ├── timetransform_logs/<from_id>__<to_id>/
    │   ├── manifest.json
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    ├── sensorlogs/
    │   ├── <sensor_log_id_1>/             ← one sensor stream per log
    │   │   ├── manifest.json
    │   │   └── segments/<padded-ns>.seg
    │   ├── <sensor_log_id_2>/
    │   │   └── ...
    │   └── <sensor_log_id_3>/
    └── poselogs/
        ├── <pose_log_id_1>/               ← one pose source per log
        │   ├── manifest.json
        │   └── segments/<padded-ns>.seg
        └── <pose_log_id_2>/
```

`<app_root>` is chosen by the integrator (boosterapp uses `/home/booster/auki/boosterapp/`); the SDK doesn't enforce structure above the registries. Registries live at the app root because hash-keyed writes are idempotent — a sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session, so per-session copies would be wasted work.

**A sensor log is one sensor stream.** Each `<sensor_log_id>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor. Multi-sensor capture means multiple parallel sensor logs sharing a session, not a multi-sensor log. Buffers, intent recordings, and time-bounded captures are all sensor logs — they differ only in their `retention_ns` (backward window kept on disk; `0` = no eviction) and `duration_ns` (forward auto-stop cap; `0` = run indefinitely). The sensor identity lives in the log's manifest (`sensor_id` + `sensor_hash`), not in the path. Whether a daemon auto-starts any sensor log at session boot is daemon-application policy, not SDK contract.

The on-disk shape is pre-1.0 and changes accumulate by tag — see [`changelog.md`](changelog.md) for the per-tag history. Most recent shape changes: v0.0.22 added `frames/<frame_id>/<hash>.json` with the Frame Registry and required `frame_id` on `RgbCamera` + `PointCloud` registry entries (breaking; pre-1.0, integrators regenerate); v0.0.6 removed the inner `<sensor_id>/` layer under `sensorlogs/`.

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
| [`auki-logs`](crates/auki-logs) | `Log<T>`, `LogReader<T>`, `Entry<T>`, `Error`, `LogPayload` (trait — consumers pick the encoder; prost types in `auki-datatypes` get a blanket impl) |
| [`auki-registry`](crates/auki-registry) | `SensorRegistryEntry` / `SensorBody` (`RgbCamera`, `PointCloud`, `Microphone`), `ClockRegistryEntry`, `FrameRegistryEntry`, `AudioLogEntry`, `PoseLogEntry`, `TransformSample`, `write_sensor` / `read_sensor`, `write_clock` / `read_clock`, `write_frame` / `read_frame`. Camera log payload (`PinholeCameraLogEntry` + `DynamicIntrinsics`) moved to [`auki-datatypes`](crates/auki-datatypes) at Step 1 (2026-05-08); `PointCloudLogEntry` moved at Step 3 (2026-05-08, opaque-bytes-only); the remaining log payload types continue to depart per the migration sprint. |
| [`auki-datatypes`](crates/auki-datatypes) | `camera::PinholeCameraLogEntry`, `camera::DynamicIntrinsics`, `point_cloud::PointCloudLogEntry`, `frame_stream::JpegFrame`, `point_cloud_stream::PointCloudFrame`, `stream::{StreamMessage, StreamRequest, AcceptInfo, Frame, DeclineReason, EndReason}` + helper constructors, `placeholder::PipelineCheck` (departs at Step 7). Every on-disk prost type satisfies `auki_logs::LogPayload` via the in-crate `impl_log_payload!` macro. |
| [`auki-manifests`](crates/auki-manifests) | `build_sensor_log_manifest`, `build_pose_log_manifest`, `build_time_transform_log_manifest`, `PoseSource`. Single owner of the SDK's per-recording manifest schemas + builders; symmetric with `auki-datatypes` (segment payloads). Manifest encoding is JCS-JSON. |
| [`auki-layout`](crates/auki-layout) | `registries_root`, `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `timetransform_log_path`, `sensorlog_path`, `poselog_path`, `id_to_segment` |
| [`auki-time-transforms`](crates/auki-time-transforms) | `Clock` (trait), `SystemClock`, `Sampler`, `SamplerState`, `tick(...)`, `TimeTransformEntry`, `TimeTransformSource` |
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
| `GET` | `/api/info` | Session-scoped identity: `app`, `name`, `session_id`, `session_clock_id` + `session_clock_hash`, `session_now_ns`, `cluster_joined_at_ns`, `peer_id`, `app_instance`. Same payload as the libp2p `/auki/cluster/0.0.1` protocol. |
| `GET` | `/api/sensor_logs` | List sensor logs across every session on disk. Filters: `session_id` (`<uuid>` or `current`), `sensor_id`, `sensor_hash`, `clock_id`, `started_after`, `started_before` (compose as AND). Each entry: `sensor_log_id`, `session_id`, `sensor_id` + `sensor_hash`, `clock_id` + `clock_hash`, `retention_ns`, `duration_ns`, `started_at_ns`, `stopped_at_ns` (`null` only for live logs in the live session). |
| `GET` | `/api/registries/sensors/<sensor_id>/<sensor_hash>` | Hash-pinned Sensor Registry entry, served verbatim, immutable. |
| `GET` | `/api/registries/clocks/<clock_id>/<clock_hash>` | Hash-pinned Clock Registry entry, same semantics. |
| `GET` | `/api/preview/latest.jpg` | Latest captured frame as JPEG (poll-based; `503` if none yet). |
| `POST` | `/api/sensor_logs` | Open a sensor log in the live session. Body: `{sensor_id, sensor_hash, retention_ns, duration_ns}`. Returns `201 {"sensor_log_id": "..."}`. `409` on `sensor_hash` mismatch with the live binding. |
| `PATCH` | `/api/sensor_logs/<id>` | Mutate `retention_ns` and/or `duration_ns` on a live log; identity fields are immutable. PATCH on a stopped or historical log is `404`. |
| `DELETE` | `/api/sensor_logs/<id>` | Stop a live log — sets `stopped_at_ns`, keeps the entry listed. DELETE on a stopped or historical log is `404`. |
| `POST` | `/api/quit` | Clean shutdown — flushes logs, closes mDNS, exits. Responds `200` *before* teardown. |

Plus an **mDNS service-discovery convention** on `_auki._tcp.local.` with TXT records `name` and `app` so consumers find daemons on the LAN automatically. See [`docs/control-api.md`](docs/control-api.md) for the full conformance checklist.

### 4. libp2p wire protocols

For peer-to-peer participation. Not REST-shaped, but they are public protocols the SDK defines.

| Protocol ID | Purpose |
|---|---|
| `/auki/cluster/0.0.1` | Peer-to-peer `ParticipantInfo` exchange. Carries the same payload as HTTP `/api/info`. |
| `/auki/stream/0.1.0` | Live sensor streaming. Prost-encoded `StreamMessage` envelope (`auki.stream` package in [`auki-datatypes`](crates/auki-datatypes); Step 2 of the migration, 2026-05-08). Two payload kinds today: JPEG frames (RGB cameras) and CDR-encoded `PointCloud2` (stereo / depth point clouds). |

Both are exposed from Python via `ClusterRuntime.open_stream` / `ClusterRuntime.open_pointcloud_stream` in `auki-network-py`, and from Rust via the `stream_protocol` / `stream_runtime` modules in `auki-network`.

---

## Networking — clusters, streams, discovery

The SDK ships a libp2p substrate behind the `auki-network` `swarm` feature. A daemon (Booster, Sentinel, Park) becomes a peer in a **cluster** — the runtime group of devices networking around a shared Domain ID — by reading a `cluster.json` discovery doc, exchanging `ParticipantInfo` over `/auki/cluster/0.0.1`, and (optionally) opening typed streams over `/auki/stream/0.1.0`.

Three protocols ship today:

| Protocol | Purpose | What it carries |
|---|---|---|
| `/auki/cluster/0.0.1` | Participant exchange | `ParticipantInfo` — `app`, `name`, `session_id`, session-clock binding, `peer_id`, `app_instance`. JSON-on-wire (length-framed by libp2p's `request_response::json` codec). |
| `/auki/stream/0.1.0` | Typed sensor data streaming | Prost-encoded `StreamMessage` envelope (`auki.stream` package in [`auki-datatypes`](crates/auki-datatypes); Step 2 of the migration, 2026-05-08) carrying `Frame { timestamp_ns, seq, payload: bytes }`. Each substream is mono-`T`; `T` is inferred from the `AcceptInfo.sensor_hash` handshake. Today: `T = JpegFrame` (grimsby v1 — byte-identical to `GET /api/preview/latest.jpg`) and `T = PointCloudFrame` (Dagaz Batch 1 — raw CDR-encoded ROS `PointCloud2` bytes). The producer's `stream_provider` callback dispatches by `sensor_id` to pick which `T` per request via the closed `StreamDispatch { AcceptJpeg, AcceptPointCloud, Decline }` enum |
| Discovery REST (Vinland) | Multi-cluster registry | Wallet-signed `register` / `fetch` / `deregister` against [`aukilabs/discovery`](https://github.com/aukilabs/discovery). `auki-network::discovery_client` is the SDK-side client, behind the `discovery_client` feature; daemons branch at startup on `--discovery-url` (no fallback chain — D3) |

Python sidecars (BoosterApp's K1 sensor capture) get the same surface via [`auki-network-py`](crates/auki-network-py) — `cluster.spawn(stream_provider=...)` accepts an `async def` generator, `runtime.open_stream` / `runtime.open_pointcloud_stream` are sync-blocking. Pattern A: SDK owns the asyncio loop on its tokio worker; sidecars stay sync-shaped.

---

## Quickstart

Add the SDK crates as Git dependencies in your `Cargo.toml`. Pin a tag — the SDK is pre-1.0 and breaking changes tick the patch version:

```toml
[dependencies]
auki-logs     = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.22" }
auki-registry = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.22" }
auki-session  = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.22" }
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
| [`auki-identity`](crates/auki-identity) | `tests::locked_sign_canonical_json_vector` | `Wallet::from_seed([3u8; 32]).sign_canonical_json(<Vinland-shaped registration>)` → exact RFC 8785 canonical bytes + 64-byte ed25519 signature |
| [`auki-network`](crates/auki-network) | `tests::locked_seed_to_peer_id_vector` | `PeerIdentity::from_wallet(Wallet::from_seed([3u8; 32])).peer_id()` → canonical `12D3KooW…` libp2p PeerId string |
| [`auki-network`](crates/auki-network) | `stream_protocol::tests::jpeg_frame_serializes_to_locked_wire_bytes` | `JpegFrame { bytes: <10-byte JFIF prefix> }` → exact prost wire bytes (`0a0a` tag/length prefix + payload) |
| [`auki-network`](crates/auki-network) | `stream_protocol::tests::point_cloud_frame_serializes_to_locked_wire_bytes` | `PointCloudFrame { bytes: <8-byte fixture> }` → exact prost wire bytes (same shape as `JpegFrame` — single `bytes` field, separate `.proto` package for independent evolution) |
| [`auki-network`](crates/auki-network) | `stream_protocol::tests::locked_stream_message_frame_with_point_cloud_payload` | Full envelope: `StreamMessage::Frame { timestamp_ns, seq, payload: <prost-encoded PointCloudFrame> }` → exact prost-encoded bytes. Park's browser-side decoder + cross-language reimplementations pin against this. |
| [`auki-network`](crates/auki-network) | `discovery_client::tests::locked_register_canonical_and_signature_vector` | Vinland Discovery `register` signed payload → exact RFC 8785 canonical bytes + 64-byte signature; pairs with Discovery's verifier-side reproduction |
| [`auki-registry`](crates/auki-registry) | `tests::frame_entry_serializes_to_canonical_bytes_matching_locked_vector` | `FrameRegistryEntry::ros_body("K1-AABBCCDDEEFF/base_link")` → `{"axes":{"x":"forward","y":"left","z":"up"},...}` JSON + XXH3-128 `fd0dc3789e898b71b5e16ee122a81a44` |

The two-stage chain in `auki-identity` + `auki-network` pins the `Wallet → libp2p PeerId` derivation that `cluster.json` depends on. The Vinland and PointCloudFrame vectors pin the wire shapes that cross language boundaries to Discovery's verifier and Park's browser-side parser. The Frame Registry vector pins the convention names so any cross-language reader (mobile companion, future Sentinel-as-consumer) parses `"forward"` / `"left"` / `"up"` byte-identically.

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
