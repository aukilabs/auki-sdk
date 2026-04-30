# Auki SDK

The on-device SDK for the Auki **real world web** — a collaborative spatial computing protocol that lets devices, robots, and services share, locate, and reason about the physical world.

> *The internet made information browsable. The Auki network makes the physical world browsable.*

---

## What this SDK is for

Each physical space — a warehouse, a hospital ward, a retail floor — is a **Domain**: a named coordinate system owned and operated by whoever built it. Domains are to the real world what websites are to the internet. They're privately owned, independently operated, and linked together by a shared protocol. Operationally, a Domain is a scene graph: typed nodes (frames, sensors, clocks) connected by transform edges, served by a designated Map.

The Auki protocol exists so any node — a phone, a robot, a cloud server, a browser tab — can answer four questions about any other node:

- **Where is this?**
- **When was this?**
- **How can I talk to you?**
- **How can I compensate you?**

This SDK is the on-device library that lets a node participate. It captures sensor data with the right identity and timing metadata, maintains a local scene graph, and (in time) crosses domain boundaries through the protocol's transform-composition operations.

---

## Spatio-temporal reasoning

A node's local scene graph is the combined output of **four append-only logs** and **three registries**. Together they answer "where was X at time *t*" through composition along a transform path:

```
T_X_session(t) = T_body_session(t) ∘ T_X_body(t)
```

Each transform edge is looked up or interpolated at time *t* in a Pose Log; the chain is composed into the answer the caller asked for.

### The four logs

| Log | What it stores |
|---|---|
| **Pose Log** | Timestamped `from → to` transforms — answers "where was this frame over time" |
| **Sensor Log** | Per-frame sensor payloads (camera frames, point clouds, IMU samples), keyed to a sensor and a clock |
| **Detection Log** | Per-frame detection outputs from extractors |
| **TimeTransform Log** | Sampled offsets between two clocks — answers "given a timestamp on clock A, what's the equivalent on clock B" |

### The three registries

| Registry | What it holds |
|---|---|
| **Frame Registry** | Coordinate convention metadata for named frames (handedness, axes, units, rotation semantics) |
| **Sensor Registry** | Per-sensor identity and interpretation metadata (type, dimensions, format, sample rate) |
| **Clock Registry** | Per-clock identity and semantics (monotonic vs UTC, scope, epoch) |

Logs reference registries by **content-addressed hash**: the hash IS the version. Refining a registry entry is a sibling-write under the same id; downstream consumers pin a specific hash deliberately.

---

## What's implemented today

This repo is in early development. The crates here implement a foundational subset of the architecture:

| Crate | Status |
|---|---|
| [`auki-logs`](crates/auki-logs) | ✓ Generic segmented ring-buffer log primitive — manifest + segment files + retention eviction |
| [`auki-registry`](crates/auki-registry) | ✓ Sensor + Clock registry types and IO. Frame Registry pending |
| [`auki-jcs`](crates/auki-jcs) | ✓ RFC 8785 JSON canonicalization (used for stable hashing of registry entries) |
| [`auki-hash`](crates/auki-hash) | ✓ XXH3-128 wrapper used for registry content-addressing |
| [`auki-time-transforms`](crates/auki-time-transforms) | ✓ Clock sampler primitives for the TimeTransform Log |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ⚠ Generic ROS2 → SDK glue. Currently broken: `r2r` 0.9.5's compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout some camera drivers publish. Fix in flight |

**Not yet implemented:**

- Pose Log and `convert_pose`
- Detection Log
- Frame Registry
- `convert_time` (the TimeTransform Log primitives exist; the `convert_time` operation that consumes them does not yet)
- Domain registration / clustering / discovery (these live in higher layers; the SDK provides primitives, not the network protocol itself)
- Python bindings (planned)

---

## On-disk format

Logs and registries write to a documented binary + JSON format. Each format spec lives with the crate that owns it — they're the source of truth for any reader, in any language:

- [`auki-logs`](crates/auki-logs/README.md) — segmented ring-buffer log layout (used by both Sensor and TimeTransform Logs)
- [`auki-registry`](crates/auki-registry/README.md) — registry entry storage layout, plus the Sensor Log payload schema and the ROS2 → SDK field mapping
- [`auki-time-transforms`](crates/auki-time-transforms/README.md) — TimeTransform Log payload schema and sampling protocol

Files within a session:

```
<session>/
├── registry/
│   ├── sensors/<sensor_id>/<hash>.json
│   ├── clocks/<clock_id>/<hash>.json
│   └── frames/<frame_id>/<hash>.json        ← coming
└── logs/
    ├── sensors/<sensor_id>/
    │   ├── manifest.json
    │   └── segments/<padded-ns>.seg
    └── time_transforms/<from_id>/<to_id>/
        ├── manifest.json
        └── segments/<padded-ns>.seg
```

---

## Quickstart

Add a SDK crate as a Git dependency in your `Cargo.toml`:

```toml
[dependencies]
auki-logs = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.1" }
auki-registry = { git = "https://github.com/aukilabs/auki-sdk", tag = "v0.0.1" }
```

Open a sensor log:

```rust
use std::path::Path;
use auki_logs::Log;

let manifest = serde_json::json!({
    "segment_duration_ns": 1_000_000_000_i64,
    "retention_ns":        30_000_000_000_i64,
    "sensor_id":           "K1-AABBCCDDEEFF/head_left_cam",
    "sensor_hash":         "abc...",
    "clock_id":            "K1-AABBCCDDEEFF/utc",
    "clock_hash":          "def...",
});

let mut log: Log<MyPayload> = Log::open(Path::new("session/logs/sensors/cam"), manifest)?;
log.append(timestamp_ns, &payload)?;
```

See each crate's `README.md` for the contract spec, and `crates/<name>/src/README.md` for the Rust-side implementation status.

---

## Design and discussion

The SDK's design is being worked through across two spaces:

- **Long-form design docs** live in Notion under [About the real world web](https://www.notion.so/3495c8e9659280b38cc9ce1540f72a29). These cover the broader Auki protocol, the four questions, the registries and logs, and the Domain/Cluster/Map architecture.
- **Open architectural questions** live in the [SDK Parking lot](https://www.notion.so/3495c8e9659280f5b0eafc6e4f70898b) — registry vs. manifest, frame pairing, schema versioning, etc.

Notion is the source of truth for design conversations; this repo is the source of truth for the implementation.

---

## Status

The SDK is **0.x and changing**. Schema and API are pre-stable; breaking changes are expected and will tick the minor version pre-1.0. Stable points are tagged on Git (`v0.0.1`, `v0.0.2`, …); downstream consumers should pin a tag.

The first downstream consumers are:

- **`booster-app`** — the on-robot application running the SDK on a Booster K1 humanoid
- **`park`** — an open-source visualization tool for Reid sessions, eventually a real-time domain viewer

---

## Contributing

Issues, design questions, and contributions are welcome via [GitHub issues](https://github.com/aukilabs/auki-sdk/issues). Bigger architectural questions are best raised in the Notion [SDK Parking lot](https://www.notion.so/3495c8e9659280f5b0eafc6e4f70898b) so they're visible to the whole team.

---

## License

MIT. See [LICENSE](LICENSE).

Copyright © 2026 Auki Labs Limited.
