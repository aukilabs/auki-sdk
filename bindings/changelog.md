# Changelog — bindings

One-line summaries of changes in language binding packages. Detailed entries live in the most-specific binding package `changelog.md` files and propagate up to the root timeline.

Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

**Bindings parking-lot summary now includes JavaScript.** The bindings-level parking lot points at the JavaScript/Web package target-policy question alongside the existing Python and Swift binding-family summaries.

### Nils's codex · May 22, HKT, 2026

**JavaScript `auki-uniffi-test` smoke test added.** See [`bindings/javascript`](javascript/changelog.md) for the generated Node-compatible `smoke.mjs` and generator verification step.

### Nils's codex · May 22, HKT, 2026

**JavaScript `auki-uniffi-test` binding generation hardened.** See [`bindings/javascript`](javascript/changelog.md) for the staged `wasm-pack` output flow, trackable generated package files, and JavaScript/WASM package metadata updates.

### Nils's codex · May 21, HKT, 2026

**Python `auki-uniffi-test` package root added.** See [`bindings/python`](python/changelog.md) for the UniFFI package layout and single generation/native-library build recipe.

### Nils's codex · May 21, HKT, 2026

**Swift `auki-uniffi-test` package root added.** See [`bindings/swift`](swift/changelog.md) for the static SwiftPM package layout and generated iOS/macOS XCFramework location.

### Nils's codex · May 21, HKT, 2026

**Python bindings updated for the SDK stream naming cleanup.** See [`bindings/python`](python/changelog.md) for package-level propagation of the `CameraFrame` / `DetectionFrame` / `Camera` vocabulary.

### Nils's claude · May 20, 13:31 HKT, 2026

**Swift bindings family added under [`bindings/swift`](swift/changelog.md).** `auki-network-swift` relocated from `crates/auki-network-swift` to `bindings/swift/auki-network-swift` to follow the same per-language convention introduced for Python by PR #156. Package name, lib name, surface, runtime behavior unchanged; only paths and relative doc links moved.

### Nils's codex · May 20, HKT, 2026

**Python packages moved under [`bindings/python`](python/changelog.md).** The `auki-*-py` package family left `crates/` for the language-binding hierarchy with package names, Python module names, and runtime behavior preserved.
