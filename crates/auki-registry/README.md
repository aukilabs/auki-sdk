# auki-registry

The Auki SDK's **identity catalog** — content-addressed Sensor / Frame / Clock registry entries plus the cross-language storage contract that backs them. Per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0): *"a shared, versioned catalog of identities + definitions that other data streams can reference without repeating metadata."*

> **Scope shrink in flight (decided 2026-05-07).** This crate currently *also* holds log payload types (`PoseLogEntry`, `TransformSample`). That's AI drift — those types are payloads, not identity, and don't fit the canonical registry definition. They're migrating step-by-step into [`auki-datatypes`](../auki-datatypes) (where they get renamed and protobuf-encoded along the way: `TransformSample` → `SpatialTransform`, `PoseLogEntry` wrapper goes away). Sequence and per-step decisions live in [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md). **Steps 1, 3, and 4 (2026-05-08) are complete** — `PinholeCameraLogEntry` + `DynamicIntrinsics` (Step 1), `PointCloudLogEntry` (Step 3, opaque-bytes-only), and `AudioLogEntry` (Step 4, opaque-bytes-only) live in [`auki-datatypes`](../auki-datatypes). Everything below this line describes today's state — including the AI-drift parts, marked as such.

## Two kinds of typed data (today, with one departing)

| Kind            | What it is                             | Where it lives                                          | Future home                          |
| --------------- | -------------------------------------- | ------------------------------------------------------- | ------------------------------------ |
| Registry entry  | Immutable identity, one per hash       | `<app_root>/registries/<kind>/<id>/<hash>.json`         | Stays in `auki-registry` (canonical) |
| Log payload     | Per-frame mutable data — **AI-drift**  | Inside an [`auki-logs`](../auki-logs) segment, mixed encoding mid-migration | Moves to [`auki-datatypes`](../auki-datatypes) (protobuf) |

Registry entries describe **what a thing is**; log payloads describe **what was sampled at a moment**. The split is right — the AI-drift was placing both halves in the same crate. The split itself stays; only the location of the second half changes.

The camera payload (`PinholeCameraLogEntry` + `DynamicIntrinsics`) moved at Step 1 (2026-05-08); `PointCloudLogEntry` moved at Step 3 (2026-05-08, opaque-bytes-only); `AudioLogEntry` moved at Step 4 (2026-05-08, opaque-bytes-only) — all three are now protobuf via prost. The remaining payloads (`PoseLogEntry`, `TransformSample`) are still here, still CBOR, until Step 5.

---

## Registry entries

### Storage layout

```
<app_root>/registries/sensors/<sensor_id>/<hash>.json
<app_root>/registries/clocks/<clock_id>/<hash>.json
<app_root>/registries/frames/<frame_id>/<hash>.json
```

Registries live at the **app root**, shared across every session of that app. Hash-keyed writes are idempotent, so a sensor entry that doesn't change between app starts produces the same `<hash>.json` regardless of session — re-writing it would be wasted work.

The full session shape (registries + per-session log directories) is documented in [`auki-layout`](../auki-layout).

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

## Sensor Log payload — moved to `auki-datatypes` (Step 1, 2026-05-08)

The Sensor Log payload (renamed `PinholeCameraLogEntry`) and `DynamicIntrinsics` now live in [`auki-datatypes`](../auki-datatypes) under the `auki.camera` `.proto` package. Encoding switched from CBOR to protobuf via prost. The split between static-registry-side and dynamic-per-frame intrinsics survives the move; the rationale (registry hash stability vs. autofocus drift) carried over verbatim. Manifest shape is unchanged — same `(sensor_id, sensor_hash)` resolution against the Sensor Registry tells a reader the segments hold `PinholeCameraLogEntry` rather than another payload type. See [`auki-datatypes/README.md`](../auki-datatypes/README.md) for the current shape.

---

## Point Cloud Log payload — moved to `auki-datatypes` (Step 3, 2026-05-08)

