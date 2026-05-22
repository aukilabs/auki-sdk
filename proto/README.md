# Auki Protobuf Schemas

This directory is the canonical source for Auki protobuf schemas.

Platform packages are generated from `proto/auki/*.proto`:

- Rust: `crates/auki-proto`
- JavaScript/TypeScript: `bindings/javascript/auki-proto` local generated output, ignored by git
- Swift/iOS: `bindings/swift/auki-proto` local generated output, ignored by git
- Python: `bindings/python/auki-proto` local generated output, ignored by git

Do not edit generated protobuf bindings by hand. Edit the `.proto` files here,
then run `just generate-proto`. Commit Rust output under `crates/auki-proto`;
do not commit generated non-Rust binding output under `bindings/`.
