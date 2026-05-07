# Parking lot — auki-datatypes

Open questions for the `auki-datatypes` crate. Cross-cutting questions that involve other crates live in the [root `parking_lot.md`](../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../CLAUDE.md) for the workflow.

---

## `.proto` package naming convention

Placeholder uses `package auki.placeholder;` (dot-separated, lowercase). Working candidates for real packages: `auki.camera` (PinholeCameraLogEntry), `auki.point_cloud` (PointCloudLogEntry), `auki.audio` (AudioLogEntry), `auki.pose` (SpatialTransform), `auki.time_transform` (TimeTransformEntry), `auki.frame_stream` (JpegFrame for libp2p), `auki.point_cloud_stream` (PointCloudFrame for libp2p). The dotted form maps to a Rust nested module path (`auki_datatypes::camera` via the `include!` in `src/lib.rs`), which is fine. Lean: snake_case singular nouns under the `auki.` umbrella, one package per logical message group. Pin before any real `.proto` lands so we don't end up with mixed styles. Open: should `JpegFrame` / `PointCloudFrame` (libp2p stream wire types) share packages with their on-disk siblings (`PinholeCameraLogEntry` / `PointCloudLogEntry`), or get their own packages? Lean: separate packages — different consumers, different evolution rates.

## Field number allocation strategy

Protobuf field numbers are forever — once a field gets a number, it can never be reused for a different field, and renumbering is a breaking change. Lean: keep a comment block at the top of each `.proto` file listing reserved numbers with a brief explanation, the way the protobuf-best-practices community recommends. Pin a convention before the first real schema.

## Locked conformance vector format — JSON or binary?

`auki-hash` / `auki-identity` / `auki-network` locked vectors pin specific input → specific output bytes (hex-encoded). For protobuf messages, the input is a structured message (no canonical text representation by default — protobuf is a binary format with multiple equivalent JSON projections). Options: (a) pin a Rust struct literal in a Rust test → encode → assert hex bytes, (b) pin a JSON file representing the message → load → encode → assert hex bytes, (c) pin both a deterministic Python encoding and a Rust encoding and assert they match. Lean: (a) for round-trip + determinism tests, (c) for the cross-language conformance dimension.

## Schema versioning — when to bump major

Protobuf wire format gives forward/backward compat almost for free (optional fields, unknown field handling). Most schema changes don't need version bumps. The exceptions are renaming a message, removing a required field, changing a field's type or number — those are breaking. Convention to pin: when does the SDK release tag bump? Lean: per-`.proto` major/minor in a header comment, surfaced via the registry entry's `sensor_hash` (which already pins the schema version transitively). No need for a separate "proto version" knob.

---

## Per-type slop fixes (surfaced 2026-05-07; resolve at the matching migration step in [`src/sprint.md`](src/sprint.md))

These are per-type design decisions to make as each `.proto` lands, surfaced when reviewing the `auki-registry` types pre-migration. Each is gating its corresponding migration step but not blocking the rest.

### `PinholeCameraLogEntry` — `dynamic_intrinsics` placement

Locked rename: `SensorLogEntry` → `PinholeCameraLogEntry` (names what it is — pinhole-projection camera frame entry, not a generic sensor entry). Open: should `dynamic_intrinsics` (fx, fy, cx, cy, distortion_coefficients) be on every frame entry, or in a sibling intrinsics-update sub-stream that frame entries reference? Today's design is inline — pays ~80 bytes/frame for typical Brown-Conrady cameras (intrinsics that mostly don't change every frame) — justified by the autofocus story. Cleaner alternatives: (a) `Option<DynamicIntrinsics>` so non-autofocusing cameras pay ~1 byte/frame, (b) a sibling intrinsics-update log that the frame log references by timestamp, (c) registry-side intrinsics version that bumps occasionally. Resolve before `auki.camera` `.proto` lands.

### `PointCloudLogEntry` — on-disk vs wire-format drift

Today's `auki-registry::PointCloudLogEntry` has `width: u32, height: u32, is_dense: bool, data: Vec<u8>` (ROS PointCloud2-shaped, typed layout fields outside the bytes). Today's `auki-network::stream_protocol::PointCloudFrame` has `bytes: Vec<u8>` only (typed layout fields ride inside the CDR bytes). Two representations of the same data on disk vs on the wire — drift. Resolve: pick one shape and use it both places. Lean: opaque-bytes-only (`PointCloudFrame { bytes }`) — interpretation comes from the registry entry's PointCloud body; doesn't bake ROS PointCloud2 into the type. Open whether the existing typed-fields approach has any reader benefit worth the asymmetry. Resolve before `auki.point_cloud` `.proto` lands.

