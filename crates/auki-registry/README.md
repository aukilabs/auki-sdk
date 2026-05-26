# auki-registry

The Auki SDK's **identity catalog** — content-addressed Sensor / Frame / Clock / Detector registry entries plus the cross-language storage contract that backs them. Per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0): *"a shared, versioned catalog of identities + definitions that other data streams can reference without repeating metadata."*

> **Scope shrink complete (decided 2026-05-07).** This crate is back to its canonical role: identity catalogs only. Log payload types live in generated [`auki-proto`](../auki-proto) modules sourced from root [`proto/auki`](../../proto/auki); `auki-registry` holds the Sensor / Clock / Frame / Detector registry types and IO only.

## Two kinds of typed data (today, with one departing)

| Kind            | What it is                             | Where it lives                                          | Future home                          |
| --------------- | -------------------------------------- | ------------------------------------------------------- | ------------------------------------ |
| Registry entry  | Immutable identity, one per hash       | `<app_root>/registries/<kind>/<id>/<hash>.json`         | Stays in `auki-registry` (canonical) |
| Log payload     | Per-frame mutable data                  | Inside an [`auki-logs`](../auki-logs) segment, protobuf encoded | Lives in [`auki-proto`](../auki-proto) |

Registry entries describe **what a thing is**; log payloads describe **what was sampled at a moment**. The split is right — the AI-drift was placing both halves in the same crate. The split itself stays; only the location of the second half changes.

All log payload types now live in [`auki-proto`](../auki-proto), protobuf via prost.

---

## Registry entries

### Storage layout

```
<app_root>/registries/sensors/<sensor_id>/<hash>.json
<app_root>/registries/clocks/<clock_id>/<hash>.json
<app_root>/registries/frames/<frame_id>/<hash>.json
<app_root>/registries/detectors/<detector_id>/<hash>.json
```

Registries live at the **app root**, shared across every session of that app. Hash-keyed writes are idempotent, so a sensor entry that doesn't change between app starts produces the same `<hash>.json` regardless of session — re-writing it would be wasted work.

The full session shape (registries + per-session log directories) is documented in [`auki-layout`](../auki-layout).

`/` in IDs is replaced by `__` so namespaced IDs like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment.

The hash *is* the version. Re-writing identical content is a no-op; writing different content under the same `id` produces a sibling file with a different hash. There are no version counters.

Hashes come from [`auki-hash`](../auki-hash) over the JCS-canonical bytes from [`auki-jcs`](../auki-jcs). All three crates form one indivisible content-addressing contract.

### Generated bindings

The crate follows the SDK binding standard:

- `src/core.rs` owns the typed Rust API and on-disk storage implementation.
- `src/ffi.rs` exposes native UniFFI adapters for Python and Swift.
- `src/wasm.rs` exposes web-safe wasm-bindgen adapters for JavaScript/WebAssembly.

Generated languages pass registry entries as JSON strings. The adapters parse
those strings into the same Rust entry types, then return JCS-canonical JSON or
XXH3-128 hashes. Native UniFFI also exposes `write_*_entry_json` and
`read_*_entry_json` helpers for filesystem storage. The wasm surface deliberately
does not export filesystem read/write helpers; browser storage needs a real
adapter before it belongs in this crate.

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
  type:      string,        // tagged-enum discriminant: "camera" | "point_cloud" | "audio" | "joint_encoders"; future: "depth", "imu", "lidar"
  ...body fields per type...
}
```

When `type = "camera"`:

```
Camera {
  width:             u32,
  height:            u32,
  frame_rate_hz:     u32,
  pixel_format:      string,    // e.g. "YUV_NV12", "RGB8"
  color_space:       string,    // e.g. "BT.709", "sRGB"
  intrinsics_model:  string,    // e.g. "pinhole"
  distortion_model:  string,    // e.g. "plumb_bob", "none"
  frame_id:          string,    // Frame Registry id for the camera optical frame
  frame_hash:        string,    // exact FrameRegistryEntry hash for that frame_id
}
```

When `type = "point_cloud"`:

```
PointCloud {
  fields:        [PointField],   // byte layout of one point
  point_step:    u32,            // bytes per point
  is_bigendian:  bool,           // byte order of multi-byte fields in `data`
  frame_rate_hz: u32,
  frame_id:      string,         // Frame Registry id for the point coordinates
  frame_hash:    string,         // exact FrameRegistryEntry hash for that frame_id
}

