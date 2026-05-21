# auki-identity-swift

UniFFI Swift bindings for [`auki-identity`](../../../crates/auki-identity) (`Wallet`) and the identity-shaped slice of [`auki-network`](../../../crates/auki-network) (`PeerIdentity`).

Sibling of [`auki-identity-py`](../../python/auki-identity-py); one binding crate per Rust component, no umbrella `auki-swift`. Thin scaffolding host — actual UniFFI proc-macros live on the upstream types behind the `swift-bindings` cargo feature.

## Surface (target)

```swift
let wallet = try Wallet.fromSeed(seed: data32Bytes)   // or Wallet()
let walletId = wallet.walletIdStr()
let peer = PeerIdentity.fromWallet(wallet: wallet)
let peerIdString = peer.peerIdString()                // canonical 12D3KooW…
```

| Swift type | Rust source |
|---|---|
| `Wallet` | `auki_identity::Wallet` |
| `PeerIdentity` | `auki_network::PeerIdentity` |

Out of scope at v0 (PR A): `derive_child`, `sign`/`sign_canonical_json`, `Signature`/`verify`/`CreationCert`. These stay accessible to non-Swift Rust callers via the un-exported `impl` blocks; future PRs lift them if iosapp's features need them.

## Build

Host gate:

```bash
cargo build -p auki-identity-swift
cargo test  -p auki-identity-swift
```

iOS XCFramework:

```bash
bindings/swift/auki-identity-swift/build-xcframework.sh
```

## Status

PR A of [Spec 1](../../docs/superpowers/specs/2026-05-20-sdk-swift-binding-expansion-design.md). See [`src/readme.md`](src/readme.md) for what's implemented and [`src/sprint.md`](src/sprint.md) for what's next.
