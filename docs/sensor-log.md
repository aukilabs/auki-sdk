# Sensor Log — payload schema v1

The Sensor Log is an [auki-logs](../src/crates/auki-logs/) `Log<SensorLogEntry>`. The framing's `timestamp_ns` is the frame timestamp (extracted from ROS2 `header.stamp` by [auki-ros-adapter](../src/crates/auki-ros-adapter/)). The payload — defined here — carries the per-frame intrinsics plus the encoded image bytes.

For the static, registry-side camera identity (sensor_id, dimensions, pixel_format, color_space, intrinsics_model, distortion_model), see `SensorRegistryEntry` in [auki-registry](../src/crates/auki-registry/src/lib.rs). The split is deliberate: per-frame fields are the ones that *change* (intrinsics refinement, autofocus); registry-side fields are camera identity.

## Manifest

JCS-canonical UTF-8 JSON, written via auki-logs. Required keys (extends auki-logs's base):

| Key                    | Type    | Notes                                                            |
| ---------------------- | ------- | ---------------------------------------------------------------- |
| `segment_duration_ns`  | integer | > 0; from auki-logs                                              |
| `retention_ns`         | integer | > 0; from auki-logs                                              |
| `clock_id`             | string  | The Clock Registry ID that the framing's `timestamp_ns` is in    |
| `clock_hash`           | string  | XXH3-128 hex of the clock's registry entry                       |
| `sensor_id`            | string  | The Sensor Registry ID this log captures                         |
| `sensor_hash`          | string  | XXH3-128 hex of the sensor's registry entry                      |

The renderer (task 8) resolves `(sensor_id, sensor_hash)` against `<session>/registry/sensors/<id>/<hash>.json` to recover dimensions, pixel format, etc.

## Entry payload (CBOR)

```
SensorLogEntry {
  dynamic_intrinsics: DynamicIntrinsics
  frame:              bytes              // image data; encoding determined by sensor's pixel_format
}

DynamicIntrinsics {
  fx:                       f64          // focal length in pixels
  fy:                       f64
  cx:                       f64          // principal point in pixels
  cy:                       f64
  distortion_coefficients:  [f64]        // ordering matches the sensor's distortion_model
}
```

There is no `timestamp` field in the payload — the auki-logs framing's `timestamp_ns` is the single source of truth for "when this frame was captured."

## Why split static vs. dynamic

`SensorRegistryEntry` is content-addressed and immutable per hash. If intrinsics shifted on every frame and lived in the registry, every frame would mint a new sensor entry — defeating the registry's "one identity per camera" semantics. Pulling intrinsics into per-frame `DynamicIntrinsics` keeps the registry stable while honoring the reality that intrinsics drift on some platforms (autofocus, runtime calibration refinement).

The K1's intrinsics are essentially constant in practice, but the schema doesn't bake that assumption.

## Mapping from ROS2 `sensor_msgs/CameraInfo`

| ROS2 field                      | Goes to                                                          |
| ------------------------------- | ---------------------------------------------------------------- |
| `width`                         | `SensorRegistryEntry.body.RgbCamera.width`                       |
| `height`                        | `SensorRegistryEntry.body.RgbCamera.height`                      |
| `distortion_model`              | `SensorRegistryEntry.body.RgbCamera.distortion_model`            |
| `k[0]`, `k[4]`, `k[2]`, `k[5]`  | `DynamicIntrinsics.fx`, `fy`, `cx`, `cy` (row-major K matrix)    |
| `d`                             | `DynamicIntrinsics.distortion_coefficients`                      |

Static metadata not present in `CameraInfo` — `pixel_format`, `color_space`, `frame_rate_hz`, `intrinsics_model` — is supplied by the integrator (the K1 binary, for M1) based on out-of-band knowledge of the platform. The K1's RGB head camera is `pixel_format=YUV_NV12`, `color_space=BT.709`, `frame_rate_hz=20`, `intrinsics_model=pinhole`.

## Mapping from ROS2 `sensor_msgs/Image`

| ROS2 field        | Goes to                                                                 |
| ----------------- | ----------------------------------------------------------------------- |
| `header.stamp`    | auki-logs framing `timestamp_ns` (sec × 1e9 + nanosec)                  |
| `data`            | `SensorLogEntry.frame`                                                  |
| `width`/`height`  | discarded (must agree with the registry entry)                          |
| `encoding`        | discarded (must agree with the registry entry's `pixel_format`)         |
| `step`            | discarded (NV12 stride is recoverable from width)                       |

Discarded fields are *checked* (debug-only or first-frame validation) but not stored; storing them per frame would be redundant with the registry entry.

## Versioning

Schema version is **1**. Bump on incompatible field changes to either `SensorLogEntry` or `DynamicIntrinsics`. The auki-logs segment format version is independent.
