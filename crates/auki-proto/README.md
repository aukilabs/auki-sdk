# auki-proto

`auki-proto` is the generated Rust protobuf crate for the Auki SDK.

Canonical schemas live at [`../../proto/auki`](../../proto/auki). Run
`just generate-rust-proto` after editing a schema and commit the generated Rust
files under `src/generated/`.

The JavaScript/TypeScript, Swift, and Python protobuf outputs are generated
locally under `bindings/*/auki-proto/` and are ignored by git. This crate is the
only committed generated protobuf output.
