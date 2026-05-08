# auki-datatypes

Single source of truth for the Auki SDK's **shared cross-language data types** — the typed payload shapes that flow through logs and streams. Owns the `.proto` schemas and the prost-generated Rust code; downstream Rust crates ([`auki-logs`](../auki-logs), [`auki-network`](../auki-network), [`auki-ros-adapter`](../auki-ros-adapter), [`auki-time-transforms`](../auki-time-transforms)) import the generated types from here. Cross-language consumers ([`auki-session-py`](../auki-session-py) via `betterproto`, future Sentinel ports, future iOS/ARKit bindings) generate their own bindings from the same `.proto` files.

The crate name names the **responsibility** (canonical shared data types), not the **implementation** (protobuf via prost). Encoding could change someday; the responsibility doesn't.

This crate exists because **segment payloads on disk are protobuf-encoded** — resolved 2026-05-07; see the [`auki-session-py` parking lot](../auki-session-py/parking_lot.md). Cross-language compatibility (Python, Rust, future ports) requires a shared schema language; protobuf via `.proto` files is that schema.

## Two encodings, each doing what it's good at

| Concern | Encoding | Owned by |
|---|---|---|
| Manifests, registry entries, signing payloads, content-addressed identity | JCS-canonical UTF-8 JSON | [`auki-jcs`](../auki-jcs) |
| Log segment payloads (bulk per-frame data) | Protobuf | this crate |

JCS earns its keep where you need canonical bytes for *signing* and *content-addressed hashing* (manifests, registry entries, `TagClaim` records). Segment payloads are different: written once, read back, never re-encoded for hash comparison. Protobuf's non-canonical-by-default encoding is fine here, and the wire compactness + cross-language schema enforcement are the wins.

The two encodings don't overlap on the wire — different files, different concerns.

## Relationship to `auki-registry`

[`auki-registry`](../auki-registry) holds the canonical **identity catalogs** — Sensor Registry, Frame Registry, Clock Registry — per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0)'s definition: *"a shared, versioned catalog of identities + definitions that other data streams can reference."* Registry entries are JCS-canonical JSON, content-hashed, served verbatim by the Control API.

This crate holds **log payload types** — the typed shapes that flow through `auki-logs` segments (camera frames, point clouds, audio chunks, transforms, time-transform samples). Logs reference registry entries via `(id, hash)` pairs to interpret the bytes; logs and registries are different things and live in different crates.

(Pre-2026-05-07, log payload types were dumped in `auki-registry` for consumer-deps reasons. That was AI drift. Migration to here happens step-by-step as each `.proto` lands — see [`src/sprint.md`](src/sprint.md).)

## Layout

```
auki-datatypes/
├── proto/                       ← .proto schema files (the source of truth)
│   ├── placeholder.proto        ← validates the prost-build pipeline; removed once another package proves it (Step 7)
│   ├── camera.proto             ← auki.camera — PinholeCameraLogEntry + DynamicIntrinsics (Step 1, 2026-05-08)
│   ├── point_cloud.proto        ← auki.point_cloud — PointCloudLogEntry, opaque bytes (Step 3, 2026-05-08; on-disk)
│   ├── frame_stream.proto       ← auki.frame_stream — JpegFrame (Step 2, 2026-05-08; libp2p wire)
│   ├── point_cloud_stream.proto ← auki.point_cloud_stream — PointCloudFrame (Step 2, 2026-05-08; libp2p wire)
│   └── stream.proto             ← auki.stream — StreamMessage envelope (Step 2, 2026-05-08; oneof of Request | Accept | Decline | Frame | EndOfStream)
├── build.rs                     ← invokes prost-build over proto/
├── src/
│   ├── lib.rs                   ← re-exports generated code from OUT_DIR; impl_log_payload! macro; helper constructors on the stream types
│   ├── readme.md                ← implementation status
│   └── sprint.md                ← migration sequence
├── Cargo.toml
├── parking_lot.md
└── changelog.md
```

## Surface

```rust
use auki_datatypes::camera::{DynamicIntrinsics, PinholeCameraLogEntry};   // Step 1 (live)
use auki_datatypes::point_cloud::PointCloudLogEntry;                       // Step 3 (live)
use auki_datatypes::frame_stream::JpegFrame;                               // Step 2 (live)
use auki_datatypes::point_cloud_stream::PointCloudFrame;                   // Step 2 (live)
use auki_datatypes::stream::{                                              // Step 2 (live)
    AcceptInfo, DeclineReason, EndReason, Frame, StreamMessage, StreamRequest,
};
use auki_datatypes::placeholder::PipelineCheck;                            // smoke test (departs Step 7)

// Future, post-migration:
// use auki_datatypes::audio::AudioLogEntry;
// use auki_datatypes::pose::SpatialTransform;
// use auki_datatypes::time_transform::TimeTransformEntry;
```

Each `.proto` package gets a Rust module under `auki_datatypes::`. Generated by `prost-build` at build time; shipped as compiled-in Rust code (no runtime codegen). Every payload type also satisfies [`auki_logs::LogPayload`](../auki-logs/src/lib.rs) via the in-crate `impl_log_payload!` macro — drop one straight into `auki_logs::Log<T>`.

## Cross-language story

The `.proto` files in [`proto/`](proto/) are the canonical schema. Other languages get their own generators:

- **Rust**: `prost-build` (this crate's `build.rs`).
- **Python**: `betterproto` (lands in [`auki-session-py`](../auki-session-py) when that crate's first implementation starts; produces dataclass-shaped Python output).
- **Future ports** (Sentinel, iOS / ARKit, Park's renderer if it gains a Rust/Python core): pick the protobuf generator natural to the language; the `.proto` file is the contract.

**Locked conformance vectors** will live in `tests/locked_vectors.rs` (TBD when the first real `.proto` lands). They pin specific message → wire-bytes pairings; any reimplementation must reproduce these bytes. Same pattern as [`auki-hash`](../auki-hash) / [`auki-identity`](../auki-identity) / [`auki-network`](../auki-network) cross-language conformance vectors.

## Build

```sh
cargo build -p auki-datatypes
cargo test -p auki-datatypes
```

`build.rs` runs every cargo build. The `protoc` binary is supplied by `protoc-bin-vendored` — no system `protoc` install required on dev machines or CI.

## Status

Steps 1, 2, and 3 of the [migration sprint](src/sprint.md) landed 2026-05-08:

- **Step 1** — `auki.camera` carries `PinholeCameraLogEntry` + `DynamicIntrinsics` with locked wire-bytes and hash.
- **Step 2** — `auki.frame_stream { JpegFrame }`, `auki.point_cloud_stream { PointCloudFrame }`, and `auki.stream` (the full envelope `StreamMessage` oneof) are the protobuf wire types that [`auki-network`](../auki-network)'s `/auki/stream/0.1.0` carries.
- **Step 3** — `auki.point_cloud` carries `PointCloudLogEntry { bytes data = 1; }`, opaque-bytes-only. Symmetric with the wire's `PointCloudFrame { bytes }`; ROS-shaped layout fields (`width`, `height`, `is_dense`) are gone — interpretation comes from the `(sensor_id, sensor_hash) → SensorBody::PointCloud` registry entry. Locked wire-bytes vector + XXH3-128 hash + segment-round-trip seam test.

Three on-disk payloads remain (audio, pose, time-transform) plus the `placeholder.proto` smoke-test (goes away at Step 7). See [`src/readme.md`](src/readme.md) for the current state and [`src/sprint.md`](src/sprint.md) for the migration sequence.
