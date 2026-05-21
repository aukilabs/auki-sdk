# Swift Bindings

Swift-facing SDK packages live here. The structure preserves the existing per-component package names: UniFFI wrappers sit alongside (eventually) generated SwiftPM artifacts, while Rust SDK implementation crates remain in [`../../crates`](../../crates) — same per-language rule the Python bindings follow.

| Package | What it does |
|---|---|
| [`auki-identity-swift`](auki-identity-swift) | UniFFI Swift bindings for `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. Thin scaffolding host; UniFFI proc-macros live on the upstream types behind a `swift-bindings` cargo feature. |
| [`auki-network-swift`](auki-network-swift) | UniFFI Swift bindings for `auki-network`'s Discovery HTTP client (Stage 1). Async `DiscoveryClient`, `ClusterEntry`/`CreateClusterOutcome`, flattened `DiscoveryError`. Stream/audio is Stage 2; cluster join + peer enumeration will arrive as a future `auki-domain-swift`. |
