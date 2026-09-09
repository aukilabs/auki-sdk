# Auki networking for Web

Use `AukiUserSession` and `AukiPeer` from JavaScript to connect a browser app.
Each start creates a new Peer ID. Browsers connect through WSS relay addresses.

Requires Rust 1.89+, wasm-pack 0.13.1, and `wasm32-unknown-unknown`.
From the SDK repository root:

~~~sh
rustup target add wasm32-unknown-unknown
wasm-pack build core/bindings/web/auki-sdk-web --target web --out-dir pkg -- --locked
~~~

Import the generated module and await its default initialization function before
using the bindings. Choose `AukiPeerReachabilityMode.OutboundOnly` to make
outgoing requests, or `RelayBacked` to also accept incoming requests.

For an app that sends and receives messages, run
[Web Echo](../../../examples/portable-echo/web/README.md).
Compile custom Rust protocols into the same Wasm module as the SDK. See
[custom protocols](../../../../docs/how-to/protocols.md).
