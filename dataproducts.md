# Resources — peer discovery

> **Status: v0 shipped.** The SDK ships peer-discovery descriptors through
> [`/auki/resources/0.0.1`](crates/auki-network/src/resources_protocol.rs)
> using the `ResourceEntry` enum (`SensorStream`, `TransformEdge`). The
> earlier per-product proposal — `CameraLogProduct` and siblings — was
> superseded; the historical sketch is preserved below the v0 section for
> design context.

## Purpose

A *resource* is one externally addressable thing a peer can offer right now — a live sensor stream, a direct rigid transform edge between two frames, and (in time) recordings, pose streams, detection streams, and calibration resources. Peers on the Auki network need to discover what their cluster-mates can provide: enough metadata to interpret payload bytes, locate the data in space, align it with their own clock, and decide whether to fetch.

This document describes the shipped `ResourceEntry` shape and where it falls short of the broader peer-discovery surface still being scoped.

---

## Shipped: `ResourceEntry` over `/auki/resources/0.0.1`

A consumer opens `/auki/resources/0.0.1` against any cluster peer, sends a `ResourcesRequest`, and gets back a `ResourcesResponse { resources: Vec<ResourceEntry> }`. The wire shape is JSON, length-prefixed; full Rust types in [`crates/auki-network/src/resources_protocol.rs`](crates/auki-network/src/resources_protocol.rs).

### `ResourcesRequest`

```
ResourcesRequest {
  kinds:                  [string],   // open-string filter; [] = "every kind I produce"
  include_sensor_entries: bool,       // embed canonical Sensor Registry JSON inline
  include_frame_entries:  bool,       // embed canonical Frame Registry JSON inline
}
```

The two `include_*` flags toggle eager-vs-lazy registry embedding. Default off — the consumer fetches registry entries through [`/auki/registries/0.0.1`](crates/auki-network) when it actually needs them. Turning them on collapses the round-trip when the consumer knows it will need every entry anyway (typical for browser-side viewers).

### `ResourceEntry`

A tagged enum (`"kind"` discriminator, open-string) with two v0 variants:

```
ResourceEntry { kind: "sensor_stream", ... } -> SensorStreamResource
ResourceEntry { kind: "transform_edge", ... } -> TransformEdgeResource
```

Future variants (pose streams, recordings, detection streams, calibration resources) add new `kind` strings without changing the protocol version. Cross-language consumers that pre-date a variant simply skip rows with unknown kinds.

### `SensorStreamResource` — live sensor stream over `/auki/stream/0.1.0`

```
SensorStreamResource {
  id:                  string,   // resource id; defaults to sensor_id in v0
  sensor_id:           string,
  sensor_hash:         string,   // content-addressed Sensor Registry hash
  sensor_kind:         string,   // "camera" / "point_cloud" / "joint_encoders" / "audio"
                                 // (the SensorBody serde tag, carried through as open string)
  stream_protocol:     string,   // "/auki/stream/0.1.0"
  payload:             string,   // decoder hint; e.g. "auki.camera.CameraFrame"

  // Optional live calibration. Producers that have a live calibration
  // snapshot (e.g. ROS CameraInfo) advertise it here.
  pinhole_intrinsics:  ResourcePinholeIntrinsics | null,

  // Optional inline registry JSON — populated when the request set
  // include_sensor_entries / include_frame_entries.
  sensor_entry_json:   string | null,   // canonical JSON of the Sensor Registry entry
  frame_entry_json:    string | null,   // canonical JSON of the camera's Frame Registry entry
}
```

`ResourcePinholeIntrinsics` is the numeric `{ fx, fy, cx, cy }` projection matrix; full intrinsics + distortion model still live in the Sensor Registry entry referenced by `sensor_hash`.

### `TransformEdgeResource` — direct rigid transform between two frames

```
TransformEdgeResource {
  id:                    string,                    // conventionally "<from>-><to>"
  from_frame_id:         string,
  from_frame_hash:       string,
  to_frame_id:           string,
  to_frame_hash:         string,
  writer_mode:           string,                    // "rigid" in v0; open string for future modes
  source:                <PoseSource-shaped JSON>,  // optional provenance; mirrors auki-manifests' tagged shape
  transform:             ResourceSpatialTransform,  // { translation: Vec3, orientation: Quat (Hamilton xyzw) }

  // Optional inline registry JSON — populated when the request set
  // include_frame_entries.
  from_frame_entry_json: string | null,
  to_frame_entry_json:   string | null,
}
```

Pose Log semantics: the transform takes a point in `from_frame_id` into `to_frame_id`. `writer_mode: "rigid"` means stationary — one sample, no time series. The mutable / movable equivalent (a live pose stream) is a future resource kind.

### Why this shape and not `CameraLogProduct`

`CameraLogProduct` baked per-product-type descriptors with eager full registry embedding, coverage metadata, status (live/sealed/aborted), and time/spatial bridge menus. The shipped `ResourceEntry`:

- **One envelope, many kinds.** Sensor streams, transform edges, and future kinds share one enum rather than a `*LogProduct` per payload type. Cross-language clients route by an open-string `kind` and ignore unknown rows.
- **Live first.** v0 advertises what a peer can stream *now*. Recorded-log resources, with coverage and lifecycle, are a future variant — not the foundation.
- **Lazy registry embedding.** Consumers opt into inline registry JSON; the default path is a small row + a `/auki/registries/0.0.1` round-trip when needed.
- **No bridge menus.** Time-transform availability and frame-transform availability are first-class resources of their own (`transform_edge` ships in v0; pose streams + time-transform bridges will follow as new kinds). The producer doesn't pre-compute a Cartesian product of "this log × every clock × every pose chain" — the consumer composes the bridges it needs.

---

## Coverage gaps in v0

