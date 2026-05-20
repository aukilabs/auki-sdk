# Changelog — auki-manifests-py

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 20, HKT, 2026

**Package relocated to `bindings/python/auki-manifests-py`.** The Python package moved from `crates/auki-manifests-py` to `bindings/python/auki-manifests-py` with no package-name, module-name, or runtime behavior changes. Cargo workspace membership and local path dependencies now point at the new location.

### broodsugar's claude · May 9, 14:15 HKT, 2026

**Crate scaffolding + Python surface for [`auki-manifests`](../auki-manifests).** Four `#[pyfunction]` wrappers — `build_sensor_log_manifest`, `build_pose_log_manifest`, `build_time_transform_log_manifest`, `build_detection_log_manifest`. Each returns a Python `dict` consumers hand straight to [`auki_logs.Log.open(path, manifest)`](../auki-logs-py/src/lib.rs).

**Enum seam.** `PoseSource`, `PoseWriterMode`, and `TimeTransformSource` are Rust tagged enums — Python takes them as **dicts** / **strings** matching the JCS-canonical JSON shape they serialize to:

- `PoseSource::Ros2Tf { publishers }` → `{"kind": "ros2_tf", "publishers": [...]}`
- `PoseWriterMode::Rigid | Movable` → `"rigid"` | `"movable"`
- `TimeTransformSource::LocalClockRead` → `{"kind": "local_clock_read"}`

Sidesteps PyClass complexity; matches the natural shape Python consumers already think in. Decode failures (unknown variant, missing `kind`) → `ValueError` with prefix.

**Manifest dict seam.** Returned Python `dict` round-trips Rust `serde_json::Value` via Python's stdlib `json` module — `json.dumps` on entry, `json.loads` on exit. Same pattern as [`auki-logs-py`](../auki-logs-py)'s manifest seam.

**Closes the manifest-construction half** of the [`detectors`](https://github.com/aukilabs/detectors) phase-2 Python ergonomics gap. Sibling to [`auki-layout-py`](../auki-layout-py) (filed in the same PR) which closes the path-construction half.

**Build pipeline.** `abi3-py38` via PyO3 0.22 + maturin. Mirrors the rest of the `*-py` family.

**Tests:**
- Rust-side (`cargo test -p auki-manifests-py`): 2 — `parse_pose_writer_mode` accepts canonical strings + rejects garbage.
- Python-side (`pytest python_tests/`): 6 — all four builders' field-presence + an enum-rejection test.

**Out of scope** (filed in [`parking_lot.md`](parking_lot.md)):
- `PoseSource::canonical_bytes` / `hash` graduation primitives.
- PyClass equivalents of the enums.
- Read-side parsers + validators — track the parallel question in `auki-manifests` itself.
- `intent` field across every builder — wired in when the Rust uniform-rollout PR lands.

Will land in v0.0.27.
