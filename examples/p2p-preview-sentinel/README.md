# Auki P2P Preview Sentinel

Development-only native Sentinel peer for exercising the RFC-first `auki-p2p`
runtime with browser-reachable transports.

The example:

- starts a native `AukiNode` with loopback WebRTC Direct and WebSocket relay listeners
- registers a deterministic demo domain
- publishes the shared `auki.sensor.rgb_camera.preview` offer profile
- serves lifecycle, offer-catalog, Get, and Subscribe through `AukiServeRuntime`
- produces one generated JPEG preview stream and fans the latest frames out to Get and Subscribe
- prints the browser bootstrap record and compact P2P state

Run:

```sh
cargo run -p auki-p2p-preview-sentinel -- --bootstrap-json /tmp/auki-preview-bootstrap.json
```

Smoke-test startup without keeping listeners alive:

```sh
cargo run -p auki-p2p-preview-sentinel -- --once --bootstrap-json /tmp/auki-preview-bootstrap.json
```

Camera capture is intentionally not wired in this first slice. The CLI keeps
`--source generated` explicit so the later camera adapter can plug into the
same offer/profile path without changing the protocol surface.