The `ResourceEntry` set is intentionally narrow today. Live pose streams, recorded sensor logs, detection-resource rows, time-transform bridges, and calibration resources are all expected future variants — they extend the enum, not replace it. Tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

The shipped Detection Log primitive (`Log<auki_datatypes::detection::DetectionFrame>` with `data` + `sensor_hash` + `type`, manifest built via `build_detection_log_manifest`) exists; what's missing is the `ResourceEntry` variant that advertises one. Same for Pose Logs (`Log<auki_datatypes::pose::SpatialTransform>` keyed per `(from, to)` frame pair) — the on-disk shape ships, the catalog row to discover them does not.

---

## Historical: `CameraLogProduct` — superseded proposal

The pre-`ResourceEntry` design is kept here for context. **Do not implement against it; the shipped surface is the `ResourceEntry` enum above.**

A *data product* was framed as one externally addressable recording a node had stored — a camera log, a point cloud log, a TimeTransform Log, a Pose Log, a Detection Log, or the live stream that could be materialized into the same shape later. The plan was one descriptor schema per product type:

```
CameraLogProduct {
  schema_version:  u32,
  app_id:          string,
  session_id:      string,
  log_id:          string,
  payload_type:    string,                  // "auki.camera.CameraFrame"

  sensor_id:       string,
  sensor_hash:     string,
  sensor_entry:    SensorRegistryEntry,     // embedded by value

  clock_id:        string,
  clock_hash:      string,
  clock_entry:     ClockRegistryEntry,      // embedded by value

  frame_id:        string,
  frame_hash:      string,
  frame_entry:     FrameRegistryEntry,      // embedded by value

  time_transforms:  [TimeTransformAvailability],   // bridge menu, by clock
  frame_transforms: [FrameTransformAvailability],  // bridge menu, by frame

  segment_duration_ns:   i64,
  retention_ns:          i64,
  earliest_timestamp_ns: i64,
  latest_timestamp_ns:   i64,
  segment_count:         u32,
  total_bytes:           u64,

  status:               "live" | "sealed" | "aborted",
  generated_at_ns:      i64,
}
```

Sibling per-type descriptors (`PointCloudLogProduct`, `TimeTransformLogProduct`, …) would have followed the same shape minus camera-specific fields. The plan never landed — `ResourceEntry` collapsed the "one envelope, every payload" question and shipped first.

### Schema details that did land

These pieces of the proposal made it into the shipped SDK, even though `CameraLogProduct` did not:

- **`Camera` registry body** — `width`, `height`, `frame_rate_hz`, `pixel_format`, `color_space`, `intrinsics_model`, `distortion_model`, and an exact `frame_id` + `frame_hash` reference to the optical Frame Registry entry. Lives in `auki-registry::SensorBody::Camera`.
- **`ClockMeta`** — `unit`, `monotonic`, `epoch`, `scope`. Lives in `auki-registry::ClockBody::{MonotonicClock, UtcClock}` (the variant carries the `monotonic` axis; `ClockMeta` carries the rest).
- **`FrameRegistryEntry`** — `frame_id`, `handedness`, `axes` (per-axis direction map), `units`. Four preset constructors (`ros_body` / `ros_optical` / `opengl` / `unity`).
- **`DetectionFrame`** — `data` (opaque per-detector bytes), `sensor_hash`, `type` (open-string discriminator). Detector identity lives in a `DetectorRegistryEntry` under `<app_root>/registries/detectors/<detector_id>/<hash>.json`, pinned from the Detection Log's manifest via `build_detection_log_manifest`.
- **Pose Log capture** — `Log<auki_datatypes::pose::SpatialTransform>`, manifest built via `build_pose_log_manifest` with `(from_frame_id, from_frame_hash) + (to_frame_id, to_frame_hash) + PoseSource + PoseWriterMode + expected_rate_hz`. Path layout via `poselog_path`. `auki-geometry` ships convention-level conversion (`convert_pose_convention`); the graph-level `convert_pose` operation that composes pose-log edges across a frame tree is still pending.

### What didn't carry over

- **Coverage / lifecycle fields** (`earliest_timestamp_ns`, `latest_timestamp_ns`, `segment_count`, `total_bytes`, `status`) — `ResourceEntry` advertises live capability, not on-disk history. A future `RecordedSensorLog` variant will need its own coverage shape.
- **`TimeTransformAvailability` / `FrameTransformAvailability` bridge menus** — the producer no longer pre-computes the Cartesian product. Consumers walk `transform_edge` rows (and future pose-stream rows) to compose the bridges they need.
- **`log_id` per-recording handle** — `ResourceEntry` ids are resource-scoped (`sensor_id` for sensor streams, `<from>-><to>` for transform edges). Per-recording handles re-appear when recorded-log resources land.

---

## Out of scope (for v0)

- **Trust / signing** — the wire bytes are not signed yet. Wrapping discovery in a signed envelope is a separate concern.
- **Domain identity / Map endpoint** — which Domain a peer participates in is a cluster-membership question, not a per-resource question.
- **Recorded-log resources** — coverage, lifecycle, segment metadata. Future enum variant.
- **Live pose-stream and detection-stream resources** — future enum variants; the on-disk shapes already exist, the catalog rows do not.
- **Raster / 2D frame conventions for image bytes** — see [#140](https://github.com/aukilabs/auki-sdk/issues/140). `frame_entry_json` describes the 3D optical frame; the raster convention of the published bytes (mirrored vs. not, origin, axes) is a parallel concern.
- **Peer-level frame-transform graph / scenegraph availability** — see [#141](https://github.com/aukilabs/auki-sdk/issues/141). The shipped `transform_edge` row is the per-edge slice; the peer-level view of all known edges + producer-derived output frames is future work.
