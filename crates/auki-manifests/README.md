# auki-manifests

Single source of truth for the Auki SDK's **log manifests** — schemas + builders for the per-recording metadata that lives at the root of each `auki-logs` log directory. Symmetric with [`auki-datatypes`](../auki-datatypes), which owns segment payload shapes; this crate owns manifest shapes.

## Two crates, two halves of an `auki-logs::Log<T>`

| Concern | Crate | Encoding |
|---|---|---|
| Manifest (per-recording metadata at the log root) | this crate | JCS-canonical UTF-8 JSON via [`auki-jcs`](../auki-jcs) |
| Segment payloads (per-frame bulk data) | [`auki-datatypes`](../auki-datatypes) | Protobuf via prost |

Manifests stay JCS-JSON because (a) JCS gives free cross-language byte-equivalence for content-hashing the inline producer identities like [`PoseSource`](src/lib.rs); (b) manifests are operator-debugged via `cat`, browser-read by Park, and inspected by ad-hoc tooling — JSON is the universal denominator; (c) per-recording metadata doesn't benefit from protobuf's wire compactness (~500 bytes, written once, read by humans + code). See the [`auki-datatypes` parking-lot decision](../auki-datatypes/parking_lot.md) for the full rationale.

## What this crate exports

```rust
pub fn build_sensor_log_manifest(
    app_id, session_id,
    sensor_id, sensor_hash,
    clock_id, clock_hash,
    frame_id: Option<&str>, frame_hash: Option<&str>,
    segment_duration, retention,
) -> serde_json::Value;

pub fn build_pose_log_manifest(
    app_id, session_id,
    from_frame_id, from_frame_hash,
    to_frame_id, to_frame_hash,
    clock_id, clock_hash,
    source: &PoseSource,
    writer_mode: PoseWriterMode,
    expected_rate_hz: u32,
    segment_duration, retention,
) -> serde_json::Value;

pub fn build_time_transform_log_manifest(
    app_id, session_id,
    from_clock_id, from_clock_hash,
    to_clock_id, to_clock_hash,
    source: &TimeTransformSource,
    segment_duration, retention,
) -> serde_json::Value;

pub fn build_detection_log_manifest(
    app_id, session_id,
    detector_id, detector_hash,
    input_log_id,
    input_sensor_id, input_sensor_hash,
    clock_id, clock_hash,
    segment_duration, retention,
) -> serde_json::Value;

pub enum PoseSource {
    Ros2Tf { publishers: Vec<String> },
    // future: Slam { algorithm, map_id, ... }, Odometry { ... }, ManualFixture { ... }
}

pub enum PoseWriterMode {
    Rigid,    // serialized as "rigid"
    Movable,  // serialized as "movable"
}

pub enum TimeTransformSource {
    LocalClockRead,    // serialized as {"kind":"local_clock_read"}
    // future: NtpSynced { server }, SyncedTo { peer_id }, ...
}
```

`PoseSource` lives **inline** in the Pose Log manifest under the `"source"` key — Pose Log has no separate registry because provenance is the only thing `source` describes (frame identity now lives in the manifest's `(from_frame_id, to_frame_id)` pair, not on the per-sample transforms). Carries `canonical_bytes()` + `hash()` for content-addressing if a future producer variant graduates to a sibling registry.

## Manifest schemas

Every manifest extends `auki-logs`'s required base (`segment_duration_ns`, `retention_ns`) with additional fields per log type.

### Sensor Log family (Sensor Log, Point Cloud Log, Audio Log)

| Key                   | Type    | Notes                                                            |
| --------------------- | ------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer | > 0; from auki-logs                                              |
| `retention_ns`        | integer | ≥ 0; from auki-logs (0 = unbounded)                              |
| `app_id`              | string  | Application identifier — matches daemon's `/api/info.app`        |
| `session_id`          | string  | UUIDv4 minted by the integrator at app boot                      |
| `sensor_id`           | string  | The Sensor Registry ID this log captures                         |
| `sensor_hash`         | string  | XXH3-128 hex of the sensor's registry entry                      |
| `clock_id`            | string  | The Clock Registry ID for the framing's `timestamp_ns`           |
| `clock_hash`          | string  | XXH3-128 hex of the clock's registry entry                       |
| `frame_id`            | string? | Optional Frame Registry ID for spatial sensor samples            |
| `frame_hash`          | string? | Optional XXH3-128 hex of the frame's registry entry              |

The `(sensor_id, sensor_hash)` pair resolves to a [`SensorRegistryEntry`](../auki-registry) whose `body` variant tells a reader which payload type the segments hold (`CameraFrame`, `PointCloudLogEntry`, `JointEncodersLogEntry`, `AudioLogEntry`). Spatial sensors also pin their sample convention through the optional `(frame_id, frame_hash)` pair; both fields must be present together or omitted together.

### Pose Log

Per-`(from, to)` log identity (Step 5 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md), 2026-05-08). Each Pose Log holds samples for exactly one `(from_frame_id, to_frame_id)` pair; segment entries are flat [`auki_datatypes::pose::SpatialTransform`](../auki-datatypes/src/lib.rs).

