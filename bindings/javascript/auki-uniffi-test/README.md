# auki-uniffi-test JavaScript bindings

Generated wasm-bindgen JavaScript package for the `auki-uniffi-test` proving crate.

Generate the package from the repo root:

```bash
just generate-javascript-bindings auki-uniffi-test
```

The generated package contains:

- `package.json` — npm package metadata for the web-targeted ESM package.
- `auki_uniffi_test.js` — wasm-bindgen JavaScript glue.
- `auki_uniffi_test.d.ts` — TypeScript declarations for the JavaScript glue.
- `auki_uniffi_test_bg.wasm` — compiled WebAssembly module.
- `auki_uniffi_test_bg.wasm.d.ts` — wasm-bindgen WebAssembly declarations.
- `smoke.mjs` — Node-compatible smoke test for the generated web-target package.
