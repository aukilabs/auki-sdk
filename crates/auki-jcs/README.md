# auki-jcs

RFC 8785 JSON Canonicalization Scheme — produces canonical UTF-8 bytes for any `serde_json::Value`. Paired with [`auki-hash`](../auki-hash) to give every registry entry a stable content hash regardless of producer-side field order or whitespace.

**Status:** Shipped.

## Public surface

- `canonicalize(value) -> Vec<u8>` — emit the RFC 8785 canonical byte sequence.

## Depends on

Nothing in the workspace.
