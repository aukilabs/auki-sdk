# `auki-uniffi-test/src/`

Implementation status for [`auki-uniffi-test`](../README.md).

## Files

- [`lib.rs`](lib.rs) — feature-gated module wiring for native UniFFI and wasm-bindgen builds.
- [`core.rs`](core.rs) — binding-free Rust logic shared by all binding surfaces.
- [`ffi.rs`](ffi.rs) — UniFFI-native API for Swift and Python generation.
- [`wasm.rs`](wasm.rs) — wasm-bindgen API for JavaScript/WebAssembly generation.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — local UniFFI codegen entry point, gated behind the `cli` feature.

## What works today

- Sync exported functions: `add`, `hello`, `make_greeting`.
- UniFFI value types: `Greeting` record and `GreetingStyle` enum.
- Typed UniFFI error: `TestError`.
- Async exported function: `delayed_greeting`.
- UniFFI object: `Counter`, including async method `add_after`.
- wasm-bindgen exports cover the same proving surface with JavaScript naming (`makeGreeting`, `delayedGreeting`, `Counter.addAfter`).
- Rust-side tests cover all exported behavior.
- Root `just generate-python-bindings auki-uniffi-test` workflow writes the generated Python package and then builds/copies native libraries under `bindings/python/auki-uniffi-test/`.
- Root `just generate-javascript-bindings auki-uniffi-test` workflow writes the generated JavaScript/WebAssembly package under `bindings/javascript/auki-uniffi-test/` and runs `smoke.mjs`.
- Root `just install-toolchain` installs or converges the Rust targets and binding CLIs needed by the Swift, Python, and JavaScript recipes.

## What does not work yet

- No published Python wheels.
- No published npm package.
- Default Linux native-library builds require Docker for `cross`.
- No dependency on real SDK crates.

Those omissions are intentional for this first proving crate.
