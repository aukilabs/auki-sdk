# auki-network-browser-wasm/src — sprint

## Now

Build the first SDK-owned browser-to-native protocol probe:

- native WebRTC Direct listener: implemented in `auki-network` behind `browser_probe`
- browser wasm libp2p peer: implemented behind `browser_libp2p`
- browser dial export: `dialBrowserProbe(seed, address, payload)` parses `/p2p/<peer-id>`, dials through SDK libp2p WebRTC Direct, and returns a UI-friendly result shape
- browser-to-native smoke: implemented in `scripts/browser_probe_smoke.html` and `scripts/smoke_browser_probe.mjs`, currently verified against local Chrome via `playwright-core`

## Next

Lift the verified probe transport into `auki-domain-browser` as the transport backing for Domain join, participant metadata, sensor catalogs, and media streams.
