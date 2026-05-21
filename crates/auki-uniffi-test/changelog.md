# Changelog — auki-uniffi-test

Append-only timeline of changes for the UniFFI proving crate. Latest entry on top.

---

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
