# Data Products — peer discovery descriptors

> **Status: WIP — working draft, not yet a committed spec.** Schemas, names,
> and lifecycles in this document are subject to change. No code in the SDK
> consumes or produces these structures yet.

## Purpose

A *data product* is one externally addressable thing a node can offer — a camera log, a point cloud log, a TimeTransform Log, a Pose Log, a Detection Log, or the live stream that could be materialized into the same shape later. Peers on the Auki network need to discover what data products a node holds: enough metadata to interpret the payload bytes, align timestamps with their own clock, locate the data in space, and decide whether to fetch.

This document drafts the **descriptor schema** — the serializable shape one peer sends to another to advertise a single data product. The wire transport (gossip, central registry, direct query, signing/trust) is a separate concern that depends on broader Auki Domain/Map architecture and is deliberately out of scope here.

The descriptor's role: pack everything from the local registries + `LogManifest`/`StreamManifest` + log state that a peer would otherwise discover through multiple round-trips, so that one fetch resolves "what is this and how do I use it."

---

## `CameraLogProduct` — schema v1

The first concrete descriptor, for an RGB camera sensor log. The shape generalizes — point clouds get a parallel `PointCloudLogProduct` with the same scaffolding minus camera-specific bits.

```
CameraLogProduct {
  schema_version:  u32,                  // 1
  app_id:          string,               // copied from LogManifest
  session_id:      string,               // copied from LogManifest
  log_id:          string,               // local fetch handle, not semantic identity
  payload_type:    string,               // "auki.camera.PinholeCameraLogEntry"

  // ── Sensor identity ─────────────────────────────────────────────
  // Embedded by value (full registry entry, not just a hash reference)
  // so the peer can interpret bytes without a follow-up registry fetch.
  // The hash fields stay because they're cheap and let receivers cache.
  sensor_id:       string,
  sensor_hash:     string,
  sensor_entry:    SensorRegistryEntry,
                   // For RGB cameras carries:
                   //   width, height, frame_rate_hz,
                   //   pixel_format, color_space,
                   //   intrinsics_model, distortion_model
                   // Dynamic intrinsics, when present, live per-entry
                   // in PinholeCameraLogEntry.dynamic_intrinsics.

  // ── Clock identity ──────────────────────────────────────────────
  // The clock the log's framing timestamps are expressed in.
  clock_id:        string,
  clock_hash:      string,
  clock_entry:     ClockRegistryEntry,

  // ── Spatial frame identity ──────────────────────────────────────
  // The Frame Registry entry for the sample convention
  // (the camera optical frame for an RGB camera).
  // The hash references a specific FrameRegistryEntry under
  // <app_root>/registries/frames/<frame_id>/<hash>.json so a peer can
  // resolve handedness / axes / units before consuming any pose data
  // tagged with this frame.
  frame_id:        string,
  frame_hash:      string,
  frame_entry:     FrameRegistryEntry,
                   // Carries: handedness, axes (x/y/z directions),
                   //          units. Tree structure (parent-child)
                   //          lives in the Pose Log, not here.

  // ── Time-alignment options ──────────────────────────────────────
  // One entry per other clock this node tracks via a TimeTransform Log.
  // Lets a peer pick a bridge clock and fetch the relevant log to
  // convert this log's timestamps into their own clock space.
  time_transforms: [TimeTransformAvailability],

  // ── Spatial-alignment options ───────────────────────────────────
  // One entry per pose chain available for `frame_id` (i.e. how this
  // sensor's mounting frame relates to other frames over time).
  // Pending Pose Log; shape TBD.
  frame_transforms: [FrameTransformAvailability],   // TBD — pending Pose Log

  // ── Log parameters (mirror log_manifest.json) ───────────────────
  segment_duration_ns: i64,
  retention_ns:        i64,              // 0 = unbounded

  // ── Coverage (computed at scan time from segment files) ─────────
  earliest_timestamp_ns: i64,            // first entry in OLDEST RETAINED segment
  latest_timestamp_ns:   i64,            // last entry on disk
  segment_count:         u32,
  total_bytes:           u64,

  // ── Status ──────────────────────────────────────────────────────
  status:           "live" | "sealed" | "aborted",
  generated_at_ns:  i64,                 // wall-clock UTC ns when produced
}
```

### `TimeTransformAvailability`

```
TimeTransformAvailability {
  to_clock_id:           string,
  to_clock_hash:         string,
  to_clock_entry:        ClockRegistryEntry,    // embedded
  log_handle:            string,                // identifier the peer uses to fetch the TimeTransform Log
  earliest_timestamp_ns: i64,
  latest_timestamp_ns:   i64,
  status:                "live" | "sealed" | "aborted",
}
```

### `FrameTransformAvailability`

```
FrameTransformAvailability {
  to_frame_id:           string,
  to_frame_hash:         string,
  to_frame_entry:        FrameRegistryEntry,    // embedded; v0.0.22 shipped
  log_handle:            string,                // e.g. "poselogs/<pose_log_id>" — relative to <session_id>
  earliest_timestamp_ns: i64,
  latest_timestamp_ns:   i64,
  status:                "live" | "sealed" | "aborted",
}
```

The Pose Log capture path uses `Log<auki_datatypes::pose::SpatialTransform>` with identity in `build_pose_log_manifest` (`from_frame_id/hash`, `to_frame_id/hash`, `PoseSource`, `PoseWriterMode`, `expected_rate_hz`) and pathing from `poselog_path`. There is no `PoseLogEntry` wrapper. `auki-geometry` ships convention-level helpers (`convert_pose_convention`, point/vector/direction conversion), while the graph-level `convert_pose` operation that composes pose-log edges across a frame tree is still pending. `FrameTransformAvailability` describes what is available; graph composition is the consumer's job today.

