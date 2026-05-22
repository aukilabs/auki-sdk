# auki-hash

XXH3-128 wrapper for the Auki SDK — returns a 32-character lowercase-hex content hash. Used as the version suffix on every registry entry on disk.

**Status:** Shipped.

## Public surface

- `hash_jcs_bytes(bytes) -> String` — hash arbitrary bytes (the input is expected to already be RFC 8785 canonical, hence the name).
- Locked test vectors pin the byte→hex mapping across language reimplementations.

## Depends on

Nothing in the workspace.
