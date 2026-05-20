# auki-network-browser-wasm/src

Implementation status for the browser/WASM networking probe.

Currently implemented:

- wasm crate scaffold
- `sdkName()` smoke export
- canonical seed-to-PeerId export
- Node import smoke script for `wasm-pack --target nodejs`
- rust-libp2p browser transport feature compile probe
- `dialBrowserProbe(seed, address, payload)` behind `browser_libp2p`, using `libp2p-webrtc-websys` to send one SDK-owned `/auki/browser-probe/0.0.1` request to a native WebRTC Direct listener

Not yet implemented:

- browser-page import smoke script
- browser-to-native protocol smoke harness
