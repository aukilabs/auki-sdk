# auki-network-browser-wasm/src

Implementation status for the browser/WASM networking probe.

Currently implemented:

- wasm crate scaffold
- `sdkName()` smoke export
- canonical seed-to-PeerId export
- Node import smoke script for `wasm-pack --target nodejs`
- exact `browser_libp2p` compile blocker captured in `parking_lot.md`

Not yet implemented:

- browser-page import smoke script
- rust-libp2p browser feature compile probe
- native browser-compatible probe listener
- browser-to-native protocol dial
