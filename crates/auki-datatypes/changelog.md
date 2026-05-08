# Changelog — auki-datatypes

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 8, 11:30 HKT, 2026

**Step 1 of the [migration](src/sprint.md) landed — first real schema. `auki.camera` ships `PinholeCameraLogEntry` + `DynamicIntrinsics`** with locked wire-bytes and XXH3-128 hash (`0496e1f71a03e00877fc68bf16190026`).

**Per-step decision: `dynamic_intrinsics` is inline-optional.** `proto/camera.proto`:

```proto
message DynamicIntrinsics { double fx=1; double fy=2; double cx=3; double cy=4; repeated double distortion_coefficients=5; }
message PinholeCameraLogEntry { DynamicIntrinsics dynamic_intrinsics=1; bytes frame=2; }
```

prost generates `dynamic_intrinsics: Option<DynamicIntrinsics>` for proto3 message fields. Non-autofocusing cameras pay only the message-tag overhead when `None`; autofocusing cameras populate per-frame. Promoting to a sibling intrinsics-update sub-stream remains a backward-compatible move (drop the field, mark its number reserved, add a sibling log) — but punted until autofocus shows up as a real workload.

**`impl_log_payload!` macro** in [`src/lib.rs`](src/lib.rs) wires every prost type into [`auki_logs::LogPayload`](../auki-logs/src/lib.rs) with one line of glue:

```rust
macro_rules! impl_log_payload { ($t:ty) => { /* encode_to_vec / decode + map_err */ }; }
impl_log_payload!(camera::PinholeCameraLogEntry);
```

Step 6's `TimeTransformEntry` will pick up the same macro; mid-migration ciborium types implement `LogPayload` directly.

**Locked vectors** (`tests::pinhole_camera_log_entry_serializes_to_locked_wire_bytes` + `_hash_is_locked`) join the workspace's cross-language conformance set. Cross-language readers (Python via betterproto, future Sentinel ports) MUST reproduce the bytes byte-identically.

**End-to-end seam test** opens an `auki_logs::Log<PinholeCameraLogEntry>`, appends two entries (one with intrinsics, one without), closes, re-reads, asserts both timestamp + payload byte-equality. Catches any regression in the macro wiring or the segment-framing path.

**New deps**: `auki-logs` (path-dep — needs the trait); dev-deps `auki-hash` (locked hash) + `serde_json` + `tempfile` (segment round-trip). Production deps add `auki-logs` only.

**Test count: 1 → 7.** Placeholder smoke test stays until Step 7 retires it. Will land in v0.0.24.

### broodsugar's dobby · May 7, 22:30 HKT, 2026

**Migration architecture decisions added to [`parking_lot.md`](parking_lot.md), Step 0 added to [`src/sprint.md`](src/sprint.md).** Two upfront decisions: (1) **Manifest encoding stays JCS-JSON, not protobuf** — JCS gives free cross-language byte-equivalence which protobuf doesn't, manifests are human/browser/ad-hoc-tool-readable, and per-recording metadata doesn't benefit from wire compactness. (2) **`build_*_log_manifest` builders + manifest schemas → new `auki-manifests` crate** — symmetric with this crate (which owns segment payload shapes); `auki-manifests` owns manifest shapes. Sequenced as **Step 0** before migration step 1, pure refactor extracting `build_sensor_log_manifest` + `build_pose_log_manifest` from `auki-registry` and `build_manifest` from `auki-time-transforms`. Naming: `auki-manifests` over `auki-logging` (idiom collision in Rust — reads as observability/tracing). Doc-only.

### broodsugar's claude · May 7, 21:00 HKT, 2026

**Crate renamed `auki-proto` → `auki-datatypes`.** Names the responsibility (canonical shared cross-language data types) instead of the implementation (protobuf via prost). Aligns with the rest of the workspace's concept-naming convention (`auki-registry`, `auki-logs`, `auki-session`, `auki-time-transforms`, `auki-network` — all named for their purpose, not their internals). Future-proofs against any downstream encoding switch.

