# Changelog - auki-proto

Append-only changelog for this crate. Latest entry on top.

---

### Nils's codex - May 24, HKT, 2026

JavaScript protobuf generation now declares a current TypeScript dev dependency so generated `@bufbuild/protobuf` 2.x declarations type-check after dependency resolution.

### Nils's codex - May 24, HKT, 2026

Audited `auki-proto` as the protobuf sibling binding track rather than a UniFFI crate. Rust prost tests passed, ignored SwiftProtobuf output generated and built, ignored JavaScript/TypeScript output generated and type-checked for representative schemas, and ignored Python protobuf output imported in a venv after `scripts/generate-python-proto.sh` was fixed to derive `protobuf>=...` from the generated gencode version.

### Nils's codex - May 24, HKT, 2026

Opted the `auki-logs` dependency out of default features after `auki-logs` adopted the binding standard, keeping protobuf `LogPayload` impls on the direct Rust trait without pulling in UniFFI.

### Nils's codex - May 24, HKT, 2026

Opted the test-only `auki-hash` dependency out of default features after `auki-hash` adopted the binding standard, keeping protobuf vector tests on the direct Rust hash API without pulling in UniFFI.

### Nils's codex - May 22, HKT, 2026

`auki-proto` is now the sole Rust protobuf package. The deprecated `auki-datatypes` shim was removed from the workspace, and crate/module docs no longer describe a compatibility layer.

### Nils's codex - May 22, HKT, 2026

Added generated `auki.message` Rust bindings plus locked-vector tests for `MessageEnvelope` and `MessageAck`, sourced from root `proto/auki/message.proto`.

### Nils's codex - May 22, HKT, 2026

Added the generated Rust `auki-proto` crate sourced from root `proto/auki` schemas.
