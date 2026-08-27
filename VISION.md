# Auki SDK

The on-device SDK for the Auki **real world web** — a collaborative spatial computing protocol that lets devices, robots, and services share, locate, and reason about the physical world.

> *The internet made information browsable. The Auki network makes the physical world browsable.*

---

## What this SDK is for

Each physical space — a warehouse, a hospital ward, a retail floor — gets a **Domain**: a unique tag applied to data, asserting that the data is *about* that space. Domains are to the real world what websites are to the internet — privately owned, independently operated, linked by a shared protocol. A Domain has one or more **scenegraphs** — structured representations of typed nodes (frames, sensors, clocks) connected by transform edges. The Domain Owner designates one as the canonical **Map**, served by default when a peer asks for "the map" without naming a scenegraph. See [GLOSSARY.md](GLOSSARY.md) for the full term list, including the distinction between Domain ID, Scenegraph ID, and Session ID.

The Auki protocol is built around five questions any node — a phone, a robot, a cloud server, a browser tab — should be able to answer about any other node:

- **Identity** — Who am I?
- **Spatial** — Where did this happen?
- **Temporal** — When did this happen?
- **Networking** — How do I talk to you?
- **Tokenomics** — How do I compensate you?

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

## Application-controlled processing components

Detectors and Mappers are executable components, not network Resources. A
registry entry describes an implementation and its compatible contracts; a
runner executes one instance; the application chooses the concrete inputs,
output, cadence, lifetime, and resource budget. The resulting Detection Log or
Map Log is the Resource other peers discover and consume.

This separation keeps policy out of implementations. A QR detector does not
choose a camera, and a voxel Mapper does not subscribe merely because a point
cloud exists. An application may run the same registered implementation many
times against different streams or not run it at all. SDK discovery helpers may
validate and propose unambiguous bindings, but source selection remains an
application decision.

Execution follows two explicit modes:

- **Live:** preserve freshness and control-plane responsiveness. Ingestion is
  asynchronous, CPU-heavy component work runs outside the async runtime, and
  bounded queues apply an observable overload policy. Image and spatial mapping
  runners normally use latest-frame-wins because stale work has declining value.
- **Replay:** preserve the recording. Samples are processed in timestamp order,
  exhaustively unless the caller cancels or a deterministic error terminates the
  run. Replay never silently adopts live dropping semantics.

SDK runners own validation, timestamps, provenance, scheduling, backpressure,
cancellation, and operational counters. Bring-your-own implementations own the
domain computation. Output sinks must also avoid blocking the async control
plane; an async method is not sufficient if it performs expensive synchronous
work before returning its future.

The detailed contract and integration checklist live in the
[Component Execution wiki page](docs/wiki/Component-Execution.md).

---

## What's implemented today

This repo is in early development. The crates here implement a foundational subset of the architecture:

