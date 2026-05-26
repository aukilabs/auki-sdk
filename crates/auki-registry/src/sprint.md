# Sprint — auki-registry/src

Current implementation status for the registry crate.

## Current status

The crate now follows the SDK binding standard:

- Core Sensor / Clock / Frame / Detector registry types and filesystem storage live in [`core.rs`](core.rs) and stay binding-free.
- The crate root re-exports the existing typed Rust API.
- Native UniFFI adapters in [`ffi.rs`](ffi.rs) expose Python/Swift-friendly JSON-string canonicalization, hash, and read/write helpers.
- wasm-bindgen adapters in [`wasm.rs`](wasm.rs) expose web-safe canonicalization, hash, and frame-preset helpers without filesystem I/O.
- `bindings.toml` and crate-owned language templates drive generated Python, Swift, and JavaScript packages.

## Next checks

1. Keep cross-language vectors aligned with the Rust locked hashes for Sensor, Clock, Frame, and Detector entries.
2. Add typed generated-language DTO helpers only when a real consumer needs them; the current generated surface intentionally uses JSON strings to avoid duplicating the nested Rust enum graph.
3. Keep browser storage out of the wasm surface until there is a concrete storage adapter design.
