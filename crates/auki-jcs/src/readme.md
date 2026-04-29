# `auki-jcs/src/`

RFC 8785 JSON Canonicalization Scheme — produces canonical UTF-8 bytes for content-addressed hashing.

## What's here

A single source file: [`lib.rs`](lib.rs).

## Public API

```rust
pub fn canonicalize(value: &serde_json::Value) -> Vec<u8>;
```

That's it. One function, one input type, one output type.

## How it works

A thin wrapper over the [`serde_jcs`](https://crates.io/crates/serde_jcs) crate. We pass through to `serde_jcs::to_vec(value)`. The wrapper exists so every consumer in the workspace canonicalizes the same way (one upstream version, one call site).

The function is **infallible** because the input is `serde_json::Value`, which cannot hold non-finite numbers (NaN/±∞). The `serde_jcs` API returns `Result` only because of that edge case; we `expect` it away with a clear panic message that documents the invariant.

## What this crate guarantees

Per RFC 8785:

- Object keys are sorted lexicographically by UTF-16 code units (§3.2.3).
- Numbers are emitted in ECMAScript canonical form (§3.2.2.3).
- Strings use short escapes where available, otherwise lowercase `\u00xx` escapes for control chars (§3.2.2.2).
- Code points above U+007F pass through as raw UTF-8 (no escaping).
- Forward slash `/` is **not** escaped.
- Object → bytes → re-parsed object → bytes is byte-identical (round-trip stable).

## Tests (8 total)

`#[cfg(test)] mod tests` in `lib.rs`:

| Test | Asserts |
|------|---------|
| `empty_object_and_array` | `{}` and `[]` produce literal `"{}"` / `"[]"` |
| `primitives` | `null`, `true`, `false`, integers, floats |
| `object_keys_are_sorted` | RFC 8785 §3.2.3 lexicographic key ordering, recursively |
| `array_order_is_preserved` | Arrays are not reordered |
| `control_chars_use_lowercase_hex_escapes` | RFC 8785 §3.2.2.2 escape rules |
| `non_ascii_passes_through_as_utf8` | `€`, `汉` emit as raw UTF-8 |
| `forward_slash_is_not_escaped` | `/` preserved verbatim |
| `round_trip_is_stable` | Value → bytes → re-parse → bytes is identical |

## Consumers in this workspace

- `auki-registry` — canonicalizes Sensor + Clock entries before hashing
- `auki-logs` — canonicalizes log manifests before writing them to disk
