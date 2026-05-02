# auki-registry

The typed data shapes the Auki SDK persists — registry entries (immutable identity) and log payloads (per-frame data) — together with the cross-language storage contract for registry entries.

## Two kinds of typed data

| Kind            | What it is                             | Where it lives                                          |
| --------------- | -------------------------------------- | ------------------------------------------------------- |
| Registry entry  | Immutable identity, one per hash       | `<app_root>/registries/<kind>/<id>/<hash>.json`         |
| Log payload     | Per-frame mutable data                 | Inside an [`auki-logs`](../auki-logs) segment, CBOR-encoded |

Registry entries describe **what a thing is**; log payloads describe **what was sampled at a moment**. The split lets the registry stay stable (no version churn from per-frame intrinsics drift) while honoring that some fields really do change over time.

---

## Registry entries

### Storage layout

```
<app_root>/registries/sensors/<sensor_id>/<hash>.json
<app_root>/registries/clocks/<clock_id>/<hash>.json
<app_root>/registries/frames/<frame_id>/<hash>.json    ← coming
```

Registries live at the **app root**, shared across every session of that app. Hash-keyed writes are idempotent, so a sensor entry that doesn't change between app starts produces the same `<hash>.json` regardless of session — re-writing it would be wasted work.

The full session shape (registries + per-session log directories) is documented in [`auki-session`](../auki-session).

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
  type:      string,        // tagged-enum discriminant: "rgb_camera" | "point_cloud" | "microphone"; future: "depth", "imu", "lidar"
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

When `type = "point_cloud"`:

```
PointCloud {
  fields:        [PointField],   // byte layout of one point
  point_step:    u32,            // bytes per point
  is_bigendian:  bool,           // byte order of multi-byte fields in `data`
  frame_rate_hz: u32,
}

PointField {
  name:     string,              // "x", "y", "z", "r", "g", "b", "a", "intensity", "ring", "t", ...
  offset:   u32,                 // byte offset within one point
  datatype: string,              // "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "float32" | "float64"
  count:    u32,                 // number of `datatype` elements; usually 1
}
```

When `type = "microphone"`:

```
Microphone {
  sample_rate_hz: u32,           // e.g. 48000
  channels:       u32,           // 1 mono, 2 stereo, N for arrays
  sample_format:  string,        // "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le"
                                 //   (raw PCM in v1; compressed formats added by extending this string)
  channel_layout: string,        // "mono" | "stereo" | "5.1" | "7.1" | "ambisonic_b" | "n_channel"
}
```

**Multi-microphone arrays are modelled as one sensor with `channels = N`,** not as N independent sensors. The right shape for physically-synchronized arrays where all channels share a clock and a beam-forming origin (K1 head array, MacBook beamformer). Use separate `SensorRegistryEntry` records only when mics are physically independent capture devices.

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

A reader resolves `(sensor_id, sensor_hash)` against `<app_root>/registries/sensors/<id>/<hash>.json` to recover dimensions, pixel format, and so on.

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

## Point Cloud Log payload — schema v1

