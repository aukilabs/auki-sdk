# Changelog — Python bindings

One-line summaries of changes in Python binding packages. Detailed entries live in each package's `changelog.md`.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**`auki-uniffi-test` UniFFI package root added.** The Python smoke package now has generated source under `auki_uniffi_test/`, native libraries under `native/<rust-target>/`, and a single `generate-python-bindings` recipe that runs generation plus `cross`-backed Linux native-library builds.

### Nils's codex · May 21, HKT, 2026

**Python bindings standardize camera frame and registry vocabulary.** Datatypes, network, registry, domain, logs, and session binding docs/surfaces now use `CameraFrame`, `DetectionFrame`, `Camera`, and `"camera"` consistently.

### Nils's codex · May 20, HKT, 2026

**`auki-*-py` packages relocated from `crates/` to `bindings/python/`.** The move preserves package names, Python module names, Cargo package names for PyO3 wrappers, and runtime behavior while making `crates/` Rust-focused.
