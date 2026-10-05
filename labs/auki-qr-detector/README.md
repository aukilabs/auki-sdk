# QR Detector

Experimental QR detection backed by QR Lab. `QrDetector::bind_product` binds a
typed camera Buffer Product to a Detector Component and exposes a `detections`
Observable. Applications can retain those results in a separate Buffer.

The input accepts JPEG or RGB8 `VideoFrame` observations whose metadata matches
the pinned Camera contract. `replace_product` explicitly selects a compatible
replacement Product; it does not automatically follow camera reconfiguration.
`bind_product_operable` optionally exposes that selection as an authorized
typed operation.

The existing `RegisteredQrDetector` adapter also supports Session Sensor Logs
and asynchronous camera streams through `start` and `start_stream`.

See [Component protocols](../../docs/reference/component-protocols.md) for
network access and its current limits.

```sh
cargo test --locked -p auki-qr-detector
```

This crate still includes the native recording integration through
`auki-session`. That dependency enables Tokio filesystem and multithreaded
runtime features, so the complete QR crate does not currently compile for
`wasm32-unknown-unknown`. The separate Component runtime and protocol adapter
are checked for that target; this is not a browser-port of the QR integration.
