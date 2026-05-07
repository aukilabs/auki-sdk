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
    segment_duration, retention,
) -> serde_json::Value;

pub fn build_pose_log_manifest(
    app_id, session_id,
    clock_id, clock_hash,
    source: &PoseSource,
    segment_duration, retention,
) -> serde_json::Value;

pub fn build_time_transform_log_manifest(
    app_id, session_id,
    from_clock_id, from_clock_hash,
    to_clock_id, to_clock_hash,
    segment_duration, retention,
) -> serde_json::Value;

pub enum PoseSource {
    Ros2Tf { publishers: Vec<String> },
    // future: Slam { algorithm, map_id, ... }, Odometry { ... }, ManualFixture { ... }
}
```

`PoseSource` lives **inline** in the Pose Log manifest under the `"source"` key — Pose Log has no separate registry because the segment payload is fully self-describing (frame names sit in each transform); source identity is provenance, not a decoder. Carries `canonical_bytes()` + `hash()` for content-addressing if a future producer variant graduates to a sibling registry.

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

The `(sensor_id, sensor_hash)` pair resolves to a [`SensorRegistryEntry`](../auki-registry) whose `body` variant tells a reader which payload type the segments hold (`SensorLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`).

### Pose Log

| Key                   | Type            | Notes                                                            |
| --------------------- | --------------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer         | > 0; from auki-logs                                              |
| `retention_ns`        | integer         | ≥ 0; from auki-logs (0 = unbounded)                              |
| `app_id`              | string          | Same as Sensor Log                                               |
| `session_id`          | string          | Same as Sensor Log                                               |
| `clock_id`            | string          | The Clock Registry ID for the framing's `timestamp_ns`           |
| `clock_hash`          | string          | XXH3-128 hex of the clock's registry entry                       |
| `source`              | tagged enum     | Inline producer identity — `PoseSource` (e.g. `{"kind":"ros2_tf","publishers":[...]}`) |

No `(sensor_id, sensor_hash)` pair — Pose Log has no sensor registry; the payload is self-describing via the frame names in each transform.

> **Pose Log shape is changing** per the synthesis decided 2026-05-07 (per-`(from, to)` identity instead of per-producer; flat `SpatialTransform` segment entries; rigid/movable writer mode; manifest gains `from_frame_id` / `to_frame_id` / `writer_mode`). The current shape above is what lands today via Step 0; the redesign lands in Step 5 of [`../auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md).

### TimeTransform Log

| Key                   | Type    | Notes                                                            |
| --------------------- | ------- | ---------------------------------------------------------------- |
| `segment_duration_ns` | integer | > 0; from auki-logs                                              |
| `retention_ns`        | integer | ≥ 0; from auki-logs                                              |
| `app_id`              | string  | Same as Sensor Log                                               |
| `session_id`          | string  | Same as Sensor Log                                               |
| `from_clock_id`       | string  | The Clock Registry ID the framing's `timestamp_ns` is on         |
| `from_clock_hash`     | string  | XXH3-128 hex of the from-clock's registry entry                  |
| `to_clock_id`         | string  | The Clock Registry ID `offset_ns` carries you to                 |
| `to_clock_hash`       | string  | XXH3-128 hex of the to-clock's registry entry                    |

## Versioning

Schema version is **1** for all three manifest shapes (Sensor Log family, Pose Log, TimeTransform Log) and for `PoseSource`. Bump on incompatible field changes. The `auki-logs` segment format version is independent; this crate only specifies what goes into the manifest header.

## Status

Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) — pure refactor extracting the manifest builders from `auki-registry` (`build_sensor_log_manifest`, `build_pose_log_manifest`, `PoseSource`) and `auki-time-transforms` (`build_manifest`, renamed to `build_time_transform_log_manifest` here for unambiguity). No behavior change, no encoding change.
