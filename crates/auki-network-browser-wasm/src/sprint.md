# auki-network-browser-wasm/src — sprint

## Now

The browser peer now runs as a real SDK libp2p peer:

- native WebRTC Direct listener: implemented in `auki-network` behind `browser_probe`
- browser wasm libp2p peer: implemented behind `browser_libp2p`
- browser dial export: `dialBrowserProbe(seed, address, payload)` parses `/p2p/<peer-id>`, dials through SDK libp2p WebRTC Direct, and returns a UI-friendly result shape
- browser-to-native smoke: implemented in `scripts/browser_probe_smoke.html` and `scripts/smoke_browser_probe.mjs`, currently verified against local Chrome via `playwright-core`
- Domain join: `BrowserDomainSession.joinDomain()` opens native `/auki/join/0.0.1`, advertises a relay circuit address, applies membership, and keeps the browser swarm alive
- Catalog parity: browser peers serve and fetch native `/auki/info/0.0.1` + `/auki/sensors/0.0.1`; the full-peer smoke requires remote audio sensors to arrive through `/auki/sensors/0.0.1`

## Next

Use the same full-peer path for direct browser audio over `/auki/stream/0.1.0`, then stage the generated browser artifacts for Park integration.