| Crate | Status |
|---|---|
| [`auki-logs`](crates/auki-logs) | ✓ Generic segmented ring-buffer log primitive — manifest + segment files + retention eviction. Encoder-agnostic via the `LogPayload` trait (Step 1, 2026-05-08); consumers pick prost / ciborium / their own. |
| [`auki-registry`](crates/auki-registry) | ✓ Sensor + Clock + Frame registry types and IO. All log payload types departed at Steps 1, 3, 4, and 5 of the `auki-datatypes` migration on 2026-05-08; the crate's scope shrunk back to identity catalogs only. Frame Registry shipped in v0.0.22 with four preset constructors (`ros_body` / `ros_optical` / `opengl` / `unity`); spatial sensor bodies (`Camera` + `PointCloud`) now pin exact `frame_id` + `frame_hash` references. |
| [`auki-datatypes`](crates/auki-datatypes) | ✓ Single source of truth for shared cross-language segment payload shapes (`.proto` schemas + prost-generated Rust). Ships on-disk payloads for camera, point cloud, audio, joint encoders, pose, time transforms, and detection logs, plus stream payloads for camera, point cloud, joint encoders, audio, pose `SpatialTransform`, and the `/auki/auth/1/stream/0.2.0` envelope. Encoding is protobuf via prost; the name names the responsibility. |
| [`auki-geometry`](crates/auki-geometry) | ✓ Pure spatial math. Phase 1 ships convention conversion for points, length-bearing vectors, unitless directions, and `SpatialTransform` poses via `convert_pose_convention` — the convention-only layer underneath future full `convert_pose`. No registry IO, log IO, or networking. |
| [`auki-manifests`](crates/auki-manifests) | ✓ Single source of truth for log manifest shapes — JCS-canonical UTF-8 JSON via `auki-jcs`. Builders for sensor, pose, time-transform, and detection logs plus `PoseSource`, `PoseWriterMode`, and `TimeTransformSource`. Symmetric with `auki-datatypes`: that crate owns segment payload shapes, this one owns per-recording manifest shapes. |
| [`auki-jcs`](crates/auki-jcs) | ✓ RFC 8785 JSON canonicalization (used for stable hashing of registry entries) |
| [`auki-hash`](crates/auki-hash) | ✓ XXH3-128 wrapper used for registry content-addressing |
| [`auki-time`](crates/auki-time) | ✓ `SessionClock`, fixed affine `TimeTransform` math, clock traits, and local sampling primitives. It owns no network-derived Domain clock. |
| [`auki-layout`](crates/auki-layout) | ✓ Path helpers for the on-disk session shape — single source of truth for app/session/recording layout, including sensor, pose, time-transform, and detection log paths. |
| [`auki-identity`](crates/auki-identity) | ✓ Wallet primitive: ed25519 keypairs, deterministic child derivation, signed creation certs. WASM-friendly |
| [`auki-identity-py`](bindings/python/auki-identity-py) | ✓ PyO3 bindings for the identity primitives BoosterApp's Python sidecar consumes — `load_or_mint_seed`, `Wallet.from_seed/derive_child/peer_id/seed`, `app_instance.derive` |
| [`auki-registry-py`](bindings/python/auki-registry-py) | ✓ PyO3 bindings for Python producers to declare and persist Sensor / Clock / Frame Registry entries. Dict constructors, canonical JSON/hash helpers, and hash-pinned `write_*` / `read_*` helpers mirror `auki-registry`, including exact `frame_id` + `frame_hash` validation for spatial sensors. |
| [`auki-p2p`](crates/auki-p2p) | ✓ Canonical stable identity and the single native authenticated libp2p node: DDS Domain credentials, mutual authentication, explicit direct/relay routes, relay reservations, and peer observations. |
| [`auki-protocols`](crates/auki-protocols) | ✓ Exact `/auki/auth/1/...` IDs, bounded codecs, validation, locked vectors, and transport-neutral wire types. Protocol families are compile-time opt-in; the crate owns no runtime. |
| [`auki-domain`](crates/auki-domain) | ✓ Public owner for one Peer/Session in one exact DDS Domain UUID. `DomainBuilder` accepts host-supplied authority, listeners, routes, and exact-version `ServedProtocols`; the default serves none. `Domain` exposes authenticated peers, catalogs, registries, blobs, messages, streams, and ordered leave. |
| [`auki-domain-relay`](crates/auki-domain-relay) | WIP standalone native/WebSocket Circuit Relay v2 server. Domain authority and route distribution remain outside the relay. |
| `auki-network-py` | Removed Manager-era networking binding; superseded by `auki-domain-py`. |
| [`auki-domain-py`](bindings/python/auki-domain-py) | ✓ Authenticated Python facade over the same bounded `Domain` owner as native Rust. |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ⚠ Generic ROS2 → SDK glue: `CameraInfo`/`Image` and `PointCloud2` translation, with RGB/RGBA normalization for point clouds. `frame_id` + `frame_hash` thread through both builders so sensor entries commit to an exact Frame Registry version. Currently broken at the transport layer: `r2r` 0.9.5's compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout some camera drivers publish. Fix in flight |

**Not yet implemented:**

