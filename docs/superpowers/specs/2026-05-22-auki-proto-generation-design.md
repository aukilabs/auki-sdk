# Auki Proto Generation Design

## Goal

Move the SDK protobuf contract out of `auki-datatypes` and into a platform-neutral `auki-proto` generation system. Rust gets a committed generated crate; browser JavaScript/TypeScript, Swift/iOS, and Python get generated local binding outputs under `bindings/` that are not committed.

## Decisions

The canonical schema source is `proto/auki/*.proto` at the repository root. No platform owns the schema files.

Each platform gets its own generated output path:

- Rust: `crates/auki-proto` committed
- JavaScript/TypeScript: `bindings/javascript/auki-proto` generated locally, ignored by git
- Swift/iOS: `bindings/swift/auki-proto` generated locally, ignored by git
- Python: `bindings/python/auki-proto` generated locally, ignored by git

`auki-datatypes` is deprecated after `auki-proto` lands. It remains temporarily as a Rust compatibility shim while workspace crates and downstream consumers move to `auki-proto`.

UniFFI is not used for protobuf bindings. UniFFI remains for behaviorful Rust APIs such as identity, domain lifecycle, or future SDK operations. Protobuf packages are schema/codegen artifacts, not FFI surfaces.

## Rust Boundary

Rust follows the same generation rule as every other platform. `crates/auki-proto` contains checked-in generated prost files under `src/generated/`, produced by a repo script. The crate does not own schema design and does not run `prost-build` at ordinary crate build time.

The Rust crate may contain a thin hand-written shell:

- module includes and re-exports for generated files;
- optional `logs` feature implementing `auki_logs::LogPayload` for generated message types, because Rust orphan rules require this impl to live in the type-owning crate;
- small constructor helpers for prost oneof ergonomics, only when they construct generated messages and carry no SDK lifecycle or network policy.

No libp2p runtime, Discovery logic, domain lifecycle, wallet operations, or app-facing APIs live in `auki-proto`.

## Layering Rule

- Schemas: `proto/auki/*.proto`
- Generated data packages: committed Rust `auki-proto`; ignored local non-Rust bindings under `bindings/`
- Log framing: `auki-logs`
- Transport protocol handlers: `auki-network`
- Cluster/app lifecycle: `auki-domain`
- Registry identity catalogs: `auki-registry`
- Manifests: `auki-manifests`

For example, `/auki/message/0.0.1` starts as `proto/auki/message.proto`, then each platform regenerates its `auki-proto` package, then `auki-network` implements the libp2p protocol handler over those bytes, and `auki-domain` later exposes app-facing send/receive ergonomics.

## Migration Constraints

The migration should preserve buildability after each commit:

1. Add root schema source and `auki-proto` before deleting the old `auki-datatypes` implementation.
2. Keep `auki-datatypes` as a shim while direct Rust imports move.
3. Preserve locked wire-byte vectors in Rust. Non-Rust generated outputs may run local parity checks, but those generated files are not committed.
4. Do not introduce a second hand-maintained schema copy.
5. Do not route protobuf messages through UniFFI.
6. Do not commit generated JavaScript/TypeScript, Swift, or Python protobuf output in this migration.

## Non-Goals

This migration does not implement `/auki/message/0.0.1` itself. It creates the schema-generation foundation that message protocol work should use.

This migration does not change the existing stream, log, pose, audio, camera, detection, point-cloud, or time-transform protobuf shapes except for source-path and package-generation ownership.
