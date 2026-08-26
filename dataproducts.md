# Data Products — peer discovery descriptors

> **Status: v1 shipped in #216.** This document describes the post-#216 schema.
> The pre-#216 `CameraLogProduct` / `PointCloudLogProduct` draft is superseded.

## Purpose

A *data product* is one externally addressable thing a node can offer — a sensor log, a pose log, a TimeTransform Log, or a Detection Log. Peers on the Auki network need to discover what data products a node holds: enough metadata to interpret the payload bytes, align timestamps with their own clock, locate the data in space, and decide whether to fetch.

This document describes the **`ResourceEntry` descriptor schema** — the serializable shape one peer sends to another to advertise a single data product over `/auki/auth/1/resources/0.2.0`.

---

## `/auki/auth/1/resources/0.2.0` live catalog contract

Authenticated Domain participation and resource readiness are decoupled. A
peer may join before its sensors, logs, registries, or stream handlers are
ready. Consumers must treat `/auki/auth/1/resources/0.2.0` as a live, pollable snapshot of
the resources that are currently requestable from that peer.

Contract:

- A resource row means the producer can currently accept the matching
  `/auki/auth/1/stream/0.2.0` open for that `source_peer_id` + `resource_id` on the
  peer being dialed.
- Producers should not advertise resources whose stream opens would currently
  fail because the backing stream, log, or registry dependency is not ready.
- Consumers, including Park, are expected to poll `/auki/auth/1/resources/0.2.0` and
  reconcile rows that appear or disappear over time.
- `resource_id` values are stable logical IDs scoped to `source_peer_id`.
  Temporary outages should remove a row from the catalog, not mint a new ID; if
  the same logical resource becomes requestable again, it should reappear with
  the same `resource_id`.
- An empty `resources` list is a valid response: the peer has joined and
  supports the protocol, but currently advertises no requestable resources.

The current schema has no supported `unavailable` or `degraded` state. The
only documented `state` values are `"live"` and `"sealed"`, and the `available`
block describes coverage volume, not health. Until the schema grows an explicit
availability state, producers signal unavailable resources by omitting those
rows from the catalog. A row with `available.entries = 0` is only valid when the
producer can still accept the stream open (for example, a freshly-started live
tail with no samples yet).

## Resource catalog row (`ResourceEntry`)

Every log variant is described by one `ResourceEntry`. The row is discriminated by a closed `variant` field; variant-specific metadata lives in typed blocks (`sensor`, `pose`, `manifest`). Common fields (`source_peer_id`, `writer_peer_id`, `resource_id`, `state`, `head`/`extent`, `available`) appear on every row.

### Three-axis taxonomy for sensor logs

Sensor logs carry three orthogonal identification axes:

| Axis | Field | Type | Notes |
|------|-------|------|-------|
| Resource variant | `variant` | closed enum | `sensor_log` |
| Sensor family | `sensor.kind` | closed enum | `camera \| rangefinder \| rf \| audio \| joint_encoders` |
| Sensor modality | `sensor.type` | open string | kind-scoped string; see documented constants below |

The three axes are never collapsed. A consumer that needs "all lidar streams" filters on `sensor.kind = "rangefinder"`; a consumer that needs "only 3D lidar" also filters on `sensor.type = "3d_lidar"`. The `sensor_id` / `sensor_hash` pair in the `sensor` block links to the full `SensorRegistryEntry` for byte-level field metadata.

### Closed sensor kinds and documented type constants

Closed `sensor.kind` values:

- `camera` — optical imager (RGB, depth, IR, etc.)
- `rangefinder` — distance sensor (lidar, radar, ultrasonic, etc.); renamed from the former `point_cloud` kind. `point_cloud` is now a `sensor.type` value under this kind.
- `rf` — radio-frequency sensor (WiFi CSI, Bluetooth, UWB, etc.)
- `audio` — microphone or acoustic sensor
- `joint_encoders` — articulated joint encoder bank

Common `sensor.type` strings per kind (open — producers may use unlisted values):

```
camera:         rgb | depth | ir | mono | multispectral
rangefinder:    point_cloud | 2d_lidar | 3d_lidar | ultrasonic | radar
rf:             wifi | bluetooth | uwb
audio:          pcm | opus
joint_encoders: absolute | incremental
```

### `source_peer_id` vs `writer_peer_id`

Two peer identity fields appear on every row and on every manifest:

| Field | Meaning |
|-------|---------|
| `source_peer_id` | Canonical data origin — the peer whose physical sensor or actuator produced the data. Preserved across materializations. |
| `writer_peer_id` | The peer that holds the underlying manifest file and log bytes. Equals `source_peer_id` for origin rows; differs when a second peer (e.g. Park) materializes a copy of Galbot's log. |