- full `convert_pose` (the Pose Log primitives — `SpatialTransform` + `Vec3` + `Quat` in [`auki-datatypes`](crates/auki-datatypes), `PoseSource` + `PoseWriterMode` + `build_pose_log_manifest` in [`auki-manifests`](crates/auki-manifests), `poselog_path` in [`auki-layout`](crates/auki-layout), and the convention-only `convert_pose_convention` layer in [`auki-geometry`](crates/auki-geometry) — are in place; the operation that composes pose-log paths is pending)
- `convert_time` (the TimeTransform Log primitives exist; the `convert_time` operation that consumes them does not yet)
- `Session::materialize_remote_log` (Phase 5 of #216). The catalog row, stream protocol, and `LogRef` plumbing are in place; the materialization layer that writes a local replica with its own retention policy is not.

---

## On-disk format

Logs and registries write to a documented binary + JSON format. Each format spec lives with the crate that owns it — they're the source of truth for any reader, in any language:

- [`auki-logs`](crates/auki-logs/README.md) — segmented ring-buffer log layout (used by both Sensor and TimeTransform Logs)
- [`auki-registry`](crates/auki-registry/README.md) — registry entry storage layout (Sensor / Clock / Frame); identity catalogs only post-migration
- [`auki-datatypes`](crates/auki-datatypes/README.md) — segment payload schemas for every on-disk log type (Pinhole cameras, Point Clouds, Audio, Joint Encoders, Pose, TimeTransform, Detection); the `.proto` files are the cross-language contract
- [`auki-manifests`](crates/auki-manifests/README.md) — Sensor / Pose / TimeTransform / Detection Log manifest shapes (JCS-JSON)
- [`auki-time`](crates/auki-time/README.md) — TimeTransform Log sampling protocol; segment payload schema lives in [`auki-datatypes`](crates/auki-datatypes) post-Step-6
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
    │   ├── log_manifest.json
    │   └── segments/<padded-ns>.seg      ← one TT log per session
    ├── sensorlogs/
    │   ├── <sensor_log_id_1>/             ← one sensor stream per log
    │   │   ├── log_manifest.json
    │   │   └── segments/<padded-ns>.seg
    │   ├── <sensor_log_id_2>/
    │   │   └── ...
    │   └── <sensor_log_id_3>/
    ├── poselogs/
    │   ├── <from_id>__<to_id>/            ← one (from_frame_id, to_frame_id) pair per log
    │   │   ├── log_manifest.json
    │   │   └── segments/<padded-ns>.seg
    │   └── <from_id_2>__<to_id_2>/
    └── detection_logs/
        └── <detector_id>__<input_log_id>/  ← slashes in detector_id become "__"
            ├── log_manifest.json
            └── segments/<padded-ns>.seg
```

`<app_root>` is chosen by the integrator (boosterapp uses `/home/booster/auki/boosterapp/`); the SDK doesn't enforce structure above the registries. Registries live at the app root because hash-keyed writes are idempotent — a sensor that doesn't change between app starts produces the same `<hash>.json` regardless of session, so per-session copies would be wasted work.

**A sensor log is one sensor stream.** Each `<sensor_log_id>/` directory is a complete `auki-logs` log (manifest + segments) for exactly one sensor. Multi-sensor capture means multiple parallel sensor logs sharing a session, not a multi-sensor log. Buffers, intent recordings, and time-bounded captures are all sensor logs — they differ only in their `retention_ns` (backward window kept on disk; `0` = no eviction) and `duration_ns` (forward auto-stop cap; `0` = run indefinitely). The sensor identity lives in the log's manifest (`sensor_id` + `sensor_hash`), not in the path. **A pose log is one ordered frame pair.** Each `<from_id>__<to_id>/` directory holds samples for exactly one `(from_frame_id, to_frame_id)` pair — same shape as TimeTransform Logs key per ordered clock pair. Multi-pair capture (a ROS `TFMessage`) fans into N parallel pose logs.

The on-disk shape is pre-1.0 and changes accumulate by tag — see the [wiki Release History](https://github.com/aukilabs/auki-sdk/wiki/Release-History) for the per-tag history. Recent shape changes include Frame Registry entries, pose logs keyed per `(from, to)` frame pair, detection log paths keyed by `(detector_id, input_log_id)`, and protobuf-owned payload types in `auki-datatypes`.

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
| [`auki-registry`](crates/auki-registry) | `SensorRegistryEntry` / `SensorBody` (`Camera`, `PointCloud`, `JointEncoders`, `Audio`), `ClockRegistryEntry`, `FrameRegistryEntry`, `write_sensor` / `read_sensor`, `write_clock` / `read_clock`, `write_frame` / `read_frame`. The crate's scope is identity catalogs only; log payloads live in [`auki-datatypes`](crates/auki-datatypes). |
| [`auki-datatypes`](crates/auki-datatypes) | `camera::CameraFrame`, `camera::DynamicIntrinsics`, `point_cloud::Data`, `audio::Data`, `joint_encoders::Data`, `detection::DetectionFrame`, `pose::{SpatialTransform, Vec3, Quat}`, `time_transform::TimeTransformEntry`, and `stream::{StreamMessage, StreamRequest, StreamManifest, StreamEntry, DeclineReason, EndReason}` helpers. Each opaque-bytes / structured-vector module ships a single `Data` message used on both disk (Sensor Log segment) and wire (libp2p `/auki/auth/1/stream/0.2.0` substream); the dual `*_stream` packages were collapsed in #176. |
| [`auki-geometry`](crates/auki-geometry) | `convert_pose_convention`, `convert_point_convention`, `convert_vector_convention`, `convert_direction_convention`, `axis_convention_matrix`, `convention_matrix`, `meters_per_unit`. Pure math over `auki-registry` declarations + `auki-datatypes` pose types. |
| [`auki-manifests`](crates/auki-manifests) | `build_sensor_log_manifest`, `build_pose_log_manifest`, `build_time_transform_log_manifest`, `build_detection_log_manifest`, `PoseSource`, `PoseWriterMode`, `TimeTransformSource`. Single owner of the SDK's per-recording manifest schemas + builders; symmetric with `auki-datatypes` (segment payloads). Manifest encoding is JCS-JSON. |
| [`auki-layout`](crates/auki-layout) | `registries_root`, `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `timetransform_log_path`, `sensorlog_path`, `poselog_path`, `detection_log_path`, `id_to_segment` |
| [`auki-time`](crates/auki-time) | `SessionClock`, pure affine `TimeTransform` math, `Clock` (trait), `SystemClock`, `Sampler`, `tick(...)`, plus re-exports `TimeTransformEntry` (from `auki-datatypes`) and `TimeTransformSource` (from `auki-manifests`). It owns no heartbeat synchronization or Domain clock. |
| [`auki-p2p`](crates/auki-p2p) | `Identity`, `Node`, `DomainAuthority`, `DdsVerificationKeys`, `SignedP2pCredential`, authenticated protocol and route types, relay reservations, and `NodeObservations`. |
| [`auki-protocols`](crates/auki-protocols) | Exact protocol IDs and versioned modules such as `catalog::v2`, `catalog::v3`, `catalog::v4`, and `stream::v2`; bounded framing and validation; no transport or hosting lifecycle. |
| [`auki-domain`](crates/auki-domain) | `DomainBuilder`, `DomainConfig`, `Domain`, `ServedProtocols`, authority/routes/status/known-peer handles, `ResourceCatalogProvider`, catalog/registry/blob/message/stream operations, and bounded ordered leave. |
| [`auki-domain-relay`](crates/auki-domain-relay) | `DomainRelay`, `DomainRelayConfig`, `DomainRelayEvent`, `DomainRelayError`. |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ROS2 message structs (`StampMsg`, `CameraInfoMsg`, `ImageMsg`, `PointCloud2Msg`, `PointFieldMsg`); builders (`build_camera_registry_entry`, `build_sensor_log_entry`, `build_point_cloud_registry_entry`, `build_point_cloud_log_entry`); `CameraSubscriber` / `PointCloudSubscriber` traits + mocks; `r2r_subscriber` module |

Each crate's own README documents the public types in detail and pins the on-disk format where applicable.

### 2. Python (PyO3) bindings

PyO3 wrappers shipped as separate crates, one per Rust component. The pattern is **per-component naming** (no umbrella `auki-py` package).

[`auki-identity-py`](bindings/python/auki-identity-py) — wallet primitives + per-machine identity:

```python
import auki_identity
import auki_domain
seed   = auki_identity.load_or_mint_seed(path)        # bytes
wallet = auki_identity.Wallet.from_seed(seed)
peer = wallet.derive_child("peer/v1")
identity = auki_domain.Identity.from_ed25519_seed(peer.seed())
peer_id = identity.peer_id                            # libp2p PeerId string
mac_id  = auki_identity.app_instance.derive()         # MAC-derived per-machine ID
```

The Manager-era `auki-network-py` package is removed. `auki-domain-py` accepts
host-fetched signed authority and explicit routes and owns the same bounded,
authenticated `Domain` lifecycle as Rust.

### 3. HTTP control API (cross-app operator surface)

Daemons that produce SDK sessions (BoosterApp, Sentinel, future) implement a uniform HTTP control surface so any UI — primarily [Park](https://github.com/aukilabs/park) — can drive any of them through one contract. The SDK **specifies** the contract; consumer apps implement it.

Specified in [`docs/control-api.md`](docs/control-api.md). All endpoints under `/api/`, JSON over HTTP, daemons bind `0.0.0.0:<port>`, trusted-LAN assumption (no auth in v1).

Identity is **not** part of this surface. Participant diagnostics travel only
over the mutually authenticated `/auki/auth/1/info/1.0.0` protocol. Any
app-local HTTP identity endpoint is operator-facing/debug, not contract.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/sensor_logs` | List sensor logs across every session on disk. Filters: `session_id` (`<uuid>` or `current`), `sensor_id`, `sensor_hash`, `clock_id`, `started_after`, `started_before` (compose as AND). Each entry: `sensor_log_id`, `session_id`, `sensor_id` + `sensor_hash`, `clock_id` + `clock_hash`, `retention_ns`, `duration_ns`, `started_at_ns`, `stopped_at_ns` (`null` only for live logs in the live session). |
| `GET` | `/api/registries/sensors/<sensor_id>/<sensor_hash>` | Hash-pinned Sensor Registry entry, served verbatim, immutable. |
| `GET` | `/api/registries/clocks/<clock_id>/<clock_hash>` | Hash-pinned Clock Registry entry, same semantics. |
| `GET` | `/api/preview/latest.jpg` | Latest captured camera frame as JPEG (poll-based; `503` if none yet). |
| `POST` | `/api/sensor_logs` | Open a sensor log in the live session. Body: `{sensor_id, sensor_hash, retention_ns, duration_ns}`. Returns `201 {"sensor_log_id": "..."}`. `409` on `sensor_hash` mismatch with the live binding. |
| `PATCH` | `/api/sensor_logs/<id>` | Mutate `retention_ns` and/or `duration_ns` on a live log; identity fields are immutable. PATCH on a stopped or historical log is `404`. |
| `DELETE` | `/api/sensor_logs/<id>` | Stop a live log — sets `stopped_at_ns`, keeps the entry listed. DELETE on a stopped or historical log is `404`. |
| `POST` | `/api/quit` | Clean shutdown — flushes logs, closes mDNS, exits. Responds `200` *before* teardown. |

Plus an **mDNS service-discovery convention** on `_auki._tcp.local.` with TXT records `name` and `app` so consumers find daemons on the LAN automatically. See [`docs/control-api.md`](docs/control-api.md) for the full conformance checklist.

### 4. libp2p wire protocols

For peer-to-peer participation. Not REST-shaped, but they are public protocols the SDK defines.

| Protocol ID | Purpose |
|---|---|
| `/auki/auth/1/info/1.0.0` | Bounded diagnostic participant metadata after mutual authentication. |
| `/auki/auth/1/resources/0.2.0` | Resource catalog rows with explicit source/writer identity. |
| `/auki/auth/1/resources/0.3.0` | v0.2 rows plus receiver-owned message channels. |
| `/auki/auth/1/resources/0.4.0` | Map Log catalog rows. |
| `/auki/auth/1/registries/0.2.0` | Authenticated Get-only registry compatibility payload. |
| `/auki/auth/1/registries/0.3.0` | Tagged Get plus bounded Device Model List. |
| `/auki/auth/1/blobs/0.1.0` | Verified content-addressed binary blob transfer. |
| `/auki/auth/1/message/0.1.0` | Receiver-owned live message channels with sequence ACKs. |
| `/auki/auth/1/stream/0.2.0` | Typed live streams bound to the expected authenticated producer. |

`auki-protocols` owns these exact wire contracts. Compiling a family does not
serve it: each `Domain` starts with `ServedProtocols::none()` and the host opts
in to every exact inbound version it implements. Client operations remain
available independently.

Rust and Python consumers use retained operations on the same native
`auki_domain::Domain` owner.

---

## Networking — authenticated Domains and explicit routes

A native daemon joins one exact DDS Domain UUID through `DomainBuilder`. The
host supplies a stable identity, signed Domain credential, verification keys,
listeners, and explicit route candidates. `Domain` owns exactly one
`auki-p2p` node and only the application protocol tasks selected through
`ServedProtocols`. Routes describe reachability only; the signed token plus
Noise Peer ID is the authority.

The main live paths are:

| Protocol | Purpose | What it carries |
|---|---|---|
| `DomainAuthority` | Signed Domain access | Host-installed DDS keys and P2P credential; refresh and challenge signing remain behind one narrow handle. |
| `DomainRoutes` | Reachability | Explicit direct and complete relay-circuit candidates keyed by expected Peer ID. |
| authenticated info/resources | Diagnostics and resource discovery | Bounded participant metadata and live catalog rows. |
| authenticated registries/blobs | Content identity | Hash-pinned canonical registry JSON and verified binary blobs. |
| authenticated messages/streams | Live application data | Receiver-owned messages and typed streams with bounded framing/backpressure. |

Discovery, DDS/DMS adapters, or application configuration may populate routes
outside these crates. None of those sources changes the authentication rule.

---

## Quickstart

App code interacts with the SDK through `auki-session`'s `Peer` / `Session`
pair, plus `auki-domain` to go on-network. Stage 1 is not yet a published tag;
the old v0.0.x tags are Manager-era and are not wire-compatible with the code
below. Evaluate the current source checkout with:

```sh
cargo check --locked -p auki-diagnostic-app
```

Move a communicating group together to the coordinated 0.1 release line when
it is published. Breaking changes tick the minor version before 1.0.

Construct a peer, declare a sensor, start a session, register a log, inspect what the SDK advertises:

```rust
use std::time::Duration;
use auki_registry::{Camera, SensorBody};
use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};

let seed = auki_identity::load_or_mint_seed(&seed_path)?;
let wallet = auki_identity::Wallet::from_seed(seed.to_vec())?;
let peer_seed: [u8; 32] = wallet
    .derive_child("peer/v1")
    .seed()
    .try_into()
    .expect("Wallet seeds are always 32 bytes");
let identity = auki_p2p::Identity::from_ed25519_seed(&peer_seed);

let peer = Peer::new(identity.peer_id().to_string(), "galbot-ctrl")
    .with_storage_root("/data/auki/galbot-01".into());

let frame = peer.register_frame("head_left_camera_optical", FrameDef::ros_optical())?;

let sensor = peer.register_sensor("head_left_rgb", SensorBody::Camera(Camera {
    r#type: "rgb".into(),
    width: 1920, height: 1200, frame_rate_hz: 30,
    pixel_format: "rgb8".into(),
    color_space: "srgb".into(),
    intrinsics_model: "pinhole".into(),
    distortion_model: "brown_conrady".into(),
    frame: frame.clone(),
}))?;

// Mints a ULID session_id and auto-registers the session's
// monotonic + UTC clocks (#284) — no hand-rolled session clock.
let session = peer.start_session()?;

let _log = session.register_sensor_log(SensorLogSpec {
    sensor,
    clock: session.monotonic_clock(),
    frame: Some(frame),
    head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
    segment_duration: Duration::from_secs(1),
    retention: Duration::from_secs(5),
})?;

// The catalog is what authenticated peers see over /auki/auth/1/resources/0.2.0
// once a DomainBuilder with host-supplied authority puts this pair online.
for row in auki_domain::catalog_of(&peer, &session) {
    println!("{} owns {} ({})", row.source_peer_id, row.resource_id, row.state);
}
```

For a deeper walkthrough — including the Python equivalent and the remaining
`Session::materialize_remote_log` work — see the [wiki Quickstart](https://github.com/aukilabs/auki-sdk/wiki/Quickstart).
The [authenticated Domain migration guide](docs/authenticated-domain-migration.md)
covers the breaking network cutover. Each crate's `README.md` has the contract
spec for that layer.

---

## Cross-language conformance vectors

Critical wire-format and derivation chains are pinned by **locked test vectors** — exact-bytes/exact-strings the Rust crate produces from a fixed input. Any reimplementation in another language (Python, Go, browser JS) is correct only if it reproduces the same outputs from the same inputs. If a vector ever drifts, every consumer in the wild drifts with it; updates require a coordinated version bump.

| Crate | Test | What it locks |
|---|---|---|
| [`auki-hash`](crates/auki-hash) | `tests::locked_*` (existing) | XXH3-128 byte vectors (used for content-addressed registry hashes) |
| [`auki-identity`](crates/auki-identity) | `tests::locked_derive_child_peer_v1_pubkey_vector` | `Wallet::from_seed([3u8; 32]).derive_child("peer/v1").public_key()` → 32-byte ed25519 pubkey |
| [`auki-identity`](crates/auki-identity) | `tests::locked_sign_canonical_json_vector` | `Wallet::from_seed([3u8; 32]).sign_canonical_json(<Vinland-shaped registration>)` → exact RFC 8785 canonical bytes + 64-byte ed25519 signature |
| [`auki-identity-py`](bindings/python/auki-identity-py) | `tests::locked_peer_id_vector` | `Wallet.from_seed(b"\x03" * 32).derive_child("peer/v1").peer_id()` → the canonical `auki-p2p::Identity` Peer ID string |
| [`auki-protocols`](crates/auki-protocols) | `stream::v2::tests::camera_frame_serializes_to_locked_wire_bytes` | `CameraFrame { dynamic_intrinsics: None, frame: <10-byte JFIF prefix> }` → exact prost wire bytes (`120a` tag/length prefix + payload) |
| [`auki-protocols`](crates/auki-protocols) | `stream::v2::tests::point_cloud_data_serializes_to_locked_wire_bytes` | `point_cloud::Data { data: <8-byte fixture> }` → exact prost wire bytes for the shared disk/wire point-cloud payload |
| [`auki-protocols`](crates/auki-protocols) | `stream::v2::tests::locked_stream_message_entry_with_point_cloud_payload` | Full envelope: `StreamMessage::Entry { timestamp_ns, seq, payload: <prost-encoded point_cloud::Data> }` → exact prost-encoded bytes. Browser-side decoders and cross-language reimplementations pin against this. |
| [`auki-registry`](crates/auki-registry) | `tests::frame_entry_serializes_to_canonical_bytes_matching_locked_vector` | `FrameRegistryEntry::ros_body("K1-AABBCCDDEEFF/base_link")` → `{"axes":{"x":"forward","y":"left","z":"up"},...}` JSON + XXH3-128 `fd0dc3789e898b71b5e16ee122a81a44` |

The wallet-child vector plus `auki-p2p::Identity::from_ed25519_seed` pin the
`Wallet -> libp2p PeerId` derivation used by authenticated Domains. The stream vectors pin wire shapes
that cross language boundaries to browser and other consumers. The Frame
Registry vector pins convention names so every reader parses `"forward"` /
`"left"` / `"up"` byte-identically.

---

## Design and discussion

The SDK's design is being worked through across two spaces:

- **Long-form design docs** live in Notion under [About the real world web](https://www.notion.so/3495c8e9659280b38cc9ce1540f72a29). These cover the broader Auki protocol, the five questions, the registries and logs, and the Domain/Map architecture.
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
