# Sprint — auki-uniffi-test

## Now

The crate exists as a small binding-generation proving surface under `crates/`. Its pure Rust logic lives in `core.rs`, while native UniFFI exports live in `ffi.rs` and JavaScript/WebAssembly exports live in `wasm.rs`. It validates the mechanics needed for future Swift, Python, and JavaScript bindings:

- plain functions
- records
- enums
- typed errors
- async free functions
- objects
- async object methods
- binding-free core logic shared by native and web exports
- `uniffi-bindgen` helper binary behind a `cli` feature
- root `just generate-swift-bindings auki-uniffi-test` workflow that writes generated Swift files to `bindings/swift/auki-uniffi-test/generated/`
- root `just build-swift-xcframework auki-uniffi-test` workflow that writes an iOS + macOS XCFramework to `bindings/swift/auki-uniffi-test/generated/` and leaves `bindings/swift/auki-uniffi-test/` as a static Swift package root
- root `just generate-python-bindings auki-uniffi-test` workflow that writes a generated Python package, then builds/copies host and Linux native libraries under `bindings/python/auki-uniffi-test/`
- root `just generate-javascript-bindings auki-uniffi-test` workflow that writes a generated wasm-bindgen package under `bindings/javascript/auki-uniffi-test/` and runs its Node-compatible smoke test
- root `just install-toolchain` workflow that installs Rust targets plus pinned `cross`, `wasm-pack`, and `wasm-bindgen-cli`

## Next

If this crate becomes the template for production UniFFI bindings:

1. Add the `just generate-swift-bindings` and `just build-swift-xcframework` checks to CI on macOS.
2. Add the `just generate-python-bindings` check to CI on a host with Python, Docker, and `cross` available.
3. Add the `just generate-javascript-bindings` check to CI on a host with Node and `wasm-pack` available.
4. Keep the surface small so failures point at binding mechanics, not SDK logic.
