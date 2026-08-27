# auki-network-browser-wasm

Browser / WASM libp2p transport probe for the prior Manager-era package line.

> **Compatibility:** this crate is excluded from the active workspace and is
> not wire-compatible with the authenticated Stage 1 Rust/Python Domain. Use
> it only with its pinned prior SDK line; it cannot join Stage 1 peers. The
> browser authenticated-engine migration is a later platform stage.

Its historical `wasm-bindgen` surface lets a browser peer derive a `PeerId`
from a wallet seed and, behind `browser_libp2p`, participate through a minimal
`ClusterManager` adapter.

A v0 exploration crate. The cluster runtime support is gated behind a feature flag and the broader browser-side stream/control protocol surface is still being scoped.

**Status:** Legacy/excluded WIP (v0.0.0).

## Public surface

- `sdkName() -> String`
- `peerIdFromSeed(seed: &[u8]) -> Result<String, JsValue>`
- Behind `browser_libp2p`: browser-flavored `ClusterManager` bootstrap + `BrowserSessionParticipant` / `BrowserMediaPresence` / `BrowserRosterSnapshot` / `BrowserSessionSensor` glue. Browser joins prefer browser-dialable Manager addresses (`/webrtc-direct`, WebSocket, or WebSocket relay circuits) from Discovery and use WebSocket + Circuit Relay v2 transport when joining through a Domain Relay.

## Depends on

- [`auki-identity`](../auki-identity) — for Wallet → PeerId derivation.
- [`auki-network`](../auki-network) — for the libp2p substrate.
- [`auki-domain`](../auki-domain) (optional, behind `browser_libp2p`) — for the cluster lifecycle facade.
