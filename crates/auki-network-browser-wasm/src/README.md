# auki-network-browser-wasm/src

Implementation status for the browser/WASM networking probe.

Currently implemented:

- wasm crate scaffold
- `sdkName()` smoke export
- canonical seed-to-PeerId export
- Node import smoke script for `wasm-pack --target nodejs`
- rust-libp2p browser transport feature compile probe

Not yet implemented:

- browser-page import smoke script
- native browser-compatible probe listener
- browser-to-native protocol dial
