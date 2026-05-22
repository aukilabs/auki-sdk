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
- `BrowserDomainSession.joinDomain()` opens `/auki/join/0.0.1`, advertises a relay circuit address for the browser peer, and keeps a full browser libp2p peer alive after membership
- browser peers serve inbound `/auki/info/0.0.1` and `/auki/sensors/0.0.1` requests from their local participant metadata and declared sensor catalog
- after membership, browser peers fetch remote browser participant info and sensor catalogs over the native `/auki/info/0.0.1` + `/auki/sensors/0.0.1` protocols; the full-peer smoke asserts remote audio sensors come from the sensors protocol
- browser media presence operations update local intent and return `ok` after join; real audio bytes remain a later stream payload

Not yet implemented:

- real audio byte capture/playback over SDK streams
- browser-created native Manager role