The Point Cloud Log is a separate `auki_logs::Log<PointCloudLogEntry>`. Each point-cloud recording is its own directory at `<session>/sensorlogs/<recording_uuid>/` — same path scheme as a camera sensor log, just with a different sensor whose registry entry has `SensorBody::PointCloud` (the manifest's `sensor_hash` is what tells a reader to expect `PointCloudLogEntry` payloads). Capturing camera + point cloud simultaneously means two parallel recordings sharing a session, not one multi-sensor recording. The framing's `timestamp_ns` is the scan timestamp; the payload here carries per-frame data.

### Manifest

JCS-canonical UTF-8 JSON. Same shape as the Sensor Log manifest — the `(sensor_id, sensor_hash)` pair resolves to a `SensorBody::PointCloud` registry entry, which is how a reader knows the segment payloads are `PointCloudLogEntry` rather than `SensorLogEntry`.

### Payload (CBOR)

```
PointCloudLogEntry {
  width:    u32,        // organized: cols; unorganized: total point count
  height:   u32,        // organized: rows; unorganized: 1
  is_dense: bool,       // true if no invalid (NaN/Inf) points in `data`
  data:     bytes,      // length MUST equal point_step × width × height (point_step from registry)
}
```

`data` is encoded as a CBOR byte string (major type 2), not an array of `u8` — same on-disk semantics, ~half the byte cost on typical point clouds. `SensorLogEntry.frame` uses the same encoding for the same reason.

### RGB(A) normalization

ROS2's `sensor_msgs/PointCloud2` historically packs RGB into a `float32` whose 4 bytes are interpreted as `0x00RRGGBB` (or `0xAARRGGBB` for `rgba`). The translation layer in [`auki-ros-adapter`](../auki-ros-adapter) **normalizes** this:

- A field with `name = "rgb"`, `datatype = float32`, `count = 1` → three sequential `uint8` fields named `r`, `g`, `b` (point_step shrinks by 1).
- A field with `name = "rgba"`, ... → four sequential `uint8` fields `r`, `g`, `b`, `a` (point_step unchanged, alpha preserved).

The bytes are repacked accordingly. A `SensorBody::PointCloud` registry entry stores the **normalized** schema; readers never see the float-packed layout. Other ROS field quirks pass through unchanged.

---

## Audio Log payload — schema v1

The Audio Log is a separate `auki_logs::Log<AudioLogEntry>`. Each recording is one microphone (or mic array) producing samples over time; the framing's `timestamp_ns` is the **chunk's start time**.

### Manifest

JCS-canonical UTF-8 JSON. Same shape as the Sensor Log manifest (segment_duration_ns, retention_ns, sensor_id, sensor_hash, clock_id, clock_hash). The `(sensor_id, sensor_hash)` pair resolves to a `SensorBody::Microphone` registry entry, which is how a reader knows the segment payloads are `AudioLogEntry`.

### Payload (CBOR)

```
AudioLogEntry {
  data: bytes,    // interleaved samples; encoded as a CBOR byte string
}
```

That's it — no per-chunk metadata. Every byte in `data` is sample data; the chunk's start time is the framing's `timestamp_ns`.

### Sample layout

Samples are **interleaved** per channel. For `channels = N`:

```
[s0_c0, s0_c1, ..., s0_c(N-1), s1_c0, s1_c1, ..., s1_c(N-1), ...]
```

Each sample's encoding is the registry entry's `sample_format`:

| `sample_format` | Bytes per sample | Encoding                      |
|-----------------|------------------|-------------------------------|
| `pcm_s16le`     | 2                | signed 16-bit, little-endian  |
| `pcm_s24le`     | 3                | signed 24-bit, little-endian  |
| `pcm_s32le`     | 4                | signed 32-bit, little-endian  |
| `pcm_f32le`     | 4                | IEEE 754 float32, little-endian |
| `pcm_f64le`     | 8                | IEEE 754 float64, little-endian |

So `data.len() = sample_byte_width × channels × samples_per_chunk`. Chunk size (samples per entry) is the integrator's choice; the SDK does not impose a value. Typical: 10–100 ms of samples per chunk at 48 kHz.

### Why minimal payload

No per-chunk silence flag, no sample count, no sequence number. Sample count is derivable from `data.len()`; silence is detectable by inspecting the bytes; sequencing comes from the framing's `timestamp_ns`. Keeping the payload bare lets compressed formats (FLAC, Opus when added) drop in cleanly — the wrapper structure stays identical, only `sample_format` changes.

### Compressed formats (future)

v1 specifies PCM only. When compression is added, `sample_format` gains values like `flac` or `opus`; `AudioLogEntry.data` carries one compressed packet per chunk; `sample_rate_hz` and `channels` in the registry still describe the decoded stream's properties. The schema doesn't change.

---

## Versioning

Schema version is **1** for all eight types (`SensorRegistryEntry`, `ClockRegistryEntry`, `SensorLogEntry`, `DynamicIntrinsics`, `PointCloudLogEntry`, `PointCloud`/`PointField`, `Microphone`, `AudioLogEntry`). Bump on incompatible field changes. The auki-logs segment format version is independent.