### `AudioLogEntry` — implicit vs explicit chunk metadata

Today's `auki-registry::AudioLogEntry` has `data: Vec<u8>` only. Sample count is implicit: `data.len() / sample_byte_width / channels`. Reader has to look up registry's `sample_format` + `channels` and compute. Adding a typed `sample_count: u32` (or `chunk_duration_ns: i64`) makes the metadata honest; small bytes overhead per chunk. Lean: add `sample_count` — one varint per chunk. Resolve before `auki.audio` `.proto` lands.

### `TimeTransformEntry` — `source` belongs in manifest, `discontinuous` is computed

Today's `auki-time-transforms::TimeTransformEntry` has `offset_ns: i64, uncertainty_ns: u32, source: TimeTransformSource, discontinuous: bool`.

- **`source: TimeTransformSource`** is per-entry constant data for the lifetime of a log — every entry in the same log has the same source. Belongs in the manifest, not on every sample. Move to manifest field at migration.
- **`discontinuous: bool`** is computed from neighboring entries (`true` iff `|offset_ns - prev_offset_ns| ≥ threshold` per the docstring). Storing it bloats every entry and bakes one writer's threshold choice into the on-disk bytes. Drop it — readers compute with their own threshold.

Resolve before `auki.time_transform` `.proto` lands.

### `TimeTransformSource` — collapse the single-variant enum

Today: `enum TimeTransformSource { LocalClockRead }`. Single-variant enum "designed to grow." Premature abstraction. Two paths: (a) drop the enum entirely (manifest field becomes `producer: "local_clock_read"` string or just no field at all), or (b) keep the enum at the manifest layer (matches `PoseSource`'s tagged-enum extension pattern). Lean: (b) — the precedent is set by `PoseSource`, and the cost is one tagged-string-field on the manifest. Resolve before `auki.time_transform` `.proto` lands.

---

## Migration architecture decisions

These are decisions taken before the migration starts so each step has them to point at instead of relitigating per-PR.

### Manifest encoding stays JCS-JSON, not protobuf

**Decided 2026-05-07.** Manifests, registry entries, signing payloads stay JCS-canonical UTF-8 JSON via [`auki-jcs`](../../auki-jcs). Only segment payloads (per-frame bulk data) are protobuf via this crate.

Reasons: (a) JCS gives free cross-language byte-equivalence — protobuf doesn't (canonical-protobuf is engineering work, not a property of the format); (b) manifests are operator-debugged via `cat`, browser-read by Park, and inspected by ad-hoc tooling — JSON is the universal denominator; (c) the property protobuf-on-segment-payloads buys (wire compactness on per-frame data, schema enforcement across languages) doesn't transfer to per-recording metadata (~500 bytes, written once, read by humans + code + browsers).

Revisit if (a) manifests start getting signed *and* canonical-protobuf tooling becomes table stakes for the team, or (b) a real Go/Swift consumer ships and finds JSON parsing painful (current consumers — Rust, Python, browser JS — don't).

### `build_*_log_manifest` builders + manifest schemas → new `auki-manifests` crate

**Decided 2026-05-07.** A new `auki-manifests` crate holds the SDK's manifest contract — the `build_*_log_manifest` builders, the manifest read-side parsers + validators, and the manifest-shape schemas (the JCS-JSON shapes currently documented in [`auki-registry/README.md`](../../auki-registry/README.md)). Symmetric with this crate: `auki-datatypes` owns segment payload shapes; `auki-manifests` owns manifest shapes. [`auki-logs`](../../auki-logs) stays pure generic framing; [`auki-registry`](../../auki-registry) stays identity-only; [`auki-session`](../../auki-session) stays path helpers + `Session::open` lifecycle.

Sequenced as a **prep PR before migration step 1** — extract `build_sensor_log_manifest` and `build_pose_log_manifest` from `auki-registry`, `build_manifest` from `auki-time-transforms`, into the new crate. Pure refactor, no behaviour change. Keeps step 1 focused on the segment-encoder swap. See [`src/sprint.md`](src/sprint.md) Step 0.

Naming: `auki-manifests` over `auki-logging` (idiom collision in Rust — "logging" reads as observability/tracing) and over `auki-log-manifests` (slightly long). Says exactly what the crate does.
