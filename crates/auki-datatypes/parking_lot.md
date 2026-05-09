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

### ✓ Resolved 2026-05-08 — `PointCloudLogEntry` is opaque-bytes-only (Step 3)

Adjudicated in favour of opaque-bytes-only: `auki.point_cloud.PointCloudLogEntry { bytes data = 1; }`. Symmetric with the wire's `PointCloudFrame { bytes }`; the ROS-shaped layout fields (`width`, `height`, `is_dense`) are gone from the per-frame type. Interpretation comes from `(sensor_id, sensor_hash) → SensorBody::PointCloud { fields, point_step, is_bigendian, frame_id }`. Resolved + propagated in the same step's PR — no Propagate task carries over.

### ✓ Resolved 2026-05-08 — `AudioLogEntry` is opaque-bytes-only (Step 4)

Adjudicated in favour of opaque-bytes-only — `auki.audio.AudioLogEntry { bytes data = 1; }`. Same stance as Step 3 for point clouds; declines the pre-Step-3 sprint lean toward adding `sample_count`. Sample count and chunk duration are both derivable from the bytes plus the SensorRegistryEntry's `Microphone { sample_format, channels, sample_rate_hz }` body. Reader needs the registry to interpret bytes anyway; denormalizing either field would risk inconsistency for marginal convenience. Resolved + propagated in the same step's PR — no Propagate task carries over.

### ✓ Resolved 2026-05-08 — `TimeTransformEntry` slop points (Step 6)

All three slop points adjudicated and landed at Step 6:

- **`source` moved to manifest** — pre-migration `source: TimeTransformSource` was per-entry constant data; now lives on the manifest as `source: TimeTransformSource` (tagged enum, mirrors `PoseSource`).
- **`discontinuous: bool` dropped** — computed by readers with their own threshold, not baked into the bytes by the writer.
- **`TimeTransformSource` kept as tagged enum at manifest layer** (Option 2) — matches `PoseSource`'s extension pattern; one variant today (`LocalClockRead`), future producers (`NtpSynced { server }`, `SyncedTo { peer_id }`, ...) attach metadata without a schema break.

Resolved + propagated in the same step's PR — no Propagate tasks carry over.

### ✓ Resolved 2026-05-08 — `DetectionLogEntry` is opaque-bytes-only (Step 8)

Adjudicated in favour of opaque-bytes-only: `auki.detection.DetectionLogEntry { bytes data = 1; }`. Same stance as Steps 3 (point cloud) and 4 (audio); the detection schema is defined per-Detector, not by the SDK. Carrying detector-specific fields on the prost type would either lock the SDK into knowing every detector's schema or force a degenerate `oneof` of every shipped detector — neither scales. Resolved + propagated in the same step's PR — no Propagate task carries over.

The Detection-Log analog of `SensorRegistryEntry` (the registry entry that pins per-`(detector_id, ...)` interpretation of the opaque bytes) is a sibling shape that lives in [`auki-registry`](../../auki-registry) when needed, not in this crate. File when subscription / discovery for detection logs needs it.

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
