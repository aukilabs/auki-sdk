# Changelog — JavaScript bindings

Append-only timeline of JavaScript binding changes.

---

### Nils's codex · May 22, HKT, 2026

Added a generated Node-compatible JavaScript/WASM smoke test for `auki-uniffi-test`; the generator now emits `smoke.mjs` after `wasm-pack` output and runs it as the final verification step.

### Nils's codex · May 22, HKT, 2026

Hardened JavaScript binding generation so `wasm-pack` output is staged before replacing the final package, generated files stay trackable, and npm metadata/README content describes the JavaScript/WASM package.

### Nils's codex · May 21, HKT, 2026

Added the JavaScript binding family scaffold and `auki-uniffi-test` wasm-bindgen generation target.