`PointCloudLogEntry` now lives in [`auki-datatypes`](../auki-datatypes) under the `auki.point_cloud` `.proto` package, encoded as protobuf via prost. The Step 3 decision was **opaque-bytes-only** — `PointCloudLogEntry { bytes data = 1; }` — symmetric with the wire's `PointCloudFrame`. The pre-migration ROS-shaped fields `width` / `height` / `is_dense` are gone from the per-frame entry; readers resolve them via the `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }` chain that already governs interpretation. See [`auki-datatypes/README.md`](../auki-datatypes/README.md) for the current shape.

The manifest shape is unchanged — same `(sensor_id, sensor_hash)` against the Sensor Registry tells a reader the segments hold `PointCloudLogEntry`. Capturing camera + point cloud simultaneously is still two parallel sensor logs sharing a session.

### RGB(A) normalization (still here)

ROS2's `sensor_msgs/PointCloud2` historically packs RGB into a `float32` whose 4 bytes are interpreted as `0x00RRGGBB` (or `0xAARRGGBB` for `rgba`). The translation layer in [`auki-ros-adapter`](../auki-ros-adapter) **normalizes** this before the bytes ever reach the SDK:

- A field with `name = "rgb"`, `datatype = float32`, `count = 1` → three sequential `uint8` fields named `r`, `g`, `b` (point_step shrinks by 1).
- A field with `name = "rgba"`, ... → four sequential `uint8` fields `r`, `g`, `b`, `a` (point_step unchanged, alpha preserved).

The bytes in the segment are the repacked layout; a `SensorBody::PointCloud` registry entry stores the **normalized** schema. Readers never see the float-packed layout. Other ROS field quirks pass through unchanged.

---

## Audio Log payload — moved to `auki-datatypes` (Step 4, 2026-05-08)

`AudioLogEntry` now lives in [`auki-datatypes`](../auki-datatypes) under the `auki.audio` `.proto` package, encoded as protobuf via prost. The Step 4 decision was **opaque-bytes-only** — `AudioLogEntry { bytes data = 1; }` — same stance as Step 3 for point clouds. The pre-Step-3 sprint lean toward adding a typed `sample_count: u32` was declined: sample count and chunk duration are both derivable from the bytes plus the SensorRegistryEntry's `Microphone { sample_format, channels, sample_rate_hz }` body, and denormalizing a derivable field would risk inconsistency for marginal reader convenience. See [`auki-datatypes/README.md`](../auki-datatypes/README.md) for the current shape.

