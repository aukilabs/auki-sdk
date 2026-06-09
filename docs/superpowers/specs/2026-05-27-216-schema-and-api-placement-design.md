# SDK as Robot Data Plane — Schema & API Placement Design

Date: 2026-05-27
Status: Approved for implementation
Issue: #216
Baseline: tag `v0.0.52`, commit `eeec1287`

## Revision history

- **2026-05-27**: initial draft from the #216 brainstorm.
- **2026-05-27 (rev 2)**: after issue #216 was edited post-brainstorm. Adopted the three-axis taxonomy (`variant` / `sensor.kind` / `sensor.type`), moved `writer_mode` into a `pose` block on the catalog row, made `time_transform_log` an explicit catalog variant, and added the `TransformEdgeResource` consumer migration. `SensorBody::PointCloud` is renamed to `SensorBody::Rangefinder` with `point_cloud` becoming a sensor.type; new `SensorBody::Rf` variant ships with a minimal body.

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

Served from each peer over `/auki/resources/0.2.0`. The row carries five axes that stay orthogonal — never collapsed into one field:

| Axis | Field | Type | Notes |
|------|-------|------|-------|
| Resource variant | `variant` | closed enum | `sensor_log \| pose_log \| time_transform_log \| detection_log` |
| Sensor family (sensor_log only) | `sensor.kind` | closed enum | `camera \| rangefinder \| rf \| audio \| joint_encoders` |
| Sensor modality (sensor_log only) | `sensor.type` | open string | kind-scoped, documented constants |
| Lifecycle | `state` | open string | v1: `"live" \| "sealed"` |
| Live head behavior | `head.kind` | closed enum | `rolling \| fixed`; only on live rows |
| Pose semantics (pose_log only) | `pose.writer_mode` | closed enum | `rigid \| movable`; mirrors `PoseWriterMode` |

Top-level fields, in JCS-canonical (alphabetical) order:

| Field | Present on | Description |
|-------|------------|-------------|
| `available` | all | Snapshot of currently-retrievable data |
| `extent` | sealed | Closed-range block (mutually exclusive with `head`) |
| `head` | live | Head-behavior block (mutually exclusive with `extent`) |
| `manifest` | all | Variant-specific registry refs the consumer needs to resolve the log |
| `pose` | pose_log | `{ writer_mode: "rigid" \| "movable" }` |
| `resource_id` | all | Per-variant derived id (see §6) |
| `sensor` | sensor_log | `{ kind, type, sensor_id, sensor_hash }` |
| `source_peer_id` | all | Canonical data origin (preserved across materializations) |
| `state` | all | Lifecycle discriminator |
| `variant` | all | Closed resource variant |
| `writer_peer_id` | all | Peer that wrote the underlying manifest file (= serving peer for v1) |

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

