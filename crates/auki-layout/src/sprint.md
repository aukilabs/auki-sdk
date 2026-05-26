# Sprint — auki-layout/src

Current implementation status for the layout crate.

## Current status

The crate now follows the SDK binding standard:

- Core path construction lives in [`core.rs`](core.rs) and stays binding-free.
- The crate root re-exports the existing Rust `&Path` / `PathBuf` API.
- Native UniFFI adapters in [`ffi.rs`](ffi.rs) expose Python/Swift-friendly string helpers.
- wasm-bindgen adapters in [`wasm.rs`](wasm.rs) expose JavaScript-friendly camelCase string helpers.
- `bindings.toml` and crate-owned language templates drive generated Python, Swift, and JavaScript packages.

## Next checks

1. Keep generated package smoke vectors aligned with the Rust path helper tests.
2. Resolve the existing ID-encoding parking-lot question before declaring the layout contract v1-stable.