The manifest shape is unchanged — same `(sensor_id, sensor_hash)` against the Sensor Registry tells a reader the segments hold `AudioLogEntry`. Sample-layout semantics (interleaved per channel; encoding per the registry's `sample_format`; compressed formats drop in cleanly via new `sample_format` values without changing the wrapper) carried over verbatim.

---

## Pose Log payload — schema v1

> *Departing — moves to [`auki-datatypes`](../auki-datatypes) as flat `SpatialTransform` segment entries (protobuf-encoded). The `PoseLogEntry` wrapper goes away; `from` / `to` move to manifest identity (per-`(from, to)` log instead of per-producer). See [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md) step 5 and [root `parking_lot.md`](../../parking_lot.md) "Pose Log capture shape" Propagate task.*

The Pose Log is an `auki_logs::Log<PoseLogEntry>`. The framing's `timestamp_ns` is the message arrival time on the daemon's clock; the payload here carries one batch of transforms (one ROS `TFMessage`, or the equivalent for other producers).

Pose Log answers "where was this frame relative to that one over time" — eventually consumed by the future `convert_pose` operation. v1 covers capture and read; composition / path-finding / `convert_pose` lands separately.

Pose Log directories live at `<session>/poselogs/<pose_log_id>/` — same parallel-log shape as Sensor Logs (multiple logs per session, distinguished only by their manifest's `retention_ns`). Multiple logs of the same source are fine; the integrator decides when to start and stop.

### Manifest

JCS-canonical UTF-8 JSON. Required keys (extends auki-logs's base):

| Key                    | Type    | Notes                                                            |
| ---------------------- | ------- | ---------------------------------------------------------------- |
| `segment_duration_ns`  | integer | > 0; from auki-logs                                              |
| `retention_ns`         | integer | ≥ 0; from auki-logs (0 = unbounded)                              |
| `app_id`               | string  | Application identifier (same as the daemon's `/api/info` `app`). |
| `session_id`           | string  | UUIDv4 minted by the integrator at app boot.                     |
| `clock_id`             | string  | The Clock Registry ID that the framing's `timestamp_ns` is in    |
| `clock_hash`           | string  | XXH3-128 hex of the clock's registry entry                       |
| `source`               | object  | The producer identity, inline (see below). Tagged-enum body.     |

**`source` is inline, not a registry reference.** Pose Log has no Pose Source Registry — payload is fully self-describing (frame names sit in each `TransformSample`), so producer identity is provenance, not a decoder. Sensor Log earns a registry because its byte payload is uninterpretable without one; Pose Log doesn't have that pressure. If/when a producer variant brings substantial identity (SLAM with `map_id` + algorithm parameters), graduating `source` to a sibling registry is straightforward — extract the body into a content-addressed JSON file and replace the inline value with `(source_id, source_hash)`.

### `PoseSource` — moved to [`auki-manifests`](../auki-manifests)

The inline producer-identity tagged enum (`PoseSource`) and the `build_pose_log_manifest` builder moved to [`auki-manifests`](../auki-manifests) in Step 0 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) (2026-05-08). They're manifest concerns, not registry concerns. See [`auki-manifests/README.md`](../auki-manifests/README.md) for the current shape and the rationale.

### Payload (CBOR)

```
PoseLogEntry {
  transforms: [TransformSample],   // empty allowed
}

TransformSample {
  parent_frame:  string,
  child_frame:   string,
  translation:   [f64; 3],          // x, y, z (typically meters)
  rotation_quat: [f64; 4],          // x, y, z, w (Hamilton convention; matches ROS)
}
```

`f64` matches ROS `geometry_msgs` and preserves precision at large-map scales. Translation and rotation are CBOR arrays (not maps) for half the byte cost; the ordering is documented and stable.

There is no per-`TransformSample` timestamp in the payload — the auki-logs framing's `timestamp_ns` is the message arrival time on the daemon's clock. ROS TF carries per-`TransformStamped` `stamp` fields too, but for the rendering use case the message arrival is what consumers need to step through. If downstream needs per-transform stamps, add them additively without breaking the existing shape.

### Frame Registry

Each `parent_frame` / `child_frame` string in a `TransformSample` references an entry in the Frame Registry — a sibling to the Sensor and Clock registries that describes what each named frame's coordinate convention is (handedness, axis directions, length unit). The registry shipped in v0.0.22 with `FrameRegistryEntry { frame_id, handedness, axes, units }` plus four preset constructors covering REP-103 body, REP-103 optical, OpenGL/Three.js, and Unity. Tree structure (frame parentage) lives in the Pose Log, not in the registry — the registry declares what each frame *is in isolation*; the Pose Log declares the edges between them. Rotation representation (Hamilton quaternion `[x, y, z, w]`) is fixed at the `TransformSample` layer; not per-frame. See [`src/readme.md`](src/readme.md#frameregistryentry) for the full type definitions.

---

## Versioning

Schema version is **1** for all types in this crate today (`SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `PointCloud`/`PointField`, `Microphone`, `PoseLogEntry`, `TransformSample`). `PoseSource` (now in [`auki-manifests`](../auki-manifests)) and `PinholeCameraLogEntry` / `DynamicIntrinsics` / `PointCloudLogEntry` / `AudioLogEntry` (now in [`auki-datatypes`](../auki-datatypes)) version independently. Bump on incompatible field changes. The auki-logs segment format version is independent of all of these.