Detection Logs use `Log<auki_datatypes::detection::DetectionLogEntry>`, where `DetectionLogEntry` is opaque bytes. The detector-specific schema is owned by the detector family, not the SDK. The log manifest pins `detector_id`, `detector_hash`, `input_log_id`, `input_sensor_id/hash`, and `clock_id/hash`; a future detector registry can make `detector_hash` resolvable the same way sensor/clock/frame hashes are today.

---

## What the peer gets in one fetch

- **Bytes** — full sensor identity (width, height, format, color space, intrinsics/distortion model) plus the payload type. Per-entry payload fields such as dynamic camera intrinsics stay in the payload stream/log itself.
- **Time** — full clock identity for the log's timestamps.
- **Space** — full frame identity (handedness, axes, units) for the sensor's mounting position.
- **Time bridges** — a menu of TimeTransform Logs to align with the peer's own clock.
- **Space bridges** — a menu of pose chains to align with the peer's own coordinate frame.
- **Coverage** — what time range is on disk, how big.
- **Lifecycle** — live, sealed, or aborted.

Everything required to decide "do I want this, and how do I consume it" without further round-trips against the producing node's registry.

---

## Coverage semantics

- **Bounded** (`retention_ns > 0`): `earliest_timestamp_ns` reflects the *oldest retained* segment's first entry — older content has been evicted. Not the first-ever entry.
- **Unbounded** (`retention_ns = 0`): nothing's been evicted; `earliest_timestamp_ns` is the absolute first.
- **Live** captures: `latest_timestamp_ns` lags wall-clock by up to `segment_duration_ns` (last fsynced entry, not in-flight). `generated_at_ns` lets a peer reason about staleness.

## Status

| Value     | Meaning                                                                  |
|-----------|--------------------------------------------------------------------------|
| `live`    | Writer is still actively appending; descriptor is a snapshot.            |
| `sealed`  | Writer closed cleanly; the recording is final.                           |
| `aborted` | Writer crashed mid-recording (heuristic — see open questions).           |

---

## Concrete example

```json
{
  "schema_version": 1,
  "app_id": "boosterapp",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "log_id": "rec-456",
  "payload_type": "auki.camera.PinholeCameraLogEntry",
  "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
  "sensor_hash": "d798fa879c80a5b00cabc1ce47ca4f7a",
  "sensor_entry": {
    "type": "rgb_camera",
    "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
    "width": 544, "height": 488, "frame_rate_hz": 20,
    "pixel_format": "YUV_NV12", "color_space": "BT.709",
    "intrinsics_model": "pinhole", "distortion_model": "plumb_bob",
    "frame_id": "K1-AABBCCDDEEFF/head_left_cam_optical"
  },
  "clock_id": "K1-AABBCCDDEEFF/utc",
  "clock_hash": "89f84f4c2e09bef81d385b2af1d17e6c",
  "clock_entry": {
    "type": "utc_clock",
    "clock_id": "K1-AABBCCDDEEFF/utc",
    "unit": "milliseconds", "monotonic": false,
    "epoch": "1970-01-01T00:00:00Z", "scope": "global"
  },
  "frame_id": "K1-AABBCCDDEEFF/head_left_cam_optical",
  "frame_hash": "fd0dc3789e898b71b5e16ee122a81a44",
  "frame_entry": {
    "frame_id": "K1-AABBCCDDEEFF/head_left_cam_optical",
    "handedness": "right",
    "axes": {"x": "right", "y": "down", "z": "forward"},
    "units": "meters"
  },
  "time_transforms": [
    {
      "to_clock_id": "K1-AABBCCDDEEFF/monotonic",
      "to_clock_hash": "1f2176888b1a6621315033f22659b9f3",
      "to_clock_entry": {
        "type": "monotonic_clock",
        "clock_id": "K1-AABBCCDDEEFF/monotonic",
        "unit": "milliseconds", "monotonic": true,
        "epoch": null, "scope": "device-local"
      },
      "log_handle": "timetransform_logs/K1-AABBCCDDEEFF__utc__K1-AABBCCDDEEFF__monotonic",
      "earliest_timestamp_ns": 1745000000000000000,
      "latest_timestamp_ns":   1745000030000000000,
      "status": "live"
    }
  ],
  "frame_transforms": [],
  "segment_duration_ns": 1000000000,
  "retention_ns": 30000000000,
  "earliest_timestamp_ns": 1745000000000000000,
  "latest_timestamp_ns":   1745000030000000000,
  "segment_count": 30,
  "total_bytes": 1572864000,
  "status": "live",
  "generated_at_ns": 1745000030500000000
}
```

---

## Open questions

Tracked in the root [`parking_lot.md`](parking_lot.md) under the "Discovery descriptor — …" sections (`log_handle` semantics, aborted-status detection, self-hash) and per-crate parking lots. The Frame Registry shape question is resolved; Pose Log capture shape is resolved; graph-level `convert_pose` path finding and descriptor transport are still pending.

---

## Out of scope (for v1)

- **Wire transport** — gossip vs. Map-mediated central registry vs. direct query. Auki protocol decision.
- **Trust / signing** — descriptor is just bytes; signing/authentication is a wrapper concern.
- **Domain identity / Map endpoint** — the Domain context this node participates in.
- **Connection info for fetching** — URL, peer ID, port. Depends on transport.
- **Multi-product wrappers** (`NodeManifest { products: [...] }`) — a level up; needed eventually but distinct schema.
- **Other product types** — `PointCloudLogProduct`, `TimeTransformLogProduct`, etc. Expected to be parallel to `CameraLogProduct` but designed once this one is locked.
