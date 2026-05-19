# auki-network-browser-wasm/src — sprint

## Now

Build the first SDK-owned browser-to-native protocol probe:

- choose the native probe listener transport, starting with WebRTC Direct
- instantiate a browser wasm libp2p peer with the canonical Auki PeerId
- dial the native probe and open one SDK-owned named protocol stream

## Next

After the probe stream opens, lift it into `auki-domain-browser` as the transport backing for Domain join, participant metadata, sensor catalogs, and media streams.
