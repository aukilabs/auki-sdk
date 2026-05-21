# `auki-hash/src/`

XXH3-128 content hash for already-canonical bytes — typically the output of [`auki_jcs::canonicalize`](../../auki-jcs/).

## What's here

A single source file: [`lib.rs`](lib.rs).

## Public API

```rust
pub fn hash_jcs_bytes(bytes: &[u8]) -> String;
```

Returns a **32-character lowercase hex** representation of the XXH3-128 hash of `bytes`.

## How it works

Wraps `xxhash_rust::xxh3::xxh3_128(bytes)` and formats the resulting `u128` with `format!("{h:032x}")`. The `032x` is load-bearing: it pads short hashes with leading zeros so the output is always exactly 32 chars regardless of input.

The seed is fixed at 0 (the `xxhash-rust` default). The seed must be fixed because content addressing has to produce the same hash on every machine, every run.

## Why a wrapper

Centralizing the hash function means every consumer in the workspace hashes the same way (same algorithm, same seed, same encoding). If we ever need to swap algorithms (e.g. to BLAKE3 for cryptographic strength), it's a one-file change.

## Tests (6 total)

`#[cfg(test)] mod tests` in `lib.rs`:

| Test | Asserts |
|------|---------|
| `length_is_always_32` | Hex output is exactly 32 chars regardless of input size |
| `lowercase_hex_only` | Output contains only `[0-9a-f]` |
| `deterministic` | Same input → same output |
| `different_inputs_differ` | Different inputs → different outputs (bit-flip sanity) |
| `known_vector_empty` | `b""` → `99aa06d3014798d86001c324468d497f` (locked vector) |
| `known_vector_abc` | `b"abc"` → `06b05ab6733a618578af5f94892f3950` (locked vector) |

The two locked vectors are regression guards: if the upstream `xxhash-rust` crate ever changes algorithm or seed defaults, these tests fail loudly instead of silently shifting every previously-stored hash.

## Consumers in this workspace

- `auki-registry` — hashes JCS-canonical Sensor + Clock entries to produce on-disk filenames (`<id>/<hash>.json`)
- `auki-time` (transitively) — manifests reference clock registry entries by hash
