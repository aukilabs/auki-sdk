# Sprint — auki-identity-swift

Closing the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now

PR A landed: thin scaffolding host, upstream `swift-bindings` feature on `auki-identity` (Wallet) and `auki-network` (PeerIdentity only). Surface: `Wallet::{new, from_seed, seed, wallet_id_str}`, `PeerIdentity::{from_wallet, peer_id_string}`. Host build + tests green. iOS XCFramework script in place; validate by running `build-xcframework.sh`.

## Next

In priority order:

1. **PR B — `auki-network-swift` expansion.** Annotate `NetworkRuntime`, the stream surface, `PeerLivenessEvent` callback. Adds `PeerId`/`Multiaddr` UniFFI custom types — once those land, the `PeerIdentity::peer_id_string()` helper this crate currently exposes may be replaceable by a direct `peer_id() -> PeerId` method (since `PeerId` would auto-marshal as a String via the custom type). Either way, the helper stays as the v0 surface.
2. **PR C — `auki-domain-swift`.** ClusterManager bootstrap consumes `PeerIdentity` arguments, so this crate's surface is the gate for that consumption.
3. **Spec 2 — iosapp wiring.** Keychain helper consumes `Wallet::{from_seed, generate, seed}` to implement the iOS analogue of `auki-identity::load_or_mint_seed`.

## Open Items

See [`parking_lot.md`](../parking_lot.md). Nothing blocks current consumers.

## Out Of Scope

- Cluster lifecycle / peer enumeration — `auki-domain-swift` (PR C).
- Stream / audio surface — `auki-network-swift` expansion (PR B).
- `Signature` / `verify` / `CreationCert` / `derive_child` / typed `WalletId` — defer until a real iosapp feature needs them.
