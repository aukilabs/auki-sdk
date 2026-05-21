# `auki-uniffi-test/src/`

Implementation status for [`auki-uniffi-test`](../README.md).

## Files

- [`lib.rs`](lib.rs) — the complete UniFFI test surface.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — local UniFFI codegen entry point, gated behind the `cli` feature.

## What works today

- Sync exported functions: `add`, `hello`, `make_greeting`.
- UniFFI value types: `Greeting` record and `GreetingStyle` enum.
- Typed UniFFI error: `TestError`.
- Async exported function: `delayed_greeting`.
- UniFFI object: `Counter`, including async method `add_after`.
- Rust-side tests cover all exported behavior.
- Root `just generate-python-bindings auki-uniffi-test` workflow writes the generated Python package and then builds/copies native libraries under `bindings/python/auki-uniffi-test/`.

## What does not work yet

- No published Python wheels.
- Default Linux native-library builds require Docker for `cross`.
- No dependency on real SDK crates.

Those omissions are intentional for this first proving crate.