| Key                   | Type            | Notes                                                            |
| --------------------- | --------------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer         | > 0; from auki-logs                                              |
| `retention_ns`        | integer         | ≥ 0; from auki-logs (0 = unbounded)                              |
| `app_id`              | string          | Same as Sensor Log                                               |
| `session_id`          | string          | Same as Sensor Log                                               |
| `from_frame_id`       | string          | The Frame Registry ID for the parent frame                       |
| `from_frame_hash`     | string          | XXH3-128 hex of the from-frame's registry entry                  |
| `to_frame_id`         | string          | The Frame Registry ID for the child frame                        |
| `to_frame_hash`       | string          | XXH3-128 hex of the to-frame's registry entry                    |
| `clock_id`            | string          | The Clock Registry ID for the framing's `timestamp_ns`           |
| `clock_hash`          | string          | XXH3-128 hex of the clock's registry entry                       |
| `source`              | tagged enum     | Inline producer identity — `PoseSource` (e.g. `{"kind":"ros2_tf","publishers":[...]}`) |
| `writer_mode`         | string          | `"rigid"` (stationary transform) or `"movable"` (time-varying)   |
| `expected_rate_hz`    | integer         | Producer's nominal sample rate; reader hint, not enforced        |

The `(from_frame_id, to_frame_id)` pair mirrors the TimeTransform Log's `(from_clock_id, to_clock_id)` shape — each log is a single ordered pair, with the registry hashes content-addressing the FrameRegistry entries that describe each side's coordinate convention. A producer that observes a multi-pair ROS `TFMessage` is responsible for fanning the message into N parallel pose logs.

### TimeTransform Log

| Key                   | Type            | Notes                                                            |
| --------------------- | --------------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer         | > 0; from auki-logs                                              |
| `retention_ns`        | integer         | ≥ 0; from auki-logs                                              |
| `app_id`              | string          | Same as Sensor Log                                               |
| `session_id`          | string          | Same as Sensor Log                                               |
| `from_clock_id`       | string          | The Clock Registry ID the framing's `timestamp_ns` is on         |
| `from_clock_hash`     | string          | XXH3-128 hex of the from-clock's registry entry                  |
| `to_clock_id`         | string          | The Clock Registry ID `offset_ns` carries you to                 |
| `to_clock_hash`       | string          | XXH3-128 hex of the to-clock's registry entry                    |
| `source`              | tagged enum     | Inline producer identity — `TimeTransformSource` (e.g. `{"kind":"local_clock_read"}`); added at Step 6 (2026-05-08), mirrors Pose Log's shape |

### Detection Log

| Key                   | Type            | Notes                                                            |
| --------------------- | --------------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer         | > 0; from auki-logs                                              |
| `retention_ns`        | integer         | ≥ 0; from auki-logs                                              |
| `app_id`              | string          | Same as Sensor Log                                               |
| `session_id`          | string          | Same as Sensor Log                                               |
| `detector_id`         | string          | Namespaced producer name (e.g. `"aukilabs/qr/v1"`). Mirrors `sensor_id`; opaque to the SDK |
| `detector_hash`       | string          | Content-hash binding the producer to a specific build (e.g. `hash(commit-SHA + config)` for code-only detectors, `hash(commit-SHA + weights + config)` for ML detectors). The exact `DetectorRegistryEntry` shape is **deferred** — the manifest carries the field as an opaque string for v1 |
| `input_log_id`        | string          | The `sensor_log_id` of the input log being tailed; pins WHICH instance of the sensor produced the frames                                  |
| `input_sensor_id`     | string          | Copied from the input sensor log's manifest so the detection log is self-contained                          |
| `input_sensor_hash`   | string          | Copied from the input sensor log's manifest                                                                  |
| `clock_id`            | string          | Same clock as the input log                                                                                  |
| `clock_hash`          | string          | Same clock-hash as the input log                                                                             |

The detection log opens with [`auki_layout::detection_log_path`](../auki-layout/src/lib.rs)'s on-disk shape `<session>/detection_logs/<detector_id>__<input_log_id>/`, mirroring how Sensor Logs and Pose Logs map a log identity to a directory. Segment payloads are [`auki_datatypes::detection::DetectionFrame`](../auki-datatypes/src/lib.rs) (Step 8 of the migration, 2026-05-08).

**No `intent` field** — the keystone's `buffer | intent_recording` dimension applies to every log but is not yet plumbed through the existing builders. Match-the-existing-builders for v1; uniform rollout is a separate PR.

## Versioning

Schema version is **1** for all four manifest shapes (Sensor Log family, Pose Log, TimeTransform Log, Detection Log) and for `PoseSource` / `PoseWriterMode` / `TimeTransformSource`. Bump on incompatible field changes. The `auki-logs` segment format version is independent; this crate only specifies what goes into the manifest header.

## Status

Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) (2026-05-08) extracted the manifest builders from `auki-registry` (`build_sensor_log_manifest`, `build_pose_log_manifest`, `PoseSource`) and `auki-time` (`build_manifest`, renamed to `build_time_transform_log_manifest` here for unambiguity). Step 5 (2026-05-08) rewrote `build_pose_log_manifest` for the new per-`(from, to)`-frame Pose Log identity per the 2026-05-07 synthesis: 13 args, including frame-pair fields, `writer_mode: PoseWriterMode`, and `expected_rate_hz: u32`. Step 6 (2026-05-08) added `&TimeTransformSource` as a `build_time_transform_log_manifest` argument and brought `TimeTransformSource` over from [`auki-time`](../auki-time) — it's manifest metadata, not a per-entry field, mirroring `PoseSource`.
