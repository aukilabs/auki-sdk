# `auki-registry-py/src/`

Implementation status for the `auki_registry` Python module. Spec: this crate's [outer `README.md`](../README.md).

## Implemented

| Surface | Status |
|---|---|
| Frame constructors | shipped |
| Sensor constructors | shipped |
| Clock constructors | shipped |
| `hash_frame` / `hash_sensor` / `hash_clock` | shipped |
| `canonical_json_frame` / `canonical_json_sensor` / `canonical_json_clock` | shipped |
| `write_frame` / `write_sensor` / `write_clock` | shipped |
| `read_frame` / `read_sensor` / `read_clock` | shipped |

The binding keeps entries as Python `dict`s instead of PyClasses. This matches the registry's on-disk JSON shape and lets Python producers pass entries through config, logs, and tests without class identity concerns.

All validation, canonicalization, hashing, path construction, and atomic writes delegate to the Rust `auki-registry` crate. The Python wrapper only converts Python dicts to JSON values and maps errors to Python exceptions.

## Tests

- `cargo test -p auki-registry-py`
- `maturin develop -m crates/auki-registry-py/Cargo.toml && pytest crates/auki-registry-py/python_tests/`
