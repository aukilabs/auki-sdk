# Changelog — auki-uniffi-test Python package

Append-only changelog for the UniFFI-generated Python package. See [AGENTS.md](../../../AGENTS.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Created the Python package root for `auki-uniffi-test`. `just generate-python-bindings auki-uniffi-test` now writes the generated UniFFI API to `auki_uniffi_test/__init__.py`, stores native libraries under `auki_uniffi_test/native/<rust-target>/`, and patches the generated loader to select the current platform. Added `just build-python-native-libs` / `just build-python-native-lib` for release native-library builds, using `cross` for Linux targets.

The package includes a minimal setuptools `BinaryDistribution` hook so built wheels are platform-tagged rather than pure Python.

Collapsed the public Just interface back to one recipe: `just generate-python-bindings auki-uniffi-test` now runs generation and native-library builds in order. Target selection is internal, with `AUKI_PYTHON_NATIVE_TARGETS` as the local override.

Removed the package-local `generated/` scratch directory. UniFFI bindgen now writes to a temporary directory, and only the patched final `auki_uniffi_test/__init__.py` is copied into the Python package root.
