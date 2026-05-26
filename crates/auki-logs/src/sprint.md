# Sprint — auki-logs/src

Current implementation status for the logs crate.

## Current status

The crate now follows the SDK binding standard:

- Generic `Log<T: LogPayload>` framing, retention, read, and tail behavior lives in [`core.rs`](core.rs) and stays binding-free.
- The crate root re-exports the existing Rust API.
- Native UniFFI adapters in [`ffi.rs`](ffi.rs) expose `BytesLog`, `BytesTail`, read-all helpers, and segment vector helpers for generated Python and Swift.
- wasm-bindgen adapters in [`wasm.rs`](wasm.rs) expose pure manifest canonicalization and segment encode/decode helpers for JavaScript/WebAssembly.
- `bindings.toml` and crate-owned language templates drive generated Python, Swift, and JavaScript packages.

## Next checks

1. Keep the generated JavaScript smoke aligned with the Rust segment-byte vector.
2. Add Python and Swift package-level smoke programs if generated package checks grow beyond import/build verification.
3. Keep filesystem log operations out of wasm until there is a concrete browser storage adapter design.
