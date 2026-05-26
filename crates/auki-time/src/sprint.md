# Sprint — auki-time/src

Current implementation notes for the time crate.

## Current state

- Rust product behavior lives in `core.rs` and is re-exported from `lib.rs`.
- Native default builds enable UniFFI through `ffi.rs`; generated Python and Swift packages expose time-transform records, NTP math, peer-clock sync state, domain-clock composition, and `SessionClock`.
- Browser builds use `--no-default-features --features wasm` and expose only web-safe wasm-bindgen adapters from `wasm.rs`.
- Native-only behavior (`SystemClock`, `tick`, `SessionClock`, `Sampler`, `auki-logs`, `auki-registry`, and `libc`) is gated away from wasm.

## Verified

- `cargo test -p auki-time --no-default-features`
- `cargo test -p auki-time`
- `cargo check -p auki-time --target wasm32-unknown-unknown --no-default-features --features wasm`
- `python3 scripts/bindings/generate_bindings.py plan python auki-time`
- `python3 scripts/bindings/generate_bindings.py plan swift auki-time`
- `python3 scripts/bindings/generate_bindings.py plan javascript auki-time`
- `AUKI_PYTHON_NATIVE_TARGETS=aarch64-apple-darwin just generate-python-bindings auki-time`
- Python import smoke against the generated UniFFI package
- `just generate-javascript-bindings auki-time`
- `just generate-swift-bindings auki-time`
- `swift build --package-path bindings/swift/auki-time`

## Next

- Add CI coverage for the generated Python, Swift, and JavaScript package checks once the repo-wide binding lane is introduced.
- Keep future `convert_time` interpolation APIs in `core.rs`; expose only deterministic, browser-safe pieces in `wasm.rs`.
