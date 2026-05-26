# Sprint — auki-hash

`auki-hash` now follows the SDK-wide multiplatform binding standard. The crate remains a tiny content-hash primitive: one binding-free core function, one UniFFI native adapter, and one wasm-bindgen JavaScript/WebAssembly adapter.

## Current status

- Rust root API remains `hash_jcs_bytes(&[u8]) -> String`.
- Native generated Python/Swift API exposes `hash_jcs_bytes(bytes: Vec<u8>) -> String`.
- JavaScript/WebAssembly API exposes `hashJcsBytes(bytes: Uint8Array) -> string`.
- Binding generation is owned by `bindings.toml` and crate-local package templates.
- Current Rust dependents opt out of default binding features with `default-features = false`.

## Next checks

- Keep empty-input and `b"abc"` vectors locked across Rust, Python, Swift, and JavaScript.
- When CI grows the SDK-wide binding matrix, add `auki-hash` to the fast lane because it is the cheapest production crate that exercises every generator.