A consumer that wants Galbot's original data follows `source_peer_id = "galbot"`. A consumer that wants to fetch bytes should dial `writer_peer_id` (the node that has them).

### Top-level fields

| Field | Present on | Description |
|-------|------------|-------------|
| `available` | all | Snapshot of currently-retrievable data (bytes, entries, duration_ns) |
| `extent` | sealed | Closed time-range block (mutually exclusive with `head`) |
| `head` | live | Head-behavior block: `rolling` (retention window) or `fixed` (start timestamp) |
| `manifest` | all | Variant-specific registry refs (see per-variant blocks below) |
| `pose` | pose_log | `{ writer_mode: "rigid" \| "movable" }` |
| `resource_id` | all | Per-variant derived id (sensor_id for sensor logs; `from->to` for pose/time-transform; `detector@sensor` for detections) |
| `sensor` | sensor_log | `{ kind, type, sensor_id, sensor_hash }` |
| `source_peer_id` | all | Canonical data origin |
| `state` | all | Lifecycle: `"live"` or `"sealed"` |
| `variant` | all | Closed resource variant |
| `writer_peer_id` | all | Peer holding the manifest and bytes |

### Per-variant manifest blocks

The `manifest` block carries only the registry refs and canonical bindings a consumer needs to resolve the log. Identity fields already hoisted into `sensor`/`pose` are not repeated.

```
sensor_log:
  clock: RegistryRef
  frame: Option<RegistryRef>

pose_log:
  from_frame:       RegistryRef
  to_frame:         RegistryRef
  clock:            RegistryRef
  source:           PoseSource
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

### Manifest file — source/writer split

On disk each log has a `manifest.json` at:

```
<storage_root>/logs/<writer_peer_id>/<resource_id>/manifest.json
```

The manifest JSON carries both `source_peer_id` and `writer_peer_id`. For origin logs (Galbot writing its own sensor data) the two are identical:

```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "galbot",
  "app_id": "galbot-ctrl",
  "session_id": "…",
  "sensor": { "peer_id": "galbot", "id": "head_left_rgb", "hash": "…" },
  "clock":  { "peer_id": "galbot", "id": "session/sdk_clock", "hash": "…" },
  "frame":  { "peer_id": "galbot", "id": "head_left_camera_optical", "hash": "…" },
  "segment_duration_ns": 1000000000,
  "retention_ns": 5000000000
}
```

For a materialization (Park caching Galbot's stream with a longer local retention), `source_peer_id` stays `"galbot"` but `writer_peer_id` becomes `"park"`.

---

## Concrete catalog row examples

### Live rolling sensor_log (RGB camera, origin)

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

### Live rolling sensor_log (rangefinder, 3D point cloud lidar)

Note: `sensor.kind = "rangefinder"` and `sensor.type = "point_cloud"`. The former `SensorBody::PointCloud` kind is replaced by `SensorBody::Rangefinder`; `point_cloud` is now a modality string within the rangefinder family.

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
    "type": "point_cloud"
  },
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "sensor_log",
  "writer_peer_id": "galbot"
}
```

### Live rolling sensor_log (non-spatial scalar battery charge)

Scalar Sensor Registry entries pin the measured quantity, unit, and expected
rate. Their Sensor Log manifest intentionally has no `frame`; each
`auki.scalar.Data` payload contains only `double value`.

```json
{
  "available": { "bytes": 900, "duration_ns": 60000000000, "entries": 60 },
  "head": { "kind": "rolling", "retention_ns": 60000000000 },
  "manifest": {
    "clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "bracketbot" }
  },
  "resource_id": "battery_charge",
  "sensor": {
    "kind": "scalar",
    "sensor_hash": "…",
    "sensor_id": "battery_charge",
    "type": "battery_charge"
  },
  "source_peer_id": "bracketbot",
  "state": "live",
  "variant": "sensor_log",
  "writer_peer_id": "bracketbot"
}
```

### Materialized sensor_log (Park serving Galbot's RGB, 5-min local retention)

`source_peer_id` is preserved as `"galbot"`; `writer_peer_id` is `"park"`. The `sensor.sensor_hash` and registry refs still point at Galbot's canonical entries.

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

### Live movable pose_log

```json
{
  "available": { "bytes": 18000000, "duration_ns": 30000000000, "entries": 5000 },
  "head": { "kind": "fixed", "started_at_ns": 1733836800000000000 },
  "manifest": {
    "clock":            { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "expected_rate_hz": 30,
    "from_frame":       { "hash": "…", "id": "left_gripper",      "peer_id": "galbot" },
    "source":           { "kind": "manual" },
    "to_frame":         { "hash": "…", "id": "object_pose",       "peer_id": "galbot" }
  },
  "pose": { "writer_mode": "movable" },
  "resource_id": "left_gripper->object_pose",
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "pose_log",
  "writer_peer_id": "galbot"
}
```

