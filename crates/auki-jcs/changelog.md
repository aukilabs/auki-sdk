# Changelog — auki-jcs

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 24, HKT, 2026

Swift package template now excludes the generated XCFramework directory from the source target while retaining it as a binary target, removing SwiftPM unhandled-file warnings from generated package builds.

### Nils's codex · May 24, HKT, 2026

`auki-jcs` now follows the multiplatform binding standard. The RFC 8785 implementation moved into binding-free `src/core.rs`, the crate root re-exports the existing Rust `canonicalize(&serde_json::Value)` API, native builds compile a UniFFI `canonicalize_json` adapter for Python/Swift generation, and wasm builds compile a wasm-bindgen `canonicalizeJson` export. The crate now owns `bindings.toml`, Python/Swift/JavaScript package templates, JavaScript smoke vectors, a local `uniffi-bindgen` helper, and a surface test preserving Rust API compatibility.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
