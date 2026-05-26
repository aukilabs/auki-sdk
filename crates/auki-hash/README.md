# auki-hash

Content-addressed identity hashing for the Auki SDK — the contract any implementation (Rust, Python, Swift, Kotlin) must follow when producing the hash that becomes a registry entry's filename.

## Contract

| Property  | Value                                     |
|-----------|-------------------------------------------|
| Algorithm | XXH3-128                                  |
| Seed      | 0                                         |
| Encoding  | 32-character lowercase hex, zero-padded   |

The 32-char width is mandatory: short hashes pad with leading zeros so the output is always exactly 32 characters regardless of input.

## Conformance vectors

Any implementation that produces these outputs for these inputs is conformant. The SDK's tests use them as the cross-language acceptance check.

| Input        | Output                              |
|--------------|-------------------------------------|
| `b""`        | `99aa06d3014798d86001c324468d497f`  |
| `b"abc"`     | `06b05ab6733a618578af5f94892f3950`  |

## Why XXH3-128

- **Fast** — ~6 GB/s on commodity hardware. Hashing a typical registry entry (~1 KB) is dominated by serialization, not hashing.
- **128 bits is enough** for content-addressing collision safety at this scale (birthday bound on ~10⁶ entries per session is negligible).
- **Not cryptographic.** We trade adversarial collision-resistance for speed. If cryptographic strength becomes a requirement (signed commits, tamper detection), swap to BLAKE3: same wrapper shape, locked vectors update.

## Why a wrapper

Centralizing the hash function means every consumer in the workspace hashes the same way (same algorithm, same seed, same encoding). Algorithm swaps are a one-file change.

## Binding generation

`auki-hash` follows the SDK's multiplatform crate standard:

- shared Rust logic lives in [`src/core.rs`](src/core.rs);
- native Python and Swift exports live in [`src/ffi.rs`](src/ffi.rs) behind the `uniffi` feature;
- JavaScript/WebAssembly exports live in [`src/wasm.rs`](src/wasm.rs) behind the `wasm` feature;
- crate-owned binding policy and package templates live under [`bindings.toml`](bindings.toml) and [`bindings/`](bindings/).

The generated Python, Swift, and JavaScript packages expose the same locked-vector hash behavior as the Rust root API. Rust workspace consumers should depend on this crate with `default-features = false` unless they are intentionally exercising generated binding support.

## See also

- [`auki-jcs`](../auki-jcs) — produces the canonical bytes that this crate hashes.
- [`auki-registry`](../auki-registry) — the primary consumer; embeds these hashes in entry filenames.