### Sealed rigid pose_log (static transform)

`state=sealed + pose.writer_mode=rigid + available.entries=1` is the canonical "static transform" shape. There is no separate transform-edge variant.

```json
{
  "available": { "bytes": 80, "duration_ns": 0, "entries": 1 },
  "extent": { "finish_at_ns": 1733836800000000000, "start_at_ns": 1733836800000000000 },
  "manifest": {
    "clock":            { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "expected_rate_hz": 0,
    "from_frame":       { "hash": "…", "id": "world",     "peer_id": "park"   },
    "source":           { "kind": "calibration" },
    "to_frame":         { "hash": "…", "id": "base_link", "peer_id": "galbot" }
  },
  "pose": { "writer_mode": "rigid" },
  "resource_id": "world->base_link",
  "source_peer_id": "galbot",
  "state": "sealed",
  "variant": "pose_log",
  "writer_peer_id": "galbot"
}
```

### Live time_transform_log

```json
{
  "available": { "bytes": 4096, "duration_ns": 60000000000, "entries": 60 },
  "head": { "kind": "rolling", "retention_ns": 60000000000 },
  "manifest": {
    "from_clock": { "hash": "…", "id": "session/sdk_clock", "peer_id": "galbot" },
    "source":     { "kind": "local_clock_read" },
    "to_clock":   { "hash": "…", "id": "wall_clock",        "peer_id": "galbot" }
  },
  "resource_id": "session/sdk_clock->wall_clock",
  "source_peer_id": "galbot",
  "state": "live",
  "variant": "time_transform_log",
  "writer_peer_id": "galbot"
}
```

### Live detection_log

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

---

## What the consumer gets in one catalog fetch

- **Log identity** — variant, resource_id, source/writer split.
- **Sensor metadata** — closed kind, open type string, and a content-addressed hash linking to the full `SensorRegistryEntry` (resolution via `/auki/auth/1/registries/0.3.0`).
- **Clock identity** — registry ref in the manifest block; resolve by hash to get unit, epoch, scope.
- **Spatial frame identity** — registry ref in the manifest block; resolve to get handedness, axes, units.
- **Coverage** — bytes, entries, duration on the `available` block; time bounds on `head` (live) or `extent` (sealed).
- **Lifecycle** — `state: "live"` or `"sealed"`.
- **Pose semantics** — `writer_mode: "rigid"` or `"movable"` on pose_log rows; `rigid + entries=1` is the canonical static-transform shape.

---

## Coverage semantics

- **Rolling head** (`head.kind = "rolling"`): `retention_ns` is the sliding-window size. The `available.duration_ns` reflects what is actually on disk — may be less than `retention_ns` if the session just started.
- **Fixed head** (`head.kind = "fixed"`): `started_at_ns` is the wall-clock time the log started; all data since then is available.
- **Sealed** (`state = "sealed"`): `extent.start_at_ns` / `extent.finish_at_ns` describe the closed-range archive.

---

## Migration notes from pre-#216 shapes

| Pre-#216 | Post-#216 |
|----------|-----------|
| `SensorStreamResource` | `ResourceEntry` with `variant: "sensor_log"` |
| `TransformEdgeResource` | `ResourceEntry` with `variant: "pose_log"`, `pose.writer_mode: "rigid"`, `state: "sealed"`, `available.entries: 1` |
| `PoseStreamResource` | `ResourceEntry` with `variant: "pose_log"`, `pose.writer_mode: "movable"` |
| `SensorBody::PointCloud` | `SensorBody::Rangefinder` with `type: "point_cloud"` |
| `sensor.kind = "point_cloud"` | `sensor.kind = "rangefinder"`, `sensor.type = "point_cloud"` |
| no `source_peer_id`/`writer_peer_id` split | explicit on every row and manifest |

---

## Out of scope (for v1)

- **Wire transport** — gossip vs. Map-mediated central registry vs. direct query. Auki protocol decision.
- **Trust / signing** — descriptor is just bytes; signing/authentication is a wrapper concern.
- **Domain identity / Map endpoint** — the Domain context this node participates in.
- **Connection info for fetching** — URL, peer ID, port. Depends on transport.
- **Multi-product wrappers** (`NodeManifest { products: [...] }`) — a level up; needed eventually but distinct schema.
- **Graph-level frame transform composition** — `convert_pose` path-finding across a frame tree is a consumer-side concern; the catalog only advertises what logs exist, not their composition.
