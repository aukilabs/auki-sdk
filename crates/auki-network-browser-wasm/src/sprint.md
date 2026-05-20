# auki-network-browser-wasm/src — sprint

## Now

Build the first SDK-owned browser-to-native protocol probe:

- native WebRTC Direct listener: implemented in `auki-network` behind `browser_probe`
- browser wasm libp2p peer: implemented behind `browser_libp2p`
- browser dial export: `dialBrowserProbe(seed, address, payload)` parses `/p2p/<peer-id>`, dials through SDK libp2p WebRTC Direct, and returns a UI-friendly result shape

## Next

Run the browser-to-native smoke harness. Once the probe stream opens in Chromium, lift it into `auki-domain-browser` as the transport backing for Domain join, participant metadata, sensor catalogs, and media streams.
