# auki-proto src

Current implementation: generated prost modules are checked in under
`src/generated/` and included from `src/lib.rs`. The crate also implements
`auki_logs::LogPayload` for log payload messages so existing log code can use
the generated records directly.

This crate remains the schema-generated exception to the SDK UniFFI migration:
there is no `core.rs` / `ffi.rs` / `wasm.rs` split here. Non-Rust protobuf
packages are generated into ignored `bindings/{python,swift,javascript}/auki-proto`
directories by the root proto scripts and should be tested as protobuf packages,
not as UniFFI packages.
