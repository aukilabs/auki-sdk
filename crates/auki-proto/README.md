# auki-proto

`auki-proto` is the generated Rust protobuf crate for the Auki SDK.

Canonical schemas live at [`../../proto/auki`](../../proto/auki). Run
`just generate-rust-proto` after editing a schema and commit the generated Rust
files under `src/generated/`.

The JavaScript/TypeScript, Swift, and Python protobuf outputs are generated
locally under `bindings/*/auki-proto/` and are ignored by git. This crate is the
only committed generated protobuf output.

`auki-proto` is intentionally not a normal UniFFI crate. It is the protobuf
schema binding track that sits beside the crate-owned UniFFI/wasm packages:
Rust consumers use checked-in prost modules, while Python, Swift, and
JavaScript consumers generate protobuf packages from the same root schemas.
The Python generator derives its `protobuf>=...` runtime dependency from the
generated gencode version emitted by the installed `protoc`.
