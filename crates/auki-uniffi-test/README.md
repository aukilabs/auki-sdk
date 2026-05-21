# auki-uniffi-test

Small standalone UniFFI proving crate for the Auki SDK workspace.

This crate is not an SDK component and does not wrap any production Auki crate. Its job is to keep a compact, easy-to-build UniFFI surface in `crates/` so Swift binding generation patterns can be tested before they are applied to real SDK components.

## Surface

- `add(left, right) -> i32`
- `hello(name) -> String`
- `make_greeting(name, style) -> Result<Greeting, TestError>`
- `delayed_greeting(name, delay_ms) -> async Result<Greeting, TestError>`
- `Counter::new(initial)`, `Counter::value()`, `Counter::add_after(delta, delay_ms)`

The surface intentionally covers plain values, records, enums, typed errors, objects, and async exports.

## Build

```bash
cargo test -p auki-uniffi-test
cargo build -p auki-uniffi-test
```

Swift binding generation uses the root `justfile` recipe:

```bash
just generate-swift-bindings auki-uniffi-test
```

The generated Swift source, FFI header/modulemap, and host debug library land in `bindings/swift/auki-uniffi-test/generated/`. The package root is `bindings/swift/auki-uniffi-test/`.

For an iOS/macOS-consumable Swift package, build the XCFramework:

```bash
just build-swift-xcframework auki-uniffi-test
```

That writes `bindings/swift/auki-uniffi-test/generated/auki_uniffi_test.xcframework` for iOS device, iOS simulator, and macOS. The static `Package.swift` lives at `bindings/swift/auki-uniffi-test/`; the recipe only refreshes files under `generated/`.

Python binding generation also uses the root `justfile`:

```bash
just generate-python-bindings auki-uniffi-test
```

The generated Python package lives at `bindings/python/auki-uniffi-test/`. The API is written to `auki_uniffi_test/__init__.py`, and native libraries live under `auki_uniffi_test/native/<rust-target>/`.

The recipe also builds native libraries for the host and default Linux targets. Linux builds use `cross`, so Docker must be running.
