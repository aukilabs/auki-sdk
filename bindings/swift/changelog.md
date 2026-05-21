# Changelog — Swift bindings

One-line summaries of changes in Swift binding packages. Detailed entries live in each package's `changelog.md`.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**`auki-uniffi-test` Swift package root added.** The package keeps a static `Package.swift` at `bindings/swift/auki-uniffi-test/` and consumes generated Swift glue, headers, and the iOS/macOS XCFramework from its `generated/` directory.

### Nils's claude · May 20, 13:31 HKT, 2026

**`auki-network-swift` relocated from `crates/` to `bindings/swift/`.** Brings the UniFFI Swift binding under the `bindings/<language>/` convention established by PR #156 for the Python packages. Package name, lib name, surface, and runtime behavior unchanged; only paths and relative doc links moved.
