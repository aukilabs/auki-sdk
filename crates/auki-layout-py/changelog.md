# Changelog — auki-layout-py

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 9, 14:15 HKT, 2026

**Crate scaffolding + Python surface for [`auki-layout`](../auki-layout).** Ten `#[pyfunction]` wrappers — `registries_root`, `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `timetransform_log_path`, `sensorlog_path`, `poselog_path`, `detection_log_path`, `id_to_segment`. Each takes Python `str` arguments, calls the corresponding `auki-layout` Rust function, returns the resulting `PathBuf` as a `str`. No state, no PyClasses.

Closes the path-construction half of the [`detectors`](https://github.com/aukilabs/detectors) phase-2 Python ergonomics gap. Previously Python consumers either hand-rolled `f"{session}/detection_logs/{detector_id.replace('/', '__')}__{input_log_id}"` (drift risk if the Rust crate's substitution rule changes) or imported `auki-layout` indirectly via subprocess hacks. Now they `import auki_layout` and call the helper.

Sibling to [`auki-manifests-py`](../auki-manifests-py) (filed in the same PR). Together they cover the path + manifest construction surface for any Python `auki-logs-py` consumer.

**Build pipeline.** `abi3-py38` via PyO3 0.22 + maturin. Mirrors [`auki-network-py`](../auki-network-py) and [`auki-logs-py`](../auki-logs-py) exactly. Default-empty Cargo features so `cargo test` against the rlib works; `extension-module` enabled by maturin via `[tool.maturin]`.

**Tests:**
- Rust-side (`cargo test -p auki-layout-py`): 3 — `detection_log_path` substitution, `sensor_entry_path` shape, `id_to_segment` round-trip.
- Python-side (`pytest python_tests/`): 9 — module shape + per-helper substitution mirrors of the Rust crate's own tests.

Will land in v0.0.27.
