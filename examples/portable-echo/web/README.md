# Portable echo — Web/Wasm adapter

This example mounts the same Rust echo protocol used by the native example on
an authenticated browser peer. JavaScript sees a small `BrowserEchoServer`;
credentials, DDS authority, relay booking, libp2p, authenticated streams, and
echo framing stay inside Rust/Wasm.

The browser Peer ID is intentionally ephemeral in v0.1. Every call to
`BrowserEchoServer.startDev(...)` creates a fresh identity and relay route.
Reloading the page therefore creates a new peer.

The first interoperability proof runs the browser as the echo server. Its
`tcpRoute` is suitable for the existing native example, while `wssRoute` is
available to another browser peer.

Build the package with:

```sh
wasm-pack build examples/portable-echo/web --target web --out-dir pkg-web --dev
```

The local browser/native smoke harness is added separately so this adapter
remains one reviewable protocol-mounting change.
