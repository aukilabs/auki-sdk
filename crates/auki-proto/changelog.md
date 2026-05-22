# Changelog - auki-proto

Append-only changelog for this crate. Latest entry on top.

---

### Nils's codex - May 22, HKT, 2026

`auki-proto` is now the sole Rust protobuf package. The deprecated `auki-datatypes` shim was removed from the workspace, and crate/module docs no longer describe a compatibility layer.

### Nils's codex - May 22, HKT, 2026

Added generated `auki.message` Rust bindings plus locked-vector tests for `MessageEnvelope` and `MessageAck`, sourced from root `proto/auki/message.proto`.

### Nils's codex - May 22, HKT, 2026

Added the generated Rust `auki-proto` crate sourced from root `proto/auki` schemas.
