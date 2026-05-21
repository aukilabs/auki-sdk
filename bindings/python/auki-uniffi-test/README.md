# auki-uniffi-test Python package

UniFFI-generated Python package for the [`auki-uniffi-test`](../../../crates/auki-uniffi-test) proving crate.

This package is a distribution smoke path, not a production SDK component. It validates that UniFFI output can be shaped into a Python package with native libraries bundled under `auki_uniffi_test/native/<rust-target>/`.

## Regenerate

```bash
just generate-python-bindings auki-uniffi-test
```

That refreshes `auki_uniffi_test/__init__.py`, creates `pyproject.toml` if it does not exist, then builds and copies native libraries for all default targets.

`setup.py` marks the distribution as binary so wheel builds are platform-tagged instead of `py3-none-any`.

## Native libraries

```bash
just generate-python-bindings auki-uniffi-test
```

By default, generation includes the host target plus these Linux targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

On macOS, generation also includes both Darwin targets:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`

Linux target builds use `cross`, so Docker must be running.

For a local one-target regeneration while iterating:

```bash
AUKI_PYTHON_NATIVE_TARGETS="aarch64-apple-darwin" just generate-python-bindings auki-uniffi-test
```

At import time the generated package selects the native library that matches `sys.platform` and `platform.machine()`. Set `AUKI_UNIFFI_TEST_LIBRARY_PATH` to force a specific dynamic library, or `AUKI_UNIFFI_TEST_NATIVE_TARGET` to force a bundled target directory.
