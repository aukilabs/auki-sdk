# Parking lot — auki-identity

---

## BIP32-style HD vs. simpler labeled-hash derivation

`derive_child` uses a labeled-hash scheme: `child_seed = XXH3-128(parent_seed || label)`, doubled to 32 bytes by re-hashing with `/expand`. Simple, no HKDF dep, reproducible cross-language.

BIP32-style hierarchical deterministic derivation (used by Bitcoin/Ethereum wallets) gives nicer guarantees — independent siblings, hardened-vs-non-hardened distinction, standard derivation paths like `m/44'/501'/0'`. But it's heavier (HMAC-SHA512, secp256k1 in canonical BIP32; we'd need an ed25519 variant like SLIP-0010), more code, more deps.

Worth revisiting if/when:
- Console UX needs to show standard derivation paths.
- Cross-tooling (existing crypto wallets) want to import/export.
- Hardware wallet integration becomes a thing.

For now, the labeled-hash scheme is fine — internal use, no compat-with-existing-wallet expectations.

---

## Encrypted-at-rest format for `seed`

Currently the seed is exposed as raw 32 bytes via `wallet.seed()`. Caller is responsible for encryption when persisting.

The Console session will need a passphrase-protected serialization (PBKDF2 / Argon2id over the passphrase, AES-GCM or XChaCha20-Poly1305 over the seed). Worth deciding whether that lives:

- In `auki-identity` (one canonical encrypted format every consumer uses)
- In a separate small crate (`auki-keystore`?) so `auki-identity` stays minimal
- In each consumer (Console picks its own; OS-keychain consumers pick theirs)

My instinct: separate crate. `auki-identity` stays a clean primitive; `auki-keystore` (or named per-consumer) handles persistence. Decision deferred until Console actually needs it.

---

## Mnemonic seed phrases (BIP39)

For backup UX, BIP39 mnemonic phrases ("witch collapse practice feed shame open …") are conventional. 12 or 24 words encoding the 32-byte seed.

Useful for human-portable backup (write it on paper, type it back in). Adds a wordlist dep. Doesn't change the wallet primitive — it's a serialization format for the seed.

Same decision shape as encrypted-at-rest: lives in a downstream crate, not in `auki-identity` itself. Parked.

---

## Signature scheme version

`CreationCert`'s signing bytes prefix is `"auki.creation-cert.v1\n"`. v1 is the only version today.

When v2 inevitably arrives (e.g. add a `not_after_ns` expiry field, add a `usage_scope` enum), the prefix changes; old verifiers fail signature check and reject; new verifiers handle both.

The interesting question is whether `CreationCert` itself becomes an enum (`V1 { ... } | V2 { ... }`) or stays a struct with new optional fields. Enum is more honest about wire-format breakage; struct with options is more ergonomic. Defer until v2 has a real reason to exist.
