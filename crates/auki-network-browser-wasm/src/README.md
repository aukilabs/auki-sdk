# auki-network-browser-wasm/src

Implementation status for the browser/WASM networking probe.

Currently implemented:

- wasm crate scaffold
- `sdkName()` smoke export
- canonical seed-to-PeerId export
- Node import smoke script for `wasm-pack --target nodejs`
- rust-libp2p browser transport feature compile probe
- `dialBrowserProbe(seed, address, payload)` behind `browser_libp2p`, using `libp2p-webrtc-websys` to send one SDK-owned `/auki/browser-probe/0.0.1` request to a native WebRTC Direct listener
- browser-to-native smoke harness that serves `pkg-web`, drives Chromium through `playwright-core`, and verifies the probe payload round-trip

Not yet implemented:

- `auki-domain-browser` integration for Domain join, participant metadata, sensor catalogs, and media streams
