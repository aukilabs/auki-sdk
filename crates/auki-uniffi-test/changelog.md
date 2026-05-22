# Changelog — auki-uniffi-test

Append-only timeline of changes for the UniFFI proving crate. Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

Moved binding-generation ownership into this crate. Added `bindings.toml` plus crate-local JavaScript, Python, and Swift templates/smoke assets; the README now documents Cargo metadata as the crate identity source, `bindings.toml` as binding policy, and root scripts as generic orchestration only.

### Nils's codex · May 22, HKT, 2026

Expanded the crate README into an agent-facing guide for the binding-generation standard. It now explains the binding-free core split, the separate UniFFI and wasm-bindgen adapter files, the Cargo feature contract, the crate-local `uniffi-bindgen` helper, the root `just` recipes, and the checklist for updating or creating another crate that can generate Swift, Python, and JavaScript/WebAssembly bindings.

### Nils's codex · May 22, HKT, 2026

Clarified binding-generation prerequisite and parking-lot propagation docs. `just install-toolchain` now validates Node/npm for the JavaScript smoke path, and the crate parking lot describes the proving surface as native UniFFI plus web wasm-bindgen rather than Swift-only UniFFI.

### Nils's codex · May 22, HKT, 2026

Split the proving crate into binding-free core logic plus separate native and web binding surfaces. `core.rs` now owns the shared `add`, greeting, delay validation, and counter behavior; `ffi.rs` owns the UniFFI Swift/Python exports; and `wasm.rs` owns the wasm-bindgen JavaScript exports. Added the `wasm` feature and kept default native builds on the UniFFI surface.

Added JavaScript/WebAssembly generation for this proving crate. `just generate-javascript-bindings auki-uniffi-test` builds the crate with `--no-default-features --features wasm`, writes a complete package under `bindings/javascript/auki-uniffi-test/`, and runs the generated Node-compatible `smoke.mjs`.

Added the root `just install-toolchain` recipe for binding-generation prerequisites: Rust targets, Docker-backed `cross`, wasm-bindgen tooling, Python, and Apple build tools on macOS.

### Nils's codex · May 21, HKT, 2026

Turned the generated Python output into a package root under `bindings/python/auki-uniffi-test`. The generated UniFFI API now lands in `auki_uniffi_test/__init__.py`, native libraries live under `auki_uniffi_test/native/<rust-target>/`, and the generated loader selects the current platform with override env vars. Added `just build-python-native-libs` and `just build-python-native-lib` recipes backed by `scripts/build-python-native-libs.sh`, using `cross` for Linux targets.

Collapsed the public Python binding workflow back to one Just recipe. `just generate-python-bindings auki-uniffi-test` now orchestrates package generation first and native-library compilation/copying second; target selection is internal, with `AUKI_PYTHON_NATIVE_TARGETS` as a local override.

### Nils's codex · May 21, HKT, 2026

Added the root `just generate-python-bindings <crate>` workflow and `scripts/generate-python-bindings.sh`. Verified with `auki-uniffi-test`: UniFFI generates `auki_uniffi_test.py`, copies the host debug library to `bindings/python/auki-uniffi-test/generated/`, and the generated Python module smoke-tests sync functions, records/enums, objects, and async exports.

### Nils's codex · May 21, HKT, 2026

Extracted the Swift binding generation shell bodies from the root `justfile` into root-level scripts. The public `just generate-swift-bindings auki-uniffi-test` and `just build-swift-xcframework auki-uniffi-test` commands stay unchanged.

### Nils's codex · May 21, HKT, 2026

Extended the Swift package smoke path to cover both iOS and macOS. `Package.swift` is now a static package-root file under `bindings/swift/auki-uniffi-test/`, while `just build-swift-xcframework auki-uniffi-test` refreshes only `generated/` with Swift glue, headers, and a device/simulator/macOS XCFramework.

### Nils's codex · May 21, HKT, 2026

Changed Swift generation to treat `bindings/swift/<crate>/` as the Swift package root and `bindings/swift/<crate>/generated/` as the generated-artifact directory. Added `just build-swift-xcframework <crate>` for iOS device + simulator static libs and XCFramework assembly. Verified `auki-uniffi-test` as a local Swift package with `xcodebuild -scheme auki-uniffi-test -destination 'generic/platform=iOS' build`.

### Nils's codex · May 21, HKT, 2026

Added the root `just generate-swift-bindings <crate>` workflow and documented this crate's smoke path: `just generate-swift-bindings auki-uniffi-test` builds the crate, runs its `cli`-gated `uniffi-bindgen` helper, and writes Swift output to `bindings/swift/auki-uniffi-test/`.

### Nils's codex · May 21, HKT, 2026

Created `auki-uniffi-test`, a standalone workspace crate under `crates/` for proving UniFFI mechanics before binding production SDK components. The crate exports plain sync functions, a `Greeting` record, `GreetingStyle` enum, typed `TestError`, async `delayed_greeting`, and a stateful `Counter` object with async `add_after`. It builds as `staticlib` / `cdylib` / `rlib` and includes a `cli`-gated `uniffi-bindgen` helper binary.