PointField {
  name:     string,              // "x", "y", "z", "r", "g", "b", "a", "intensity", "ring", "t", ...
  offset:   u32,                 // byte offset within one point
  datatype: string,              // "int8" | "uint8" | "int16" | "uint16" | "int32" | "uint32" | "float32" | "float64"
  count:    u32,                 // number of `datatype` elements; usually 1
}
```

When `type = "audio"`:

```
Audio {
  sample_rate_hz: u32,           // e.g. 48000
  channels:       u32,           // 1 mono, 2 stereo, N for arrays
  sample_format:  string,        // "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le"
                                 //   (raw PCM in v1; compressed formats added by extending this string)
  channel_layout: string,        // "mono" | "stereo" | "5.1" | "7.1" | "ambisonic_b" | "n_channel"
}
```

Renamed from `Microphone` 2026-05-14 — signal-type naming for consistency with `PointCloud` / `JointEncoders`.

**Multi-microphone arrays are modelled as one sensor with `channels = N`,** not as N independent sensors. The right shape for physically-synchronized arrays where all channels share a clock and a beam-forming origin (K1 head array, MacBook beamformer). Use separate `SensorRegistryEntry` records only when mics are physically independent capture devices.

The tagged-enum body is the extension point for future sensor types.

Spatial sensor bodies (`Camera` and `PointCloud`) must pin an exact frame
convention with both `frame_id` and `frame_hash`. `write_sensor` validates that
`<app_root>/registries/frames/<frame_id>/<frame_hash>.json` exists before
writing the sensor entry. Editing a `FrameRegistryEntry` therefore produces a
new frame hash, and producers that re-write against it get a new sensor hash as
well; consumers do not silently reinterpret old sensor entries against new axes.

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

## Sensor Log payload — in `auki-proto`

The Sensor Log payload (`CameraFrame`) and `DynamicIntrinsics` live in [`auki-proto`](../auki-proto) under the `auki.camera` `.proto` package. Encoding is protobuf via prost. The split between static-registry-side and dynamic-per-frame intrinsics survives the move; the rationale (registry hash stability vs. autofocus drift) carried over verbatim. Manifest shape is unchanged — same `(sensor_id, sensor_hash)` resolution against the Sensor Registry tells a reader the segments hold `CameraFrame` rather than another payload type.

---

## Point Cloud Log payload — in `auki-proto`

`PointCloudLogEntry` lives in [`auki-proto`](../auki-proto) under the `auki.point_cloud` `.proto` package, encoded as protobuf via prost. The payload is **opaque-bytes-only** — `PointCloudLogEntry { bytes data = 1; }` — symmetric with the wire's `PointCloudFrame`. The pre-migration ROS-shaped fields `width` / `height` / `is_dense` are gone from the per-frame entry; readers resolve them via the `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id, frame_hash }` chain that already governs interpretation.

The manifest shape is unchanged — same `(sensor_id, sensor_hash)` against the Sensor Registry tells a reader the segments hold `PointCloudLogEntry`. Capturing camera + point cloud simultaneously is still two parallel sensor logs sharing a session.

### RGB(A) normalization (still here)

ROS2's `sensor_msgs/PointCloud2` historically packs RGB into a `float32` whose 4 bytes are interpreted as `0x00RRGGBB` (or `0xAARRGGBB` for `rgba`). The translation layer in [`auki-ros-adapter`](../auki-ros-adapter) **normalizes** this before the bytes ever reach the SDK:

- A field with `name = "rgb"`, `datatype = float32`, `count = 1` → three sequential `uint8` fields named `r`, `g`, `b` (point_step shrinks by 1).
- A field with `name = "rgba"`, ... → four sequential `uint8` fields `r`, `g`, `b`, `a` (point_step unchanged, alpha preserved).

The bytes in the segment are the repacked layout; a `SensorBody::PointCloud` registry entry stores the **normalized** schema. Readers never see the float-packed layout. Other ROS field quirks pass through unchanged.

---

## Audio Log payload — in `auki-proto`

`AudioLogEntry` lives in [`auki-proto`](../auki-proto) under the `auki.audio` `.proto` package, encoded as protobuf via prost. The payload is **opaque-bytes-only** — `AudioLogEntry { bytes data = 1; }` — same stance as point clouds. Sample count and chunk duration are both derivable from the bytes plus the SensorRegistryEntry's `Audio { sample_format, channels, sample_rate_hz }` body, and denormalizing a derivable field would risk inconsistency for marginal reader convenience.

The manifest shape is unchanged — same `(sensor_id, sensor_hash)` against the Sensor Registry tells a reader the segments hold `AudioLogEntry`. Sample-layout semantics (interleaved per channel; encoding per the registry's `sample_format`; compressed formats drop in cleanly via new `sample_format` values without changing the wrapper) carried over verbatim.

---

## Pose Log payload — in `auki-proto`

The Pose Log payload lives in [`auki-proto`](../auki-proto) under the `auki.pose` `.proto` package, encoded as protobuf via prost. The pre-migration `PoseLogEntry { transforms: Vec<TransformSample> }` wrapper is gone, and per-sample `parent_frame`/`child_frame` strings are gone too. The segment entry is flat `SpatialTransform { Vec3 translation; Quat orientation }`; frame identity lives in the manifest's `(from_frame_id, to_frame_id)` pair, mirroring how TimeTransform Log already keys per `(from_clock_id, to_clock_id)`.

A producer that observes a multi-pair ROS `TFMessage` is responsible for fanning the message into N parallel pose logs (one per `(from, to)` pair). Each log has stable identity over its lifetime; the manifest carries `(from_frame_id + from_frame_hash)`, `(to_frame_id + to_frame_hash)`, `clock_id + clock_hash`, the inline `PoseSource` provenance tag, plus a `writer_mode: "rigid" | "movable"` hint and an `expected_rate_hz` rate hint per the synthesis. See [`auki-proto/README.md`](../auki-proto/README.md) for the segment payload and [`auki-manifests/README.md`](../auki-manifests/README.md) for the manifest builder.

The on-disk pose-log directory is `<session>/poselogs/<from_id>__<to_id>/` (each frame_id's `/` substituted to `__` per the same convention as `timetransform_log_path`).

`convert_pose` itself is still pending — capture and read are in place; composition / path-finding lands separately.

### Frame Registry

Each `from_frame_id` / `to_frame_id` in a Pose Log manifest references an entry in the Frame Registry — a sibling to the Sensor and Clock registries that describes what each named frame's coordinate convention is (handedness, axis directions, length unit). The registry shipped in v0.0.22 with `FrameRegistryEntry { frame_id, handedness, axes, units }` plus four preset constructors covering REP-103 body, REP-103 optical, OpenGL/Three.js, and Unity. Tree structure (frame parentage) lives in the Pose Log manifests' `(from, to)` pairs, not in the registry — the registry declares what each frame *is in isolation*; the manifests declare the edges between them. Rotation representation (Hamilton quaternion `(x, y, z, w)`) is fixed at the `SpatialTransform` layer; not per-frame. See [`src/readme.md`](src/readme.md#frameregistryentry) for the full type definitions.

---

## Versioning

Schema version is **1** for all types in this crate today (`SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`, `PointCloud`/`PointField`, `Audio`). `PoseSource` (now in [`auki-manifests`](../auki-manifests)) and the on-disk log payload types (`CameraFrame` / `DynamicIntrinsics` / `PointCloudLogEntry` / `AudioLogEntry` / `SpatialTransform` / `Vec3` / `Quat`, all now in [`auki-proto`](../auki-proto)) version independently. Bump on incompatible field changes. The auki-logs segment format version is independent of all of these.
