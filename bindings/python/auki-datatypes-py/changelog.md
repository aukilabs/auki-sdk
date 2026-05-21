# Changelog — auki-datatypes-py

Append-only changelog for this package. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**Generated datatypes now expose `CameraFrame` and `DetectionFrame`.** The committed betterproto files and locked-vector tests use the final payload names while preserving the prost/betterproto wire-byte equality contract.

### Nils's codex · May 20, HKT, 2026

**Package relocated to `bindings/python/auki-datatypes-py`.** The Python package moved from `crates/auki-datatypes-py` to `bindings/python/auki-datatypes-py` with no package-name, module-name, or runtime behavior changes. Cargo workspace membership and local path dependencies now point at the new location.

### Nils's codex · May 18, HKT, 2026

**`frame_stream` is removed from the Python betterproto surface.** The generated `auki_datatypes.auki.frame_stream` module and top-level `frame_stream` re-export are gone because camera streams now use `auki_datatypes.camera.PinholeCameraLogEntry` directly. The module-shape test and README were updated so Python consumers see the same "camera log record == camera stream record" contract as Rust.

### Nils's codex · May 16, 12:31 HKT, 2026

**Python betterproto stream binding follows `StreamDescriptor`.** The generated `auki_datatypes.auki.stream` binding now exposes `StreamDescriptor` with `sensor_id`, `sensor_hash`, `clock_id`, `clock_hash`, `frame_id`, and `frame_hash`, and `StreamMessage.accept` points at that class. This mirrors the Rust `auki.stream` schema change from `AcceptInfo` to descriptor-shaped accept metadata. Added a Python round-trip smoke test for the descriptor shape.

### broodsugar's claude · May 9, 14:30 HKT, 2026

**Crate scaffolding + betterproto-generated Python bindings for [`auki-datatypes`](../auki-datatypes).** Closes [Step 9 of the migration sprint](../auki-datatypes/src/sprint.md) — the typed-message half of [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4. The bytes-level half [shipped in `auki-logs-py`](../auki-logs-py); this PR lets Python consumers skip hand-rolling prost.

**Surface mirrors the Rust crate one-to-one.** Every `auki_datatypes::<name>::<Type>` in Rust has a matching `auki_datatypes.<name>.<Type>` in Python — `audio.AudioLogEntry`, `camera.PinholeCameraLogEntry`, `detection.DetectionLogEntry`, `frame_stream.JpegFrame`, `joint_encoders.JointEncodersLogEntry`, `joint_encoders_stream.JointEncodersFrame`, `point_cloud.PointCloudLogEntry`, `point_cloud_stream.PointCloudFrame`, `pose.{SpatialTransform, Vec3, Quat}`, `stream.*`, `time_transform.TimeTransformEntry`. Top-level re-exports in `auki_datatypes/__init__.py` let consumers write `from auki_datatypes import detection` directly.

**Cross-language byte equality verified.** `tests/test_locked_vectors.py` pins every Rust locked vector from `auki-datatypes/src/lib.rs`'s `*_serializes_to_locked_wire_bytes` tests; betterproto's encoder produces byte-identical bytes for the same input. **10/10 tests passing** — `PinholeCameraLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`, `SpatialTransform`, `TimeTransformEntry`, `DetectionLogEntry`, `JointEncodersLogEntry`, plus round-trip + module-shape smoke tests. The locked vectors are the cross-language contract: any drift between betterproto and prost trips them immediately.

**Why betterproto over `protobuf`.** `google.protobuf` (the official Python library) generates classes that aren't dataclasses, with non-Pythonic API (`msg.field.CopyFrom(obj)` instead of `msg.field = obj` for messages). `betterproto` generates plain dataclasses — `Msg(field=value, nested=Nested(x=1))` works as expected, `bytes(msg)` / `Msg().parse(bytes)` give clean serialization symmetry. Wire format is identical between the two; only the Python API changes. Pinned to `betterproto==1.2.5`.

**Build pipeline — pure Python, not maturin.** No Rust code in this crate; the codegen is pure Python (`protoc-gen-python_betterproto` from `betterproto[compiler]`). Therefore `pyproject.toml` uses `hatchling` as the build backend, NOT maturin. No `Cargo.toml`; not a Cargo workspace member. This is a deliberate departure from `auki-network-py` / `auki-logs-py` etc., which all wrap Rust code.

**Codegen workflow.** `./regen.sh` runs `protoc -I ../auki-datatypes/proto --python_betterproto_out=auki_datatypes ../auki-datatypes/proto/*.proto`. Requires `protoc` on PATH (e.g. `brew install protobuf`) and the `[regen]` extras (`pip install -e .[regen]`). Generated files are committed alongside the script — consumers pip-install the package and don't need protoc. Contributors editing `.proto` files re-run regen and commit the diff.

**End-to-end with the rest of the SDK's Python surface.** Combined with [`auki-logs-py`](../auki-logs-py), [`auki-layout-py`](../auki-layout-py), and [`auki-manifests-py`](../auki-manifests-py), the ESL detector author writes the entire phase-2 loop in Python with four `pip install`s and zero hand-rolled prost.

**Out of scope** (filed in [`parking_lot.md`](parking_lot.md)):
- Regen-check CI test (drift detection between `.proto` files and committed generated code).
- `betterproto` 2.x bump.
- PyPI distribution policy (track every other `*-py` crate's parallel discussion).
- Type stubs (betterproto's generated dataclasses already carry type annotations; stubs are lower-priority here than for PyO3 wrappers).

Will land in v0.0.27.
