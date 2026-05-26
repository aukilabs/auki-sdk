# Sprint — auki-jcs

`auki-jcs` now follows the SDK-wide multiplatform binding standard. The crate remains the canonical RFC 8785 wrapper for Rust consumers while generated bindings expose a JSON-string adapter that can be called from Python, Swift, and JavaScript.

## Current status

- Rust root API remains `canonicalize(&serde_json::Value) -> Vec<u8>`.
- Shared implementation and tests live in `core.rs`.
- Native generated Python/Swift API exposes `canonicalize_json(json: String) -> Result<Vec<u8>, JcsError>`.
- JavaScript/WebAssembly API exposes `canonicalizeJson(json: string) -> Uint8Array`.
- Binding generation is owned by `bindings.toml` and crate-local package templates.
- Current Rust dependents opt out of default binding features with `default-features = false`.

## Next checks

- Keep object ordering, slash preservation, control-character escaping, and UTF-8 behavior locked across Rust, Python, Swift, and JavaScript.
- When CI grows the SDK-wide binding matrix, add `auki-jcs` beside `auki-hash` in the fast lane because it proves JSON byte equivalence across generators.
