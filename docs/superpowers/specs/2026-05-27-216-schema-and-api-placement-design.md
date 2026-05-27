# SDK as Robot Data Plane — Schema & API Placement Design

Date: 2026-05-27
Status: Approved for implementation
Issue: #216
Baseline: tag `v0.0.52`, commit `eeec1287`

## Goal

Lock the v1 schema and crate boundaries for [#216](https://github.com/aukilabs/auki-sdk/issues/216) — "Make the SDK the robot data plane for peer-owned logs." This spec resolves the issue's Open Design Points and the structural questions surfaced during review, so a follow-up writing-plans pass can produce a coordinated migration plan.

The product invariant from the issue:

```
A peer owns data products.
A data product has exactly one canonical peer_id.
Materialized copies preserve that peer_id.
```

After this design:

- Every log/registry record carries explicit peer-ownership fields.
- Every manifest is self-describing about origin vs writer.
- App-facing surface is declarative: apps tell a `Session` what they have; the SDK handles advertisement, discovery, transfer, and materialization.
- Apps never construct catalog rows, manifest JSON, or stream protocol identity by hand.

## Non-Goals

- Runtime brainstorming for the stream/read internals (eviction loops, segment writers, backpressure). Schemas and surface only.
- Detailed materialization replication semantics (sample lineage, multi-source convergence). The materialization API is sketched; the deep design lands in a follow-up spec.
- Swift / browser binding work beyond confirming the new schemas are exposable. Per-binding follow-ups are tracked as separate cards.
- Trust / signing of wire bytes. Same posture as before #216.

## §1 — Catalog row shape

Served from each peer over `/auki/resources/0.2.0`. Top-level fields:

| Field             | Description |
|-------------------|-------------|
| `source_peer_id`  | Canonical data origin (preserved across materializations) |
| `writer_peer_id`  | Peer that wrote the underlying manifest file (= serving peer for v1) |
| `resource_id`     | Per-log type structured id (see §6) |
| `kind`            | Closed enum: `camera \| point_cloud \| audio \| joint_encoders \| pose \| time_transform \| detection` |
| `state`           | Open-string discriminator; v1: `"live" \| "sealed"` |
| `head` *or* `extent` | Mutually exclusive bounds block keyed by `state` |
| `available`       | Snapshot of currently-retrievable data |
| `manifest`        | Canonical-field summary the consumer needs to materialize / stream |

Field-level types:

```
head (live):
  kind: "rolling" + retention_ns: i64
  kind: "fixed"   + started_at_ns: i64       (clock inherited from manifest)

extent (sealed):
  start_at_ns: i64
  finish_at_ns: i64                          (clock inherited from manifest)

available:
  bytes:        u64
  entries:      u64
  duration_ns:  i64
```

### Examples

Live rolling-head camera (origin):
```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "galbot",
  "resource_id": "head_left_rgb",
  "kind": "camera",
  "state": "live",
  "head": { "kind": "rolling", "retention_ns": 5000000000 },
  "available": { "bytes": 3000000000, "entries": 900, "duration_ns": 5000000000 },
  "manifest": {
    "sensor": { "peer_id": "galbot", "id": "head_left_rgb", "hash": "…" },
    "clock":  { "peer_id": "galbot", "id": "session/sdk_clock", "hash": "…" },
    "frame":  { "peer_id": "galbot", "id": "head_left_camera_optical", "hash": "…" }
  }
}
```

Live fixed-head pose log (intent recording):
```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "galbot",
  "resource_id": "left_gripper->object_pose",
  "kind": "pose",
  "state": "live",
  "head": { "kind": "fixed", "started_at_ns": 1733836800000000000 },
  "available": { "bytes": 18000000, "entries": 5000, "duration_ns": 30000000000 },
  "manifest": {
    "from_frame": { "peer_id": "galbot", "id": "left_gripper", "hash": "…" },
    "to_frame":   { "peer_id": "galbot", "id": "object_pose",  "hash": "…" },
    "clock":      { "peer_id": "galbot", "id": "session/sdk_clock", "hash": "…" }
  }
}
```

Sealed multi-sample camera log:
```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "galbot",
  "resource_id": "yesterday_capture",
  "kind": "camera",
  "state": "sealed",
  "extent": { "start_at_ns": 1733750400000000000, "finish_at_ns": 1733754000000000000 },
  "available": { "bytes": 50000000000, "entries": 108000, "duration_ns": 3600000000000 },
  "manifest": { … }
}
```

One-sample rigid pose log (static transform):
```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "galbot",
  "resource_id": "world->base_link",
  "kind": "pose",
  "state": "sealed",
  "extent": { "start_at_ns": 1733836800000000000, "finish_at_ns": 1733836800000000000 },
  "available": { "bytes": 80, "entries": 1, "duration_ns": 0 },
  "manifest": {
    "from_frame": { "peer_id": "park",   "id": "world",     "hash": "…" },
    "to_frame":   { "peer_id": "galbot", "id": "base_link", "hash": "…" },
    "clock":      { "peer_id": "galbot", "id": "session/sdk_clock", "hash": "…" }
  }
}
```

Materialization (Park serving Galbot's RGB with 5-min local retention):
```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "park",
  "resource_id": "head_left_rgb",
  "kind": "camera",
  "state": "live",
  "head": { "kind": "rolling", "retention_ns": 300000000000 },
  "available": { "bytes": 12000000000, "entries": 9000, "duration_ns": 300000000000 },
  "manifest": { "sensor": {…}, "clock": {…}, "frame": {…} }
}
```

The materialized row's `head` reflects Park's local cache policy. The consumer fetching Park's underlying manifest sees Park's segment / retention layout; the consumer fetching Galbot's manifest sees Galbot's. Row and manifest are internally consistent on the serving peer.

The `manifest` block on the row carries the manifest's *canonical* fields — every field a materializer needs to recreate the data shape on the consumer side. Writer-local manifest fields (`app_id`, `session_id`, `segment_duration_ns`, `retention_ns`) are not in the block; the consumer either doesn't need them (materialization picks its own) or sees `retention_ns` via the row's `head` block.

Per kind, the manifest block is:

```
camera | point_cloud | audio | joint_encoders:
  sensor: RegistryRef, clock: RegistryRef, frame: RegistryRef

pose:
  from_frame: RegistryRef, to_frame: RegistryRef, clock: RegistryRef,
  writer_mode: "rigid" | "movable",
  source: PoseSource,
  expected_rate_hz: u32

time_transform:
  from_clock: RegistryRef, to_clock: RegistryRef,
  source: TimeTransformSource

detection:
  detector: RegistryRef, input_log: LogRef, input_sensor: RegistryRef, clock: RegistryRef
```

A consumer with just the row has everything needed to identify the log, fetch the underlying registry entries, materialize a local copy with its own retention, or open a stream. No separate manifest-fetch endpoint is required for v1.

## §2 — Registry entry schema

Four types stay: `SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`. Each gains `peer_id` as a top-level field. IDs drop the historical peer-prefix path encoding; `(peer_id, id)` is the unique key.

### Rust shapes

```rust
// crates/auki-registry/src/lib.rs

pub struct SensorRegistryEntry {
    pub peer_id: PeerId,
    pub sensor_id: String,
    pub body: SensorBody,            // closed enum (Camera | PointCloud | Audio | JointEncoders)
}

pub struct ClockRegistryEntry {
    pub peer_id: PeerId,
    pub clock_id: String,
    pub body: ClockBody,
}

pub struct FrameRegistryEntry {
    pub peer_id: PeerId,
    pub frame_id: String,
    pub handedness: Handedness,
    pub axes: AxesMap,
    pub units: Units,
}

pub struct DetectorRegistryEntry {
    pub peer_id: PeerId,
    pub detector_id: String,
    pub body: DetectorBody,
    pub output_types: Vec<String>,
}
```

`PeerId` reuses `auki-identity::PeerId`.

### `SensorBody` knock-on

The body's nested frame reference becomes a `RegistryRef`:

```rust
pub enum SensorBody {
    Camera {
        width: u32, height: u32, frame_rate_hz: f32,
        pixel_format: String, color_space: String,
        intrinsics_model: String, distortion_model: String,
        frame: RegistryRef,                  // was (frame_id, frame_hash)
    },
    PointCloud { frame: RegistryRef, … },
    Audio { … },
    JointEncoders { … },
}
```

### Canonical JSON

```json
{
  "peer_id": "galbot",
  "sensor_id": "head_left_rgb",
  "body": {
    "type": "camera",
    "width": 1920, "height": 1200,
    "frame_rate_hz": 30.0,
    "pixel_format": "rgb8",
    "color_space": "srgb",
    "intrinsics_model": "pinhole",
    "distortion_model": "brown_conrady",
    "frame": { "peer_id": "galbot", "id": "head_left_camera_optical", "hash": "…" }
  }
}
```

### Disk layout

```
<app_root>/registries/
  sensors/<peer_id>/<sensor_id>/<sensor_hash>.json
  clocks/<peer_id>/<clock_id>/<clock_hash>.json
  frames/<peer_id>/<frame_id>/<frame_hash>.json
  detectors/<peer_id>/<detector_id>/<detector_hash>.json
```

Materialized registry entries from another peer are stored under that peer's `peer_id` segment, never under self.

### Wire (`/auki/registries/0.2.0`)

Same JCS-canonical JSON byte-for-byte. Request keys: `(kind, id, hash)`. The serving peer responds with its own owned entry; entries owned by other peers are served only if locally materialized (rare; v1 focuses on self-serve).

### Hash regeneration

Adding `peer_id` changes every canonical JSON. All existing `sensor_hash` / `clock_hash` / `frame_hash` / `detector_hash` references go stale.

## §3 — Manifest schema

Manifests describe a *specific log file*. Fields split into:

**Canonical (preserved across materializations; "what is this data"):**
- `source_peer_id` — data origin
- registry refs (`sensor` / `clock` / `frame` / `from_frame` / `to_frame` / `detector` / `input_log` / `input_sensor` / `from_clock` / `to_clock` as applicable)
- `writer_mode` (pose logs only)
- `source` (pose, time-transform)
- `expected_rate_hz` (pose only)

**Writer-local (set by this file's writer; "who wrote it, how is it stored"):**
- `writer_peer_id`
- `app_id`
- `session_id`
- `segment_duration_ns`
- `retention_ns`

Cross-references to registry entries become a `RegistryRef` triple. Cross-references to other logs use `LogRef`.

```rust
// crates/auki-registry/src/lib.rs (shared types)

pub struct RegistryRef {
    pub peer_id: PeerId,
    pub id: String,
    pub hash: String,
}

pub struct LogRef {
    pub source_peer_id: PeerId,
    pub resource_id: String,
}
```

### Rust shapes

```rust
// crates/auki-manifests/src/lib.rs

pub struct SensorLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub sensor: RegistryRef,
    pub clock:  RegistryRef,
    pub frame:  Option<RegistryRef>,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

pub struct PoseLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub from_frame: RegistryRef,
    pub to_frame:   RegistryRef,
    pub clock:      RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,    // Rigid | Movable
    pub expected_rate_hz: u32,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

pub struct TimeTransformLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub from_clock: RegistryRef,
    pub to_clock:   RegistryRef,
    pub source: TimeTransformSource,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

pub struct DetectionLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub detector:     RegistryRef,
    pub input_log:    LogRef,
    pub input_sensor: RegistryRef,
    pub clock:        RegistryRef,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}
```

### Materialized manifest example

Park materializing Galbot's `head_left_rgb` with 5-min retention, 10s segments:

```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "park",
  "app_id":         "park-vis",
  "session_id":     "01HV-park-session",
  "sensor": { "peer_id": "galbot", "id": "head_left_rgb",        "hash": "…" },
  "clock":  { "peer_id": "galbot", "id": "session/sdk_clock",    "hash": "…" },
  "frame":  { "peer_id": "galbot", "id": "head_left_camera_optical", "hash": "…" },
  "segment_duration_ns": 10000000000,
  "retention_ns":        300000000000
}
```

Samples don't carry `peer_id`. The log header carries it; samples on the wire travel under the protocol context `(source_peer_id, resource_id)`.

## §4 — `auki-session` crate (new)

New crate hosting the app-facing declarative surface. Apps interact with `Session`; the SDK does the rest.

### `Session`

```rust
// crates/auki-session/src/lib.rs

pub struct Session {
    inner: Arc<RwLock<SessionInner>>,
    domain: Option<Domain>,                 // populated after join_domain
}

struct SessionInner {
    peer_id: PeerId,                        // self
    app_id: String,
    session_id: String,                     // ULID, generated at Session::new
    storage_root: PathBuf,
    sensors:   RegistryStore<SensorRegistryEntry>,
    clocks:    RegistryStore<ClockRegistryEntry>,
    frames:    RegistryStore<FrameRegistryEntry>,
    detectors: RegistryStore<DetectorRegistryEntry>,
    sensor_logs:    HashMap<(PeerId, String), Arc<SensorLog>>,
    pose_logs:      HashMap<(PeerId, String), Arc<PoseLog>>,
    time_logs:      HashMap<(PeerId, String), Arc<TimeTransformLog>>,
    detection_logs: HashMap<(PeerId, String), Arc<DetectionLog>>,
}

impl Session {
    pub fn new(peer_id: PeerId, app_id: impl Into<String>) -> Self;
    pub fn with_storage_root(self, root: PathBuf) -> Self;

    // -- registry registration (returns RegistryRef to feed into log specs) --
    pub fn register_sensor(&self,   sensor_id: &str,   body: SensorBody)   -> Result<RegistryRef>;
    pub fn register_clock(&self,    clock_id: &str,    body: ClockBody)    -> Result<RegistryRef>;
    pub fn register_frame(&self,    frame_id: &str,    frame: FrameDef)    -> Result<RegistryRef>;
    pub fn register_detector(&self, detector_id: &str, body: DetectorBody) -> Result<RegistryRef>;

    // -- log registration (declarative spec → handle) --
    pub fn register_sensor_log(&self,         spec: SensorLogSpec)         -> Result<SensorLogHandle>;
    pub fn register_pose_log(&self,           spec: PoseLogSpec)           -> Result<PoseLogHandle>;
    pub fn register_time_transform_log(&self, spec: TimeTransformLogSpec) -> Result<TimeTransformLogHandle>;
    pub fn register_detection_log(&self,      spec: DetectionLogSpec)      -> Result<DetectionLogHandle>;

    // -- domain participation --
    pub async fn join_domain(&mut self, config: ClusterConfig) -> Result<()>;
    pub async fn leave_domain(&mut self) -> Result<()>;

    // -- remote consumption --
    pub async fn fetch_remote_catalog(&self, peer: PeerId) -> Result<Vec<ResourceEntry>>;
    pub async fn open_remote_stream<T: Payload>(&self, log_ref: LogRef, from: ReadFrom) -> Result<RemoteStream<T>>;
    pub async fn materialize_remote_log(&self, log_ref: LogRef, retention: Duration, segment_duration: Duration) -> Result<MaterializedLogHandle>;

    // -- internal (used by auki-domain) --
    pub fn catalog(&self) -> Vec<ResourceEntry>;
    pub fn get_manifest(&self, source_peer_id: &PeerId, resource_id: &str) -> Option<ManifestBlob>;
}
```

### Spec types

```rust
pub enum HeadSpec {
    Rolling { retention_ns: i64 },          // ring buffer; eviction at this age
    Fixed,                                  // append-only; Session stamps started_at_ns on creation
}

pub struct SensorLogSpec {
    pub sensor: RegistryRef,
    pub clock:  RegistryRef,
    pub frame:  Option<RegistryRef>,
    pub head:   HeadSpec,
    pub segment_duration: Duration,
    pub retention:         Duration,
}

pub struct PoseLogSpec {
    pub from_frame: RegistryRef,
    pub to_frame:   RegistryRef,
    pub clock:      RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,
    pub expected_rate_hz: u32,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention:         Duration,
}

pub struct TimeTransformLogSpec { /* from_clock, to_clock, source, head, segment, retention */ }
pub struct DetectionLogSpec     { /* detector, input_log, input_sensor, clock, head, segment, retention */ }
```

### Handles

```rust
pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    write_sink: WriteSink<auki_datatypes::camera::CameraFrame>,
}

impl SensorLogHandle {
    pub fn write(&mut self, frame: CameraFrame) -> Result<()>;
    pub fn seal(self) -> Result<()>;
}
```

Analogous `PoseLogHandle`, `TimeTransformLogHandle`, `DetectionLogHandle`, `MaterializedLogHandle`.

### Wiring direction

`auki-session` depends on `auki-domain`. `Session::join_domain` constructs a `Domain` internally, handing it an `Arc<RwLock<SessionInner>>`. App code never touches `Domain` directly.

### App-side example

```rust
let mut session = Session::new(self_peer_id, "boosterapp");

let sensor = session.register_sensor("head_left_rgb", Camera { … })?;
let clock  = session.register_clock("sdk_clock",      MonotonicClock { … })?;
let frame  = session.register_frame("head_left_camera_optical", FrameDef::ros_optical())?;

let log = session.register_sensor_log(SensorLogSpec {
    sensor, clock,
    frame: Some(frame),
    head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
    segment_duration: Duration::from_secs(1),
    retention:         Duration::from_secs(5),
})?;

session.join_domain(cluster_config).await?;

log.write(camera_frame)?;
```

## §5 — Domain protocol surface

Domain runs inside `Session` and exposes three libp2p protocols.

```
/auki/resources/0.2.0    — request peer's catalog; response Vec<ResourceEntry>
/auki/registries/0.2.0   — fetch a specific registry entry by (kind, id, hash)
/auki/stream/0.2.0       — read/tail a log
```

No `/auki/manifests/0.2.0` endpoint in v1 — the catalog row's `manifest` block (see §1) carries every canonical field a consumer needs. Writer-local fields are either not required for consumption (the materializer picks its own) or are surfaced through the row's `head` block (`retention_ns` on a live rolling log). If a future need for full-manifest fetch emerges (full provenance audit, debugging), it lands as a follow-up endpoint.

### Stream request

```rust
pub struct StreamRequest {
    pub source_peer_id: PeerId,             // canonical data origin
    pub resource_id: String,
    pub from: ReadFrom,
}

pub enum ReadFrom {
    Latest,
    FromStart,
    FromTimestamp(i64),                     // clock inherited from the manifest
}
```

`writer_peer_id` is implicit by the libp2p connection: you're talking to the writer of whatever file gets served. If a serving peer holds multiple files for the same `(source_peer_id, resource_id)` (rare; same-peer materialization), v1 picks one deterministically (most recent / largest); a disambiguator can be added later.

### Catalog production

```rust
impl Session {
    pub fn catalog(&self) -> Vec<ResourceEntry>;
}
```

Returns all locally-owned logs (`writer_peer_id == self.peer_id, source_peer_id == self.peer_id`) plus all locally-stored materializations (`writer_peer_id == self.peer_id, source_peer_id != self.peer_id`).

### Materialization (sketch)

```rust
impl Session {
    pub async fn materialize_remote_log(
        &self,
        log_ref: LogRef,
        retention: Duration,
        segment_duration: Duration,
    ) -> Result<MaterializedLogHandle>;
}
```

Steps:
1. Fetch a catalog row matching `log_ref` from the cluster (canonical owner preferred; any materializer accepted as fallback).
2. Extract canonical fields from the row's `manifest` block (registry refs, writer_mode, source, expected_rate_hz as applicable).
3. Open `/auki/stream/0.2.0` against the serving peer with `StreamRequest { source_peer_id, resource_id, from }`.
4. Write a new local manifest: `source_peer_id = log_ref.source_peer_id`, `writer_peer_id = self.peer_id`, `app_id` / `session_id` = local Session's, local segment / retention from args.
5. Ingest incoming samples into the local `Log<T>` instance.

Local storage layout for materializations:

```
<storage_root>/logs/<source_peer_id>/<resource_id>/
  manifest.json                  # writer = self, source = source_peer_id
  segments/0001.bin
  segments/0002.bin
  …
```

Deep design (sample lineage on multi-source convergence, materializer-of-materializer chains, eviction policy under multi-consumer load) is deferred to a follow-up spec.

### Resolution strategy (consumer side)

`Session::open_remote_stream(log_ref, from)`:

1. If `log_ref.source_peer_id` is reachable in the cluster, prefer it (canonical source has freshest data).
2. Otherwise, walk peer catalogs and pick any peer advertising `(source_peer_id, resource_id)`.

## §6 — `resource_id` derivation rules

SDK-enforced format strings per log type. Apps never type the resource_id literal — it derives from the log's bindings inside `Session::register_*_log`.

```
Sensor log:         <sensor.id>
Pose log:           <from_frame.id> -> <to_frame.id>
Time-transform log: <from_clock.id> -> <to_clock.id>
Detection log:      <detector.id> @ <input_sensor.id>
```

### Charset constraint

Registry entry IDs (`sensor_id`, `clock_id`, `frame_id`, `detector_id`) must not contain:
- `>` (collides with the `->` pose / time-transform separator)
- `@` (collides with the detection separator)
- whitespace

Enforced at `Session::register_*` time. Migration of the #216 schema validates existing IDs in any seeded fixtures.

### Materialization preserves resource_id

`Session::materialize_remote_log` takes `LogRef.resource_id` verbatim. `source_peer_id` differs from `writer_peer_id`, but the resource_id stays.

### Uniqueness contract

`(source_peer_id, resource_id)` must be unique within a writer's local namespace.

- `register_*_log`: source = self; reject duplicate `resource_id` on same-self entries.
- `materialize_remote_log`: source = remote; reject duplicate of `(remote.source_peer_id, remote.resource_id)`.

Cross-writer overlaps (Park and another peer both holding their own materialization of Galbot's `head_left_rgb`) are fine — different writer_peer_ids disambiguate at the row level.

### Cross-peer references in format strings

Pose / time-transform format strings encode only the local IDs (no peer prefix). The writer guarantees uniqueness within its own namespace. If a writer needs to disambiguate two pose logs that both connect frames named `world` from different peers, the writer picks distinct local naming — the SDK enforces uniqueness but doesn't auto-prefix.

## §7 — Testing & migration locks

### Locked canonical JSON fixtures

```
crates/auki-registry/tests/locked/
  sensor_camera.json
  sensor_point_cloud.json
  sensor_audio.json
  sensor_joint_encoders.json
  clock_monotonic.json
  clock_utc.json
  frame_ros_body.json
  frame_ros_optical.json
  frame_opengl.json
  frame_unity.json
  detector_object_detection.json

crates/auki-manifests/tests/locked/
  sensor_log_origin.json
  sensor_log_materialized.json
  pose_log_rigid.json
  pose_log_movable.json
  time_transform_log.json
  detection_log.json

crates/auki-network/tests/locked/
  catalog_row_live_rolling_camera.json
  catalog_row_live_fixed_pose.json
  catalog_row_sealed_camera.json
  catalog_row_sealed_one_sample_pose.json
  catalog_row_materialization.json
  resources_request_response.json
  registries_request_response.json
  stream_request.json
```

Each fixture pins JCS-canonical JSON byte order and its content hash. Round-trip tests catch unintended schema drift.

### Wire round-trip tests

For each of the four protocols, round-trip a representative request and response through the Rust types and assert byte-equality against the locked fixture.

### Cross-language parity tests

Python bindings (`auki-registry-py`, `auki-manifests-py`, `auki-session-py`, `auki-network-py`) construct each locked fixture from their public API and assert byte-equal canonical JSON output. Catches divergence in field ordering, defaulting, or serialization.

### Charset enforcement tests

`Session::register_*` rejects IDs containing `>`, `@`, whitespace. Each rejection produces a typed error variant (not just a string).

### Materialization smoke test

`crates/auki-session/tests/materialization.rs` walks the end-to-end flow on a tmpdir:

1. Session A (peer = galbot) registers sensor / clock / frame.
2. Session A registers a sensor log with rolling head; writes samples.
3. Assert local manifest has `source_peer_id == writer_peer_id == galbot`.
4. Session B (peer = park) in the same tmpdir materializes Galbot's log with different retention and segment_duration.
5. Assert Park's manifest has `source_peer_id == galbot`, `writer_peer_id == park`, Park-chosen segment / retention.
6. Park's catalog row shows `(source_peer_id == galbot, writer_peer_id == park)`.
7. A third Session C consumes from Park successfully; samples decode to the same payload bytes Galbot wrote.

### Clean-cut data migration

The new SDK refuses to parse pre-#216 registry entries, manifests, or catalog rows. Consumers (Park, Booster) wipe their on-disk caches during the upgrade. The Migration Notes section of #216 endorses this posture; document the wipe step in the SDK release notes for v0.0.53 (or whatever version this lands as).

### Hash regeneration

A `cargo xtask regen-locked-fixtures` target regenerates every locked JSON fixture and its hash from the updated structs. Run once when the schema lands; PR review diffs the regenerated fixtures against the struct changes.

## Crate impact summary

| Crate | Change |
|-------|--------|
| `auki-registry` | Add `peer_id` to all 4 entries; introduce `RegistryRef`, `LogRef`; update `SensorBody` to use `RegistryRef` for frame ref; update disk layout; regen locked fixtures |
| `auki-manifests` | Add `source_peer_id` + `writer_peer_id` to all 4 manifests; switch cross-refs to `RegistryRef` / `LogRef`; regen locked fixtures |
| `auki-network` | New `/auki/resources/0.2.0`, `/auki/registries/0.2.0`, `/auki/stream/0.2.0` protocols; delete old `SensorStreamResource`, `TransformEdgeResource`, `PoseStreamResource`; new `ResourceEntry` shape per §1; new `StreamRequest` per §5 |
| `auki-domain` | Stop being app-facing; expose the four protocols above against a session handle; remove `stream_manifest`, `cluster_manager` public re-exports that apps used; integration becomes internal-to-`auki-session` |
| `auki-session` (NEW) | Per-process Session, declarative registration API, log handles, materialization, owns Domain internally |
| `auki-logs` | No schema change; gain a `HeadSpec`-aware constructor variant for fixed-head logs |
| `bindings/python/auki-registry-py` | Mirror new shapes; regen test vectors |
| `bindings/python/auki-manifests-py` | Mirror new shapes; regen test vectors |
| `bindings/python/auki-domain-py` | Stop being app-facing; either delete or re-expose under auki-session-py (decide during plan) |
| `bindings/python/auki-session-py` (NEW) | Python facade over the new Rust crate |
| `bindings/swift`, `bindings/browser` | Out of scope for this card; leave explicit follow-up cards |
| `dataproducts.md` | Update once the schema lands; in flight on branch `docs/210-rewrite-dataproducts` |

## Out of scope (follow-up cards)

- Deep materialization replication semantics (sample lineage, multi-source convergence, materializer-of-materializer)
- Swift / browser binding surface updates
- Stream protocol payload framing changes (the wire bytes for samples within `/auki/stream/0.2.0` are unchanged for v1; future protobuf revisions are separate)
- Signing / authenticated wire bytes
- `dataproducts.md` text rewrite once schema lands (current `docs/210-rewrite-dataproducts` branch carries the bulk of it; needs a follow-up commit aligning with the locked schema)
