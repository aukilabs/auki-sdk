# auki-network-browser-wasm

Browser / WASM libp2p transport probe — `wasm-bindgen`-exposed surface that lets a browser peer derive a `PeerId` from a wallet seed and (behind the `browser_libp2p` feature) participate in a session through a minimal `ClusterManager` adapter.

A v0 exploration crate. The cluster runtime support is gated behind a feature flag and the broader browser-side stream/control protocol surface is still being scoped.

**Status:** WIP (v0.0.0).

## Public surface

- `sdkName() -> String`
- `peerIdFromSeed(seed: &[u8]) -> Result<String, JsValue>`
- Behind `browser_libp2p`: browser-flavored `ClusterManager` bootstrap + `BrowserSessionParticipant` / `BrowserMediaPresence` / `BrowserRosterSnapshot` / `BrowserSessionSensor` glue.

## Depends on

- [`auki-identity`](../auki-identity) — for Wallet → PeerId derivation.
- [`auki-network`](../auki-network) — for the libp2p substrate.
- [`auki-domain`](../auki-domain) (optional, behind `browser_libp2p`) — for the cluster lifecycle facade.
