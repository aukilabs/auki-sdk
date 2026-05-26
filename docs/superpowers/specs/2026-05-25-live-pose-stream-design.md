# Live Pose Stream Design

Date: 2026-05-25
Status: Approved for implementation
Issue: #206

## Goal

Add first-class live movable pose-edge support so RoboStreamer on a Galbot G1 can publish the time-varying transform from `base_link` to `head_left_rgb_optical`, and Park can discover and consume it while rendering `base_lidar_pointcloud` from the head-camera viewpoint.

The SDK already has the on-disk pose-log payload shape: `auki_datatypes::pose::SpatialTransform`, with `(from_frame_id, from_frame_hash, to_frame_id, to_frame_hash, clock_id, clock_hash, source, writer_mode, expected_rate_hz)` carried in the Pose Log manifest. This design brings the same identity model to the live resource and stream surfaces.

## Non-Goals

- Do not implement graph-level `convert_pose` path search or interpolation.
- Do not fake the Galbot head edge as a rigid `TransformEdgeResource`.
- Do not change the existing camera, point-cloud, joint-encoder, audio, or detection stream payload contracts.
- Do not require Park to interpret ROS `TFMessage` directly. RoboStreamer fans out the relevant edge and publishes SDK `SpatialTransform` samples.

## Resource Catalog

Add a new `/auki/resources/0.0.1` resource kind:

```rust
ResourceKind::PoseStream => "pose_stream"
ResourceEntry::PoseStream(PoseStreamResource)
```

`PoseStreamResource` carries the discoverable identity for one live pose edge:

```rust
PoseStreamResource {
    id: String,
    from_frame_id: String,
    from_frame_hash: String,
    to_frame_id: String,
    to_frame_hash: String,
    clock_id: String,
    clock_hash: String,
    stream_protocol: String,       // "/auki/stream/0.1.0"
    payload: String,               // "spatial_transform"
    writer_mode: String,           // "movable" for Galbot head-to-base
    expected_rate_hz: u32,         // 0 means unspecified
    source: Option<serde_json::Value>,
    from_frame_entry_json: Option<String>,
    to_frame_entry_json: Option<String>,
    clock_entry_json: Option<String>,
}
```

`ResourcesRequest` gains `include_clock_entries: bool`. `with_registry_entries()` sets sensor, frame, and clock embedding to true. Frame embedding applies to `TransformEdgeResource`, `SensorStreamResource`, and `PoseStreamResource`; clock embedding applies to `PoseStreamResource`.

For the first Galbot test, RoboStreamer advertises one row:

- `kind = "pose_stream"`
- `from_frame_id = "<galbot>/base_link"`
- `to_frame_id = "<galbot>/head_left_rgb_optical"`
- `writer_mode = "movable"`
- `payload = "spatial_transform"`

Park pairs this row with the existing `sensor_stream` rows for `base_lidar_pointcloud` and `head_left_rgb` by matching frame ids and hashes.

## Stream Protocol

Keep `/auki/stream/0.1.0` as the transport and add fields to the existing protobuf messages. Existing sensor stream wire bytes remain unchanged because default-valued new fields do not encode.

`StreamRequest` gains:

```proto
string resource_id = 2;
```

Sensor consumers keep sending `sensor_id`. Pose-stream consumers send `resource_id` with the resource catalog row id. Producers may decline requests with neither field, both fields, or an unknown id.

`StreamManifest` gains pose-resource identity fields:

```proto
string resource_id = 7;
string payload = 8;
string from_frame_id = 9;
string from_frame_hash = 10;
string to_frame_id = 11;
string to_frame_hash = 12;
string writer_mode = 13;
uint32 expected_rate_hz = 14;
```

For sensor streams, these new fields remain empty/zero. For pose streams, the existing `clock_id` and `clock_hash` are still the timestamp clock for `StreamEntry.timestamp_ns`; the existing `sensor_id`, `sensor_hash`, `frame_id`, and `frame_hash` remain empty.

The stream payload is the prost encoding of `auki_datatypes::pose::SpatialTransform`. The `StreamEntry.timestamp_ns` value is the sample time on `StreamManifest.clock_id`.

## Rust Surface

`auki-network` adds:

- `pub use auki_datatypes::pose`
- `StreamDispatch::AcceptPose { manifest, source: SourceStream<pose::SpatialTransform> }`
- `pump_typed::<pose::SpatialTransform>` dispatch in the inbound stream handler