**Scope clarified, accidental dual-purpose split out.** Per the [Notion Registries doc](https://www.notion.so/34e5c8e96592809d8977feb17c32e5d0)'s definition (*"a shared, versioned catalog of identities + definitions"*), [`auki-registry`](../auki-registry) is supposed to hold registry entries only — Sensor / Frame / Clock identity + definitions, JCS-canonical JSON, content-hashed. The log payload types (`SensorLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`, `PoseLogEntry`, `TransformSample`, `DynamicIntrinsics`) currently dumped in `auki-registry` were AI drift Nils didn't catch. Each migration step now **moves** a type from `auki-registry` into here (rather than the earlier framing of "auki-registry re-exports from auki-datatypes"). Post-migration, `auki-registry` shrinks back to its canonical scope; consumers add an `auki-datatypes` dep alongside.

**Locked renames in [`src/sprint.md`](src/sprint.md):**

- `SensorLogEntry` → `PinholeCameraLogEntry` — names what it actually is (pinhole-projection camera frame entry; `DynamicIntrinsics` is pinhole-shaped). The original generic-sounding name was wrong.
- `TransformSample` → `SpatialTransform` — matches the [Notion Pose Log doc](https://www.notion.so/34b5c8e9659280bd9580c25991f5d491). Also drops the `PoseLogEntry { transforms: Vec<TransformSample> }` wrapper — flat segments per the Pose Log synthesis.
- `TimeTransformLogEntry` → `TimeTransformEntry` — earlier sprint draft typo; correct name in [`auki-time-transforms`](../auki-time-transforms) source is `TimeTransformEntry`.
- `AudioLogEntry` migration step **added** — was missing from the earlier sprint draft.

**Five per-type slop questions added to [`parking_lot.md`](parking_lot.md)** (resolve at the matching migration step, not upfront): PinholeCameraLogEntry intrinsics placement (inline vs sub-stream vs registry-versioned); PointCloudLogEntry on-disk-vs-wire drift (typed-fields-outside-bytes vs raw-bytes-only); AudioLogEntry implicit-vs-explicit chunk metadata; TimeTransformEntry — move `source` to manifest, drop computed `discontinuous`; TimeTransformSource — collapse the single-variant enum.

**No code changes.** Cargo.toml `name` updated, all in-crate doc references updated, workspace `Cargo.toml` member entry retargeted, [`auki-session-py`](../auki-session-py) cross-references updated, root [`parking_lot.md`](../../parking_lot.md) subfolder summary updated. `cargo test -p auki-datatypes` 1 passing (placeholder pipeline-check round-trip, unchanged). The 19:30 entry below describes the original scaffold under the old name; preserved verbatim per append-only.

### broodsugar's claude · May 7, 19:30 HKT, 2026

**Crate scaffolding.** New crate `auki-proto` — single source of truth for the SDK's protobuf schemas. Owns the `.proto` definitions and the prost-generated Rust code; downstream Rust crates (`auki-registry`, `auki-logs`, `auki-network`, `auki-time-transforms`, `auki-ros-adapter`) will import the generated types from here once the migration starts. Cross-language consumers (Python via `betterproto` from [`auki-session-py`](../auki-session-py/), future Sentinel ports) generate their own bindings from the same `.proto` files.

**Why this exists.** Resolves the [`auki-session-py` `payload: bytes` encoding contract](../auki-session-py/parking_lot.md) — segment payloads on disk become protobuf-encoded; manifests + registry entries + signing payloads continue to use JCS-canonical JSON via [`auki-jcs`](../auki-jcs). Two encodings, each doing what they're good at, no overlap on the wire.

**Sub-decisions locked 2026-05-07:** `.proto` files live in a dedicated `auki-proto` crate (vs per-crate `proto/` directories or repo-root `/proto/`) — single source of truth, mirrors how `auki-hash` / `auki-jcs` work as cross-cutting primitives. `prost` for Rust codegen (libp2p-ecosystem default; clean idiomatic structs). `betterproto` for Python (lands in `auki-session-py` when first impl starts; produces dataclass-shaped output that matches the booster-claude sketch).

**Build pipeline self-contained.** `protoc` binary supplied by `protoc-bin-vendored` build-dep — no system `protoc` install needed on dev machines or CI. `build.rs` compiles every `.proto` under `proto/` into Rust under `OUT_DIR`.

**Scaffold contents.** `proto/placeholder.proto` (single empty message — validates the build pipeline end-to-end; will be removed once the first real `.proto` lands), `build.rs` (prost-build invocation), `src/lib.rs` (re-exports the placeholder module), `src/readme.md` (status), `src/sprint.md` (six-step migration plan starting with `SensorLogEntry`), [`README.md`](README.md), [`parking_lot.md`](parking_lot.md). One inline test: `placeholder_pipeline_check_round_trips` verifies encode + decode work.

**Test count: 1.** `cargo test -p auki-proto` passes. `cargo check -p auki-proto` clean.

**Migration sequence in [`src/sprint.md`](src/sprint.md):** (1) `auki.sensor_log` — `SensorLogEntry`; (2) `auki.frame` — `JpegFrame` and `auki.pointcloud` — `PointCloudFrame`; (3) `auki.pose_log` — `PoseLogEntry` + `TransformSample`; (4) `auki.time_transform` — `TimeTransformLogEntry`; (5) remove placeholder; (6) Python codegen for `auki-session-py`. Each step is its own PR with locked conformance vectors. Will land in v0.0.24.

**Four open questions in [`parking_lot.md`](parking_lot.md):** package naming convention; field number allocation strategy; locked conformance vector format (Rust struct literal / JSON / both); schema versioning policy. None gating the scaffold; all need to land before the first real `.proto`.
