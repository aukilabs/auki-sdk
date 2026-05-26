# Sprint - auki-proto

- Keep Rust generated output in sync with `proto/auki`.
- Keep non-Rust generated protobuf output outside git under ignored `bindings/` paths.
- Treat `auki-proto` as the protobuf sibling binding track, not as a UniFFI crate.
- Keep `scripts/generate-python-proto.sh` deriving the Python `protobuf>=...` runtime dependency from generated gencode so local package installs match the installed `protoc`.