sensor (sensor_log only):
  kind:        SensorKind                    (closed: camera | rangefinder | rf | audio | joint_encoders)
  type:        String                        (open string; common values documented per kind)
  sensor_id:   String                        (peer-local; same as the row's resource_id)
  sensor_hash: String                        (content hash of the SensorRegistryEntry)

pose (pose_log only):
  writer_mode: PoseWriterMode                (Rigid | Movable; mirrors PoseLogManifest::PoseWriterMode)

manifest (variant-keyed contents — see Per-variant manifest blocks below)
```

### Sensor kind set + documented type constants

Closed `sensor.kind`:

- `camera`
- `rangefinder` — replaces the previous `point_cloud` kind. `point_cloud` becomes a sensor.type under rangefinder.
- `rf` — new kind for RF-based sensors. v1 ships the kind in the catalog enum + a minimal `SensorBody::Rf` registry body; production-quality RF sensors land via follow-up cards.
- `audio`
- `joint_encoders`

Documented `sensor.type` constants per kind (open string — these are SDK-documented common values, not an enforced enum):

```
camera:           rgb | depth | ir | mono | multispectral
rangefinder:      point_cloud | 2d_lidar | 3d_lidar | ultrasonic | radar
rf:               wifi | bluetooth | uwb
audio:            pcm | opus
joint_encoders:   absolute | incremental
```

Producers may use unlisted type strings; consumers MUST handle unknown types gracefully (fall back to the kind-level handler or ignore the row).

### Per-variant manifest blocks

The `manifest` block contains only the registry refs and canonical bindings the consumer needs to resolve the log. Sensor identity (kind/type/sensor_id/sensor_hash) and pose semantics (writer_mode) are hoisted into the dedicated `sensor` / `pose` blocks above.

```
sensor_log:
  clock: RegistryRef
  frame: Option<RegistryRef>

pose_log:
  from_frame: RegistryRef
  to_frame:   RegistryRef
  clock:      RegistryRef
  source:     PoseSource
  expected_rate_hz: u32

time_transform_log:
  from_clock: RegistryRef
  to_clock:   RegistryRef
  source:     TimeTransformSource

detection_log:
  detector:     RegistryRef
  input_log:    LogRef
  input_sensor: RegistryRef
  clock:        RegistryRef
```

A consumer with just the row has everything needed to identify the log, fetch the underlying registry entries by hash, materialize a local copy with its own retention, or open a stream. No separate manifest-fetch endpoint is required for v1.

### Examples

Live rolling-head sensor_log (camera, origin):
```json
{
  "available": { "bytes": 3000000000, "duration_ns": 5000000000, "entries": 900 },
  "head": { "kind": "rolling", "retention_ns": 5000000000 },
  "manifest": {
    "clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "frame": { "hash": "…", "id": "head_left_camera_optical", "peer_id": "galbot" }
  },
  "resource_id": "head_left_rgb",
  "sensor": {
    "kind": "camera",
    "sensor_hash": "…",
    "sensor_id": "head_left_rgb",
    "type": "rgb"
  },
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "sensor_log",
  "writer_peer_id": "galbot"
}
```

Sensor_log on a rangefinder (3D point cloud lidar):
```json
{
  "available": { "bytes": 1500000000, "duration_ns": 1000000000, "entries": 100 },
  "head": { "kind": "rolling", "retention_ns": 1000000000 },
  "manifest": {
    "clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "frame": { "hash": "…", "id": "head_lidar", "peer_id": "galbot" }
  },
  "resource_id": "head_lidar",
  "sensor": {
    "kind": "rangefinder",
    "sensor_hash": "…",
    "sensor_id": "head_lidar",
    "type": "3d_lidar"
  },
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "sensor_log",
  "writer_peer_id": "galbot"
}
```

Live fixed-head pose_log (movable, intent recording):
```json
{
  "available": { "bytes": 18000000, "duration_ns": 30000000000, "entries": 5000 },
  "head": { "kind": "fixed", "started_at_ns": 1733836800000000000 },
  "manifest": {
    "clock":      { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "expected_rate_hz": 30,
    "from_frame": { "hash": "…", "id": "left_gripper",      "peer_id": "galbot" },
    "source":     { "kind": "manual" },
    "to_frame":   { "hash": "…", "id": "object_pose",       "peer_id": "galbot" }
  },
  "pose": { "writer_mode": "movable" },
  "resource_id": "left_gripper->object_pose",
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "pose_log",
  "writer_peer_id": "galbot"
}
```

One-sample rigid pose_log (static transform):
```json
{
  "available": { "bytes": 80, "duration_ns": 0, "entries": 1 },
  "extent": { "finish_at_ns": 1733836800000000000, "start_at_ns": 1733836800000000000 },
  "manifest": {
    "clock":      { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "expected_rate_hz": 0,
    "from_frame": { "hash": "…", "id": "world",     "peer_id": "park"   },
    "source":     { "kind": "calibration" },
    "to_frame":   { "hash": "…", "id": "base_link", "peer_id": "galbot" }
  },
  "pose": { "writer_mode": "rigid" },
  "resource_id": "world->base_link",
  "source_peer_id": "galbot",
  "state": "sealed",
  "variant": "pose_log",
  "writer_peer_id": "galbot"
}
```

Live rolling time_transform_log:
```json
{
  "available": { "bytes": 4096, "duration_ns": 60000000000, "entries": 60 },
  "head": { "kind": "rolling", "retention_ns": 60000000000 },
  "manifest": {
    "from_clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "source":     { "kind": "heartbeat" },
    "to_clock":   { "hash": "…", "id": "wall_clock",        "peer_id": "galbot" }
  },
  "resource_id": "session/sdk_clock->wall_clock",
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "time_transform_log",
  "writer_peer_id": "galbot"
}
```

Live rolling detection_log:
```json
{
  "available": { "bytes": 250000, "duration_ns": 5000000000, "entries": 150 },
  "head": { "kind": "rolling", "retention_ns": 5000000000 },
  "manifest": {
    "clock":        { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "detector":     { "hash": "…", "id": "yolo_v8",           "peer_id": "galbot" },
    "input_log":    { "resource_id": "head_left_rgb", "source_peer_id": "galbot" },
    "input_sensor": { "hash": "…", "id": "head_left_rgb",     "peer_id": "galbot" }
  },
  "resource_id": "yolo_v8@head_left_rgb",
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "detection_log",
  "writer_peer_id": "galbot"
}
```

Materialization (Park serving Galbot's RGB with 5-min local retention):
```json
{
  "available": { "bytes": 12000000000, "duration_ns": 300000000000, "entries": 9000 },
  "head": { "kind": "rolling", "retention_ns": 300000000000 },
  "manifest": {
    "clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "frame": { "hash": "…", "id": "head_left_camera_optical", "peer_id": "galbot" }
  },
  "resource_id": "head_left_rgb",
  "sensor": {
    "kind": "camera",
    "sensor_hash": "…",
    "sensor_id": "head_left_rgb",
    "type": "rgb"
  },
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "sensor_log",
  "writer_peer_id": "park"
}
```

The materialized row's `head` reflects Park's local cache policy. The consumer fetching Park's underlying manifest sees Park's segment / retention layout; the consumer fetching Galbot's manifest sees Galbot's. The `sensor.sensor_hash` and registry refs point at Galbot's canonical registry entries (peer_id=galbot inside the RegistryRef), preserving identity.

### Axis-separation notes

- `state=sealed + pose.writer_mode=rigid + available.entries=1` is the canonical "static transform" shape. There is no separate transform-edge row variant — consumers detect rigid pose logs by these three fields.
- `head.kind` (rolling/fixed) is only meaningful while `state=live`. Sealed rows omit `head` and carry `extent` instead.
- Pose `writer_mode` is *pose-specific semantics*, not a generic resource axis. Non-pose variants never carry the `pose` block.

## §2 — Registry entry schema

Four types stay: `SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`. Each gains `peer_id` as a top-level field. IDs drop the historical peer-prefix path encoding; `(peer_id, id)` is the unique key.

### Rust shapes

```rust
// crates/auki-registry/src/lib.rs

pub struct SensorRegistryEntry {
    pub peer_id: PeerId,
    pub sensor_id: String,
    pub body: SensorBody,            // closed enum (Camera | Rangefinder | Rf | Audio | JointEncoders)
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

### `SensorBody` restructuring

Each body gains a `type: String` (the open-string sensor.type from §1) and switches frame refs to `RegistryRef`. `SensorBody::PointCloud` is renamed to `SensorBody::Rangefinder`; a new `SensorBody::Rf` variant ships with a minimal body.

```rust
pub enum SensorBody {
    Camera {
        r#type: String,                              // "rgb" | "depth" | "ir" | "mono" | "multispectral" | …
        width: u32,
        height: u32,
        frame_rate_hz: u32,
        pixel_format: String,
        color_space: String,
        intrinsics_model: String,
        distortion_model: String,
        frame: RegistryRef,                          // was (frame_id, frame_hash)
    },
    Rangefinder {
        r#type: String,                              // "point_cloud" | "2d_lidar" | "3d_lidar" | "ultrasonic" | "radar" | …
        // For type="point_cloud" the body carries the migrated PointCloud fields below.
        // For other types, the additional fields are omitted (Option/None) until
        // a future card lands the schema for that type.
        fields: Vec<PointField>,                     // point_cloud type
        point_step: u32,                             // point_cloud type
        is_bigendian: bool,                          // point_cloud type
        frame_rate_hz: u32,
        frame: RegistryRef,
    },
    Rf {
        r#type: String,                              // "wifi" | "bluetooth" | "uwb" | …
        frame: RegistryRef,
        // Minimal v1 body — actual rf-sensor canonical fields land via a follow-up
        // card when an SDK producer ships. The kind exists in v1 so catalog rows
        // can declare `sensor.kind = "rf"` without a registry-shape mismatch.
    },
    Audio {
        r#type: String,                              // "pcm" | "opus" | …
        sample_rate_hz: u32,
        channels: u8,
        sample_format: String,
        frame: RegistryRef,
    },
    JointEncoders {
        r#type: String,                              // "absolute" | "incremental"
        joint_count: u8,
        units: String,
        frame: RegistryRef,
    },
}
```

Migration: existing `SensorBody::PointCloud` bodies become `SensorBody::Rangefinder` with `type = "point_cloud"`. Existing `Camera` / `Audio` / `JointEncoders` bodies gain a `type` field with the documented default for that kind (`type = "rgb"` for camera, `type = "pcm"` for audio, `type = "absolute"` for joint_encoders) — adjusted by the producer where they know better.

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

let sensor = session.register_sensor("head_left_rgb", SensorBody::Camera {
    r#type: "rgb".to_string(),
    width: 1920, height: 1200, frame_rate_hz: 30,
    pixel_format: "rgb8".to_string(),
    color_space: "srgb".to_string(),
    intrinsics_model: "pinhole".to_string(),
    distortion_model: "brown_conrady".to_string(),
    frame: head_left_camera_optical_ref.clone(),
})?;
let clock  = session.register_clock("sdk_clock",      ClockBody::MonotonicClock { /* … */ })?;
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

### `TransformEdgeResource` consumer migration

Today consumers read a static rigid transform directly from `TransformEdgeResource.transform` on the catalog row — a single inline field. After #216 there is no separate transform-edge variant; rigid transforms are sealed one-sample `pose_log` rows with `pose.writer_mode = "rigid"`.

Migration recipe for consumers:

1. Walk a peer's catalog and filter for rows where `variant = "pose_log"` and `pose.writer_mode = "rigid"` and `state = "sealed"`.
2. For each, open a stream against the canonical owner with `StreamRequest { source_peer_id, resource_id, from: ReadFrom::FromStart }`.
3. Read exactly one `SpatialTransform` sample. Use it as the rigid transform.

Consumer code that previously did:

```rust
for row in catalog {
    if let ResourceEntry::TransformEdge(edge) = row {
        scenegraph.insert_rigid_edge(edge.from_frame_id, edge.to_frame_id, edge.transform);
    }
}
```

becomes:

```rust
for row in catalog {
    if row.variant == Variant::PoseLog
        && row.pose.as_ref().map_or(false, |p| p.writer_mode == PoseWriterMode::Rigid)
    {
        let mut stream = session.open_remote_stream::<SpatialTransform>(
            LogRef { source_peer_id: row.source_peer_id.clone(), resource_id: row.resource_id.clone() },
            ReadFrom::FromStart,
        ).await?;
        let sample = stream.next().await.unwrap()?;
        scenegraph.insert_rigid_edge(row.manifest.pose_from(), row.manifest.pose_to(), sample.payload);
    }
}
```

A `Session` helper (`Session::resolve_static_transform(log_ref)`) can wrap the open-stream-then-one-sample dance so consumers don't write that loop by hand. Add the helper as part of Phase 4.

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
  sensor_camera_rgb.json
  sensor_camera_depth.json
  sensor_rangefinder_point_cloud.json
  sensor_rangefinder_3d_lidar.json
  sensor_rf_wifi.json
  sensor_audio_pcm.json
  sensor_joint_encoders_absolute.json
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
  catalog_row_sensor_log_camera_live_rolling.json
  catalog_row_sensor_log_rangefinder_live_rolling.json
  catalog_row_sensor_log_sealed.json
  catalog_row_sensor_log_materialization.json
  catalog_row_pose_log_movable_live_fixed.json
  catalog_row_pose_log_rigid_sealed.json
  catalog_row_time_transform_log.json
  catalog_row_detection_log.json
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
| `auki-registry` | Add `peer_id` to all 4 entries; introduce `RegistryRef`, `LogRef`; rename `SensorBody::PointCloud` → `SensorBody::Rangefinder`; add `SensorBody::Rf` variant; add `type: String` to every sensor body; update disk layout; regen locked fixtures |
| `auki-manifests` | Add `source_peer_id` + `writer_peer_id` to all 4 manifests; switch cross-refs to `RegistryRef` / `LogRef`; regen locked fixtures |
| `auki-network` | New `/auki/resources/0.2.0`, `/auki/registries/0.2.0`, `/auki/stream/0.2.0` protocols; delete old `SensorStreamResource`, `TransformEdgeResource`, `PoseStreamResource`; new `ResourceEntry` shape per §1 with `variant` + `sensor` + `pose` blocks; new `StreamRequest` per §5 |
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
