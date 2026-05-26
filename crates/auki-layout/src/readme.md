# `auki-layout/src/`

Path helpers for the on-disk session shape. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`core.rs`](core.rs) — binding-free path helper implementation and unit tests. No external dependencies — just `std::path`.
- [`lib.rs`](lib.rs) — feature-gated module wiring and Rust API re-exports.
- [`ffi.rs`](ffi.rs) — native UniFFI string adapters for Python and Swift generation.
- [`wasm.rs`](wasm.rs) — wasm-bindgen string adapters for JavaScript/WebAssembly generation.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — crate-local UniFFI bindgen entrypoint.

## Public functions

```rust
pub fn registries_root(app_root: &Path) -> PathBuf;
pub fn sensor_entry_path(app_root: &Path, sensor_id: &str, hash: &str) -> PathBuf;
pub fn clock_entry_path(app_root: &Path, clock_id: &str, hash: &str) -> PathBuf;
pub fn frame_entry_path(app_root: &Path, frame_id: &str, hash: &str) -> PathBuf;
pub fn session_root(app_root: &Path, session: &str) -> PathBuf;
pub fn timetransform_log_path(session_root: &Path, from_id: &str, to_id: &str) -> PathBuf;
pub fn sensorlog_path(session_root: &Path, sensor_log_id: &str) -> PathBuf;
pub fn poselog_path(session_root: &Path, from_frame_id: &str, to_frame_id: &str) -> PathBuf;
pub fn detection_log_path(session_root: &Path, detector_id: &str, input_log_id: &str) -> PathBuf;
pub fn id_to_segment(id: &str) -> String;
```

## Tests (14 core unit tests + 1 surface integration test)

| Test | Asserts |
|------|---------|
| `registries_root_is_under_app` | `<app>/registries` |
| `sensor_entry_path_includes_id_substitution_and_hash_filename` | `<app>/registries/sensors/<id-subst>/<hash>.json`; `/` → `__` |
| `clock_entry_path_uses_clocks_dir` | Same, under `clocks/` |
| `frame_entry_path_uses_frames_dir` | Same, under `frames/` (Frame Registry; v0.0.22) |
| `detector_entry_path_uses_detectors_dir_and_id_substitution` | Same, under `detectors/` |
| `session_root_is_app_join_session_id` | `<app>/<session>` |
| `timetransform_log_path_uses_double_underscore_separator` | Joined as `<from>__<to>`, both substituted |
| `sensorlog_path_is_session_join_sensorlogs_join_sensor_log_id` | `<session>/sensorlogs/<sensor_log_id>` |
| `sensorlog_path_does_not_substitute_sensor_log_id` | sensor_log_id passes through opaque (no `/` → `__` substitution) |
| `poselog_path_uses_double_underscore_separator` | `<session>/poselogs/<from>__<to>`, both substituted (mirrors `timetransform_log_path`; Step 5, 2026-05-08) |
| `poselog_path_substitutes_slashes_inside_each_frame_id` | Each frame_id's `/` → `__`, then sides join with another `__` |
| `detection_log_path_keys_on_detector_id_and_input_log_id` | `<session>/detection_logs/<detector_id>__<input_log_id>` (Detector binding API, 2026-05-09) |
| `detection_log_path_substitutes_slashes_in_detector_id_only` | Detector ID's `/` → `__`; input_log_id passes through opaque |
| `id_to_segment_is_idempotent_for_ids_without_slashes` | No-op for ids without `/` |
| `tests/surface.rs::rust_root_api_remains_source_compatible` | Crate-root re-exports keep the existing Rust `&Path` / `PathBuf` API available under the default UniFFI feature |

## Consumers in this workspace

- `auki-registry` — uses `sensor_entry_path` / `clock_entry_path` / `frame_entry_path` to locate registry entries. The single source of truth for the layout lives here.
- Generated Python, Swift, and JavaScript packages — use string adapters over the same helpers through crate-owned UniFFI and wasm-bindgen assets.
- *Downstream apps* (boosterapp, Park) — use these helpers to construct paths for log opens and registry reads, instead of string-concatenating layout-specific directory names.
- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — phase-2 integration uses `detection_log_path` to locate the per-`(detector, input log)` output log. Caller-decides per [keystone](../../../parking_lot.md) — Park / Boosterapp opens the log; the detector receives the write-handle.
