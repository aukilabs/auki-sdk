# Parking lot — auki-identity-swift

Open questions specific to the Swift identity binding.

---

## `WalletId` is hidden behind `wallet_id_str()`

The upstream `WalletId(pub String)` tuple struct isn't a UniFFI `Record` candidate without an upstream refactor (Records require named fields). PR A's binding exposes a `wallet_id_str() -> String` helper instead. If iosapp wants typed handling on the Swift side (`struct WalletId: Hashable { let raw: String }`) that's a thin Swift-side wrapper — no Rust change needed. Revisit if a real consumer needs typed Swift treatment.

## Async-shaped Swift API vs. `-py` sync precedent _(inherited from auki-network-swift)_

Same standing flag-for-human-confirmation as the existing `auki-network-swift` parking lot. Swift's async-await + iOS main-thread rules mean the binding exposes async where the upstream is async; the `-py` precedent is sync. Confirm before any reversal.

## `Wallet`'s other methods (sign, derive_child, sign_canonical_json) not exposed yet

The binding only exposes the v0-essential subset (`new`, `from_seed`, `seed`, `wallet_id_str`). Lift the others into the annotated impl block when a real iosapp feature needs them.

## `from_seed` returns `Result<Arc<Wallet>, IdentityError>` not `Wallet`

UniFFI 0.31's constructor contract for `uniffi::Object` requires `Arc<Self>`, and `Vec<u8>` (not `&[u8; 32]`) is the only seed-length-checkable FFI shape. Swift sees `try Wallet.fromSeed(seed:)`. Rust callers that built Wallets via `from_seed` had to adapt to `.expect()` or `.try_into()` on a length-guaranteed seed. The signature change is permanent — see `parking_lot.md`'s sibling `auki-identity` entry for the upstream rationale.
