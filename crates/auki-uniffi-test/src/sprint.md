# Sprint — auki-uniffi-test

## Now

The crate exists as a small UniFFI proving surface under `crates/`. It validates the mechanics needed for future Swift bindings:

- plain functions
- records
- enums
- typed errors
- async free functions
- objects
- async object methods
- `uniffi-bindgen` helper binary behind a `cli` feature
- root `just generate-swift-bindings auki-uniffi-test` workflow that writes generated Swift files to `bindings/swift/auki-uniffi-test/generated/`
- root `just build-swift-xcframework auki-uniffi-test` workflow that writes an iOS + macOS XCFramework to `bindings/swift/auki-uniffi-test/generated/` and leaves `bindings/swift/auki-uniffi-test/` as a static Swift package root
- root `just generate-python-bindings auki-uniffi-test` workflow that writes a generated Python package, then builds/copies host and Linux native libraries under `bindings/python/auki-uniffi-test/`

## Next

If this crate becomes the template for production UniFFI bindings:

1. Add the `just generate-swift-bindings` and `just build-swift-xcframework` checks to CI on macOS.
2. Add the `just generate-python-bindings` check to CI on a host with Python available.
3. Run the full `just generate-python-bindings` check once CI has Docker / `cross` available.
4. Keep the surface small so failures point at UniFFI mechanics, not SDK logic.
