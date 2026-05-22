# JavaScript Bindings

JavaScript-facing SDK packages live here. `auki-uniffi-test` is the proving package for wasm-bindgen generation over shared Rust core logic.

Generate the proving package from the repo root:

```bash
just generate-javascript-bindings auki-uniffi-test
```

That writes `bindings/javascript/auki-uniffi-test/` as a complete web-targeted ESM package with JavaScript glue, TypeScript declarations, the compiled WebAssembly module, npm metadata, and `smoke.mjs`. The generation recipe runs the smoke test after replacing the package.
