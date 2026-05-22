# Changelog - proto/auki

Append-only timeline of canonical Auki protobuf schema changes. Latest entry on top.

---

### Nils's codex - May 22, HKT, 2026

Updated schema comments that referenced the removed `auki-datatypes` crate; disk/wire symmetry notes now point at `auki-proto`.

### Nils's codex - May 22, HKT, 2026

Added `message.proto` with `auki.message.MessageEnvelope` and `MessageAck` for the proto-backed `/auki/message/0.0.1` peer message protocol.

### Nils's codex - May 22, HKT, 2026

Copied the existing Auki protobuf schemas from `crates/auki-datatypes/proto/` into root `proto/auki/` as the canonical schema source.
