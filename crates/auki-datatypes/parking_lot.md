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

## Structured prost fields vs opaque bytes — when does each apply? _(filed 2026-05-09 after [#77](https://github.com/aukilabs/auki-sdk/pull/77) made the split precedent visible)_

This crate has split precedent for log payload shapes. Pinning the principle would help future PRs not relitigate per-type.

**Opaque-bytes-only (`bytes data = 1`):**
- `PointCloudLogEntry` (Step 3, 2026-05-08) — interpretation via `SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`.
- `AudioLogEntry` (Step 4, 2026-05-08) — interpretation via `SensorBody::Audio { sample_format, channels, sample_rate_hz, ... }` (renamed from `Microphone` 2026-05-14).
- `DetectionLogEntry` (Step 8, 2026-05-08) — per-Detector schema; the SDK does not interpret detector-specific fields.

**Structured prost fields:**
- `PinholeCameraLogEntry { DynamicIntrinsics dynamic_intrinsics; bytes frame }` (Step 1, 2026-05-08) — structured intrinsics inline-optional + opaque JPEG bytes.
- `SpatialTransform { Vec3 translation; Quat orientation }` (Step 5, 2026-05-08) — fully structured.
- `TimeTransformEntry { int64 offset_ns; uint32 uncertainty_ns }` (Step 6, 2026-05-08) — fully structured.
- `JointEncodersLogEntry { repeated float angles_rad }` ([#77](https://github.com/aukilabs/auki-sdk/pull/77), 2026-05-09) — fully structured.

**Working principle (lean):**

- **Structured if** the bytes have a SINGLE canonical interpretation that holds across all instances of the sensor type. Examples: every pose has a translation and orientation; every time-transform sample is `(offset_ns, uncertainty_ns)`; every joint-encoder reading is `f32[joint_count]`; every pinhole camera intrinsics block is `(fx, fy, cx, cy, distortion[])`. The schema is universal; structured prost gives free language portability and field-level forward/backward compat.
- **Opaque-bytes-only if** the bytes have MULTIPLE possible layouts a producer must specify, OR the schema is owned by a downstream consumer outside the SDK. Examples: point cloud's variable `fields` (XYZ vs XYZRGB vs XYZRGBL...) requires per-stream metadata in `SensorBody::PointCloud`; audio's `sample_format` knob requires `SensorBody::Audio`; detection schemas are per-Detector and the SDK explicitly doesn't interpret them. Layout knowledge lives in the registry-side body type (or, for detection, with the Detector); the segment payload is just bytes.

**Edge case — mixed (structured envelope + opaque bytes):** `PinholeCameraLogEntry`. The intrinsics block is structured (universal across pinhole cameras) but the frame is opaque JPEG bytes (multiple possible image-format choices). Both halves follow the principle independently.

**Forward path:** pin this as a section in [`src/readme.md`](src/readme.md) alongside the migration documentation, and reference it from each new prost type's per-step decision. Each future payload type designer can then either match the principle or document why they're departing. Defer the actual writeup until a future payload-type design needs to reference it — filing here so the principle is captured before another PR relitigates it.

**Confidence: medium.** The principle is descriptive of the existing types but isn't tested against weird future cases (e.g. a detector that emits structured prost on the wire because two consumers want field-level access — does it become a sibling registry-backed body, like cameras? would that make `DetectionLogEntry` un-opaque case-by-case?). Revisit when a real future case stretches it.

---

## Per-type slop fixes (surfaced 2026-05-07; resolve at the matching migration step in [`src/sprint.md`](src/sprint.md))

These are per-type design decisions to make as each `.proto` lands, surfaced when reviewing the `auki-registry` types pre-migration. Each is gating its corresponding migration step but not blocking the rest.

### ✓ Resolved 2026-05-08 — `PointCloudLogEntry` is opaque-bytes-only (Step 3)

Adjudicated in favour of opaque-bytes-only: `auki.point_cloud.PointCloudLogEntry { bytes data = 1; }`. Symmetric with the wire's `PointCloudFrame { bytes }`; the ROS-shaped layout fields (`width`, `height`, `is_dense`) are gone from the per-frame type. Interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`. Resolved + propagated in the same step's PR — no Propagate task carries over.

### ✓ Resolved 2026-05-08 — `AudioLogEntry` is opaque-bytes-only (Step 4)

Adjudicated in favour of opaque-bytes-only — `auki.audio.AudioLogEntry { bytes data = 1; }`. Same stance as Step 3 for point clouds; declines the pre-Step-3 sprint lean toward adding `sample_count`. Sample count and chunk duration are both derivable from the bytes plus the SensorRegistryEntry's `Audio { sample_format, channels, sample_rate_hz }` body (renamed from `Microphone` 2026-05-14). Reader needs the registry to interpret bytes anyway; denormalizing either field would risk inconsistency for marginal convenience. Resolved + propagated in the same step's PR — no Propagate task carries over.

### ✓ Resolved 2026-05-08 — `TimeTransformEntry` slop points (Step 6)

All three slop points adjudicated and landed at Step 6:

- **`source` moved to manifest** — pre-migration `source: TimeTransformSource` was per-entry constant data; now lives on the manifest as `source: TimeTransformSource` (tagged enum, mirrors `PoseSource`).
- **`discontinuous: bool` dropped** — computed by readers with their own threshold, not baked into the bytes by the writer.
- **`TimeTransformSource` kept as tagged enum at manifest layer** (Option 2) — matches `PoseSource`'s extension pattern; one variant today (`LocalClockRead`), future producers (`NtpSynced { server }`, `SyncedTo { peer_id }`, ...) attach metadata without a schema break.

Resolved + propagated in the same step's PR — no Propagate tasks carry over.

### ✓ Resolved 2026-05-08 — `DetectionLogEntry` is opaque-bytes-only (Step 8)

Adjudicated in favour of opaque-bytes-only: `auki.detection.DetectionLogEntry { bytes data = 1; }`. Same stance as Steps 3 (point cloud) and 4 (audio); the detection schema is defined per-Detector, not by the SDK. Carrying detector-specific fields on the prost type would either lock the SDK into knowing every detector's schema or force a degenerate `oneof` of every shipped detector — neither scales. Resolved + propagated in the same step's PR — no Propagate task carries over.

The Detection-Log analog of `SensorRegistryEntry` (the registry entry that pins per-`(detector_id, ...)` interpretation of the opaque bytes) is a sibling shape that lives in [`auki-registry`](../../auki-registry) when needed, not in this crate. File when subscription / discovery for detection logs needs it.

### ✓ Resolved 2026-05-09 — `JointEncodersLogEntry` and `JointEncodersFrame` are structured (`repeated float angles_rad`), and ship as a paired wire/disk package

Decided: `auki.joint_encoders.JointEncodersLogEntry` (on disk) and `auki.joint_encoders_stream.JointEncodersFrame` (on wire), both `repeated float angles_rad = 1`. Byte-identical wire/disk shape, locked by an explicit symmetry test (`joint_encoders_disk_wire_byte_identical`) — Step 2/3 didn't need that test because `bytes`-only payloads were trivially identical. Two separate proto packages so the wire and log code paths dispatch on distinct Rust types (Step 2/3 precedent). Resolved + propagated in the same PR.

Two component-decisions filed at the same PR:

- **`angles_rad` precision: f32 over f64.** Going with `repeated float` (f32) to match `SpatialTransform`'s quaternion components. Revisit if a consumer needs higher precision for low-rate slow-motion replay.
- **No `velocity_rad_per_s` / `effort_nm` companion fields on `JointEncodersLogEntry`.** v1 ships positions only — minimal-fields stance from Steps 3/4. ROS `JointState` carries velocity and effort and the K1 publishes velocity, so the upstream signal is there. Revisit when a consumer (predictive smoothing, force-controlled teleop, non-Park use) earns the addition. Adding new proto fields later is cheap (new field number); adding them now bakes them in for everyone.

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
