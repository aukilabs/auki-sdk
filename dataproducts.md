# Data Products — peer discovery descriptors

> **Status: WIP — working draft, not yet a committed spec.** Schemas, names,
> and lifecycles in this document are subject to change. No code in the SDK
> consumes or produces these structures yet.

## Purpose

A *data product* is one externally addressable thing a node has captured — a camera log, a point cloud log, a TimeTransform Log, eventually a Pose Log or Detection Log. Peers on the Auki network need to discover what data products a node holds: enough metadata to interpret the bytes, align timestamps with their own clock, locate the data in space, and decide whether to fetch.

This document drafts the **descriptor schema** — the serializable shape one peer sends to another to advertise a single data product. The wire transport (gossip, central registry, direct query, signing/trust) is a separate concern that depends on broader Auki Domain/Map architecture and is deliberately out of scope here.

The descriptor's role: pack everything from the local registry + log state that a peer would otherwise have to discover via multiple round-trips, so that one fetch resolves "what is this and how do I use it."

---

## `CameraLogProduct` — schema v1

The first concrete descriptor, for an RGB camera sensor log. The shape generalizes — point clouds get a parallel `PointCloudLogProduct` with the same scaffolding minus camera-specific bits.

```
CameraLogProduct {
  schema_version:  u32,                  // 1

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

  // ── Clock identity ──────────────────────────────────────────────
  // The clock the log's framing timestamps are expressed in.
  clock_id:        string,
  clock_hash:      string,
  clock_entry:     ClockRegistryEntry,

  // ── Spatial frame identity ──────────────────────────────────────
  // The Frame Registry entry for the sensor's mounting position.
  // Frame Registry is currently pending — `frame_entry`'s shape is
  // TBD until that schema lands.
  frame_id:        string,
  frame_hash:      string,
  frame_entry:     FrameRegistryEntry,   // TBD — pending Frame Registry

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

  // ── Log parameters (mirror the on-disk auki-logs manifest) ──────
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

### `FrameTransformAvailability` *(TBD — pending Pose Log)*

```
FrameTransformAvailability {
  to_frame_id:           string,
  to_frame_hash:         string,
  to_frame_entry:        FrameRegistryEntry,    // embedded
  log_handle:            string,
  earliest_timestamp_ns: i64,
  latest_timestamp_ns:   i64,
  status:                "live" | "sealed" | "aborted",
}
```

---

## What the peer gets in one fetch

- **Bytes** — full sensor identity (width, height, format, color space, intrinsics/distortion model).
- **Time** — full clock identity for the log's timestamps.
- **Space** — full frame identity for the sensor's mounting position (once Frame Registry exists).
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
  "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
  "sensor_hash": "e8cb3879fcfa7f716047aa0892b0c0c0",
  "sensor_entry": {
    "type": "rgb_camera",
    "sensor_id": "K1-AABBCCDDEEFF/head_left_cam",
    "width": 544, "height": 488, "frame_rate_hz": 20,
    "pixel_format": "YUV_NV12", "color_space": "BT.709",
    "intrinsics_model": "pinhole", "distortion_model": "plumb_bob"
  },
  "clock_id": "K1-AABBCCDDEEFF/utc",
  "clock_hash": "89f84f4c2e09bef81d385b2af1d17e6c",
  "clock_entry": {
    "type": "utc_clock",
    "clock_id": "K1-AABBCCDDEEFF/utc",
    "unit": "milliseconds", "monotonic": false,
    "epoch": "1970-01-01T00:00:00Z", "scope": "global"
  },
  "frame_id": "K1-AABBCCDDEEFF/head_left_cam_frame",
  "frame_hash": "...",
  "frame_entry": "<TBD — pending Frame Registry>",
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

## Pending / open questions

- **Frame Registry schema.** `frame_entry` and `frame_transforms[].to_frame_entry` need a concrete shape. Blocks finalization of this descriptor.
- **Pose Log shape.** `frame_transforms[].log_handle` only resolves to something fetchable once Pose Log exists.
- **`log_handle` semantics.** What's the actual handle? `(sensor_id, sensor_hash)` pair? URL relative to a node base? Peer-ID-prefixed path? Depends on wire-protocol decisions.
- **Aborted-status detection.** The on-disk format doesn't currently mark clean-close vs. crash. Heuristic: "no recent updates AND no sealed marker." Worth pinning before we rely on it.
- **Self-hash of the descriptor.** Would let peers cache by descriptor identity, but adds a chicken-and-egg with `generated_at_ns`. Skip for v1.

---

## Out of scope (for v1)

- **Wire transport** — gossip vs. Map-mediated central registry vs. direct query. Auki protocol decision.
- **Trust / signing** — descriptor is just bytes; signing/authentication is a wrapper concern.
- **Domain identity / Map endpoint** — the Domain context this node participates in.
- **Connection info for fetching** — URL, peer ID, port. Depends on transport.
- **Multi-product wrappers** (`NodeManifest { products: [...] }`) — a level up; needed eventually but distinct schema.
- **Other product types** — `PointCloudLogProduct`, `TimeTransformLogProduct`, etc. Expected to be parallel to `CameraLogProduct` but designed once this one is locked.
