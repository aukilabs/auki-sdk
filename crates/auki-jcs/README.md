# auki-jcs

JSON canonicalization for the Auki SDK — produces the byte-stable input that [`auki-hash`](../auki-hash) hashes.

## Contract

Implements **RFC 8785** (JSON Canonicalization Scheme) verbatim:

- Object keys sorted lexicographically by UTF-16 code units (§3.2.3).
- Numbers in ECMAScript canonical form (§3.2.2.3).
- Short escapes for control chars where available, otherwise lowercase `\u00xx` (§3.2.2.2).
- Code points above U+007F pass through as raw UTF-8 (no escaping).
- Forward slash `/` is **not** escaped.
- Round-trip stable: object → bytes → re-parse → bytes is byte-identical.

## Why this matters

Without canonicalization, two equivalent JSON values (different key order, different number formatting) hash to different bytes. Content-addressed storage requires that semantically-equal data hashes the same; RFC 8785 is the standard way to get there.

## Cross-language

Any implementation of the SDK MUST canonicalize per RFC 8785. JCS is well-specified and has reference implementations in major languages — this is a deliberate non-invention.

## See also

- [`auki-hash`](../auki-hash) — hashes the bytes this crate produces.
- [`auki-registry`](../auki-registry) — canonicalizes Sensor + Clock entries before writing/hashing.
- [`auki-logs`](../auki-logs) — canonicalizes log manifests before writing them to disk.
