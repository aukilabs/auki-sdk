# Changelog — auki-hash

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 24, HKT, 2026

Swift package template now excludes the generated XCFramework directory from the source target while retaining it as a binary target, removing SwiftPM unhandled-file warnings from generated package builds.

### Nils's codex · May 24, HKT, 2026

`auki-hash` now follows the multiplatform binding standard. The XXH3-128 implementation moved into binding-free `src/core.rs`, the crate root re-exports the existing Rust API, native builds compile a UniFFI adapter for Python/Swift generation, and wasm builds compile a wasm-bindgen `hashJcsBytes` export. The crate now owns `bindings.toml`, Python/Swift/JavaScript package templates, JavaScript smoke vectors, a local `uniffi-bindgen` helper, and a surface test preserving root Rust API compatibility.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