`NetworkRuntime::open_stream::<pose::SpatialTransform>(peer_id, request)` already works once the dispatch arm exists. Higher layers can wrap it with clearer naming, but the generic Rust primitive remains the core consumer API.

`auki-domain` re-exports `PoseStreamResource` through its existing resource-catalog surface and enriches pose rows from the registry app root when requested.

## Python Surface

`auki_network.cluster` adds:

- `SpatialTransformFrame`
- `StreamDecision.accept_pose(manifest, source)`
- `StreamItem(payload=SpatialTransformFrame(...))`
- `StreamEntry.payload` returning `SpatialTransformFrame`

`SpatialTransformFrame` is the Python stream wrapper for `auki.pose.SpatialTransform`. It uses the sidecar-friendly numeric shape RoboStreamer and Park already exchange: a 7-value transform `(tx, ty, tz, qx, qy, qz, qw)`, with getters returning fresh Python values and conversion to the prost payload handled inside the binding.

`auki_domain` adds:

- `PoseStreamResource`
- resource provider extraction and fetched-resource conversion for `PoseStreamResource`
- `ClusterManager.open_pose_stream(peer_id, resource_id)`
- generic `ClusterManager.open_stream(peer_id, id)` support for `pose_stream` rows when the resource catalog advertises `payload = "spatial_transform"`

The Python producer path for RoboStreamer is:

1. Advertise `PoseStreamResource` from the resource catalog provider.
2. In `stream_provider(peer_id, request)`, match `request.resource_id`.
3. Return `StreamDecision.accept_pose(...)`.
4. Yield `StreamItem(timestamp_ns=..., payload=SpatialTransformFrame(...))`.

The Python consumer path for Park is:

1. Fetch the remote resource catalog with frame and clock entries.
2. Select the `pose_stream` row whose `from_frame_*` and `to_frame_*` match `base_link` to `head_left_rgb_optical`.
3. Open `ClusterManager.open_pose_stream(peer_id, resource.id)`.
4. Use `entry.timestamp_ns` on `subscription.manifest.clock_id` to align with `base_lidar_pointcloud` and `head_left_rgb`.

## Data Flow

RoboStreamer receives or computes Galbot head transforms from the robot stack. It normalizes each sample into a single `SpatialTransform` for `base_link -> head_left_rgb_optical`, timestamps it on the same clock model used by the Galbot sensor streams, and publishes it over the existing libp2p stream transport.

Park discovers the pose stream from `/auki/resources/0.0.1`, opens it by `resource_id`, and buffers recent pose samples keyed by timestamp. Park can then align point-cloud and camera frames using the advertised clock identity. Interpolation and graph composition remain Park-side or future SDK work.

## Error Handling

- Unknown `resource_id` returns `DeclineReason::sensor_not_found()` for wire compatibility.
- A pose provider yielding any non-pose payload ends the stream with `EndReason::producer_error(...)`.
- A pose stream whose manifest disagrees with its `PoseStreamResource` row is treated as producer error by consumers. Park should reject the stream and log the mismatch.
- Missing frame or clock registry entries do not prevent advertisement, but requested embedding leaves the corresponding `*_entry_json` empty.

## Testing

Implementation is complete when these pass:

- `auki-network` unit tests for `pose_stream` resource JSON round-trip and resource kind stability.
- `auki-network` stream runtime test proving one producer accepts and streams `pose::SpatialTransform` samples.
- `auki-domain` resource enrichment test embedding from/to frame entries and clock entry for `PoseStreamResource`.
- `auki-domain-py` tests for `PoseStreamResource`, `open_pose_stream`, and generic `open_stream` resolution.
- `auki-network-py` tests for `SpatialTransformFrame`, `StreamItem` conversion, `accept_pose`, and consumer entry conversion.
- `cargo check --workspace`.

First hardware acceptance is Galbot G1 through RoboStreamer into Park:

- RoboStreamer advertises `base_link -> head_left_rgb_optical` as a movable `pose_stream`.
- Park discovers the row and opens the live pose stream.
- Park receives timestamped `SpatialTransform` samples on the same clock model as the Galbot sensor streams.
- Park can combine the pose stream with `base_lidar_pointcloud` and `head_left_rgb` frame metadata without treating the head edge as rigid.
