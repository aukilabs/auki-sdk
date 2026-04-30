# auki-registry

The typed data shapes the Auki SDK persists — registry entries (immutable identity) and log payloads (per-frame data) — together with the cross-language storage contract for registry entries.

## Two kinds of typed data

| Kind            | What it is                             | Where it lives                              |
| --------------- | -------------------------------------- | ------------------------------------------- |
| Registry entry  | Immutable identity, one per hash       | `<root>/registry/<kind>/<id>/<hash>.json`   |
| Log payload     | Per-frame mutable data                 | Inside an [`auki-logs`](../auki-logs) segment, CBOR-encoded |

Registry entries describe **what a thing is**; log payloads describe **what was sampled at a moment**. The split lets the registry stay stable (no version churn from per-frame intrinsics drift) while honoring that some fields really do change over time.

---

## Registry entries

### Storage layout

```
<root>/registry/sensors/<sensor_id>/<hash>.json
<root>/registry/clocks/<clock_id>/<hash>.json
<root>/registry/frames/<frame_id>/<hash>.json    ← coming
```

`/` in IDs is replaced by `__` so namespaced IDs like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment.

The hash *is* the version. Re-writing identical content is a no-op; writing different content under the same `id` produces a sibling file with a different hash. There are no version counters.

Hashes come from [`auki-hash`](../auki-hash) over the JCS-canonical bytes from [`auki-jcs`](../auki-jcs). All three crates form one indivisible content-addressing contract.

### Recommended sensor_id naming convention

The SDK treats `sensor_id` and `clock_id` as opaque strings — it does not parse, validate, or interpret them. Integrators are free to use any scheme.

For cross-app readability we **recommend** (do not enforce):

```
<platform-tag>-<machine-id>/<sensor-name>
```

For example, boosterapp produces `K1-AABBCCDDEEFF/head_rgb` for the K1's head RGB camera (platform tag `K1`, MAC-derived 12-hex machine id, sensor name). The shape is self-describing across captures from different platforms — looking at a registry path, an operator can tell which device produced it without consulting other metadata.

Whether this becomes a formal SDK convention or stays a documentation recommendation is an open question — see [`parking_lot.md`](parking_lot.md).

### Atomic writes

Writes go to `.<filename>.tmp` first, fsync, then rename. A crash mid-write leaves either nothing or the complete file; never a half-written one.

### `SensorRegistryEntry`

```
SensorRegistryEntry {
  sensor_id: string,
  type:      string,        // tagged-enum discriminant: "rgb_camera"; future: "depth", "imu", "lidar"
  ...body fields per type...
}
```

When `type = "rgb_camera"`:

```
RgbCamera {
  width:             u32,
  height:            u32,
  frame_rate_hz:     u32,
  pixel_format:      string,    // e.g. "YUV_NV12", "RGB8"
  color_space:       string,    // e.g. "BT.709", "sRGB"
  intrinsics_model:  string,    // e.g. "pinhole"
  distortion_model:  string,    // e.g. "plumb_bob", "none"
}
```

The tagged-enum body is the extension point for future sensor types.

### `ClockRegistryEntry`

```
ClockRegistryEntry {
  clock_id:  string,
  type:      string,        // "monotonic_clock" | "utc_clock"
  unit:      string,        // "ns"
  monotonic: bool,
  epoch:     string?,       // null for monotonic clocks (the absence is meaningful)
  scope:     string,        // "device-local" | "domain-local" | "global"
}
```

`epoch` MUST serialize as `"epoch": null` for monotonic clocks — the absence of an epoch is information, not omission. JCS canonicalization preserves null fields.

---

## Sensor Log payload — schema v1

The Sensor Log is an `auki_logs::Log<SensorLogEntry>`. The framing's `timestamp_ns` is the frame timestamp; the payload here carries per-frame data.

### Manifest

JCS-canonical UTF-8 JSON, written via auki-logs. Required keys (extends auki-logs's base):

| Key                    | Type    | Notes                                                            |
| ---------------------- | ------- | ---------------------------------------------------------------- |
| `segment_duration_ns`  | integer | > 0; from auki-logs                                              |
| `retention_ns`         | integer | ≥ 0; from auki-logs (0 = unbounded)                              |
| `clock_id`             | string  | The Clock Registry ID that the framing's `timestamp_ns` is in    |
| `clock_hash`           | string  | XXH3-128 hex of the clock's registry entry                       |
| `sensor_id`            | string  | The Sensor Registry ID this log captures                         |
| `sensor_hash`          | string  | XXH3-128 hex of the sensor's registry entry                      |

A reader resolves `(sensor_id, sensor_hash)` against `<root>/registry/sensors/<id>/<hash>.json` to recover dimensions, pixel format, and so on.

### Payload (CBOR)

```
SensorLogEntry {
  dynamic_intrinsics: DynamicIntrinsics,
  frame:              bytes,       // image data; encoding determined by sensor's pixel_format
}

DynamicIntrinsics {
  fx:                       f64,    // focal length in pixels
  fy:                       f64,
  cx:                       f64,    // principal point in pixels
  cy:                       f64,
  distortion_coefficients:  [f64],  // ordering matches the sensor's distortion_model
}
```

There is no `timestamp` field in the payload — the auki-logs framing's `timestamp_ns` is the single source of truth for "when this frame was captured."

### Why split static vs. dynamic

`SensorRegistryEntry` is content-addressed and immutable per hash. If intrinsics shifted on every frame and lived in the registry, every frame would mint a new sensor entry — defeating the registry's "one identity per camera" semantics. Pulling intrinsics into per-frame `DynamicIntrinsics` keeps the registry stable while honoring the reality that intrinsics drift on some platforms (autofocus, runtime calibration refinement).

The K1's intrinsics are essentially constant in practice, but the schema doesn't bake that assumption.

---

## Versioning

Schema version is **1** for all four types (`SensorRegistryEntry`, `ClockRegistryEntry`, `SensorLogEntry`, `DynamicIntrinsics`). Bump on incompatible field changes. The auki-logs segment format version is independent.
