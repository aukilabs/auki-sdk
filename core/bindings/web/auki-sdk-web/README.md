# Auki networking for Web

Rust/Wasm bindings expose `AukiUserSession` and `AukiPeer` to JavaScript.
Browser peers use ephemeral identities and dial WSS relay routes.

Requires Rust 1.89+, wasm-pack 0.13.1, and `wasm32-unknown-unknown`.
From the SDK repository root:

~~~sh
rustup target add wasm32-unknown-unknown
wasm-pack build core/bindings/web/auki-sdk-web --target web --out-dir pkg -- --locked
~~~

Import the generated module and await its default initialization function before
using the bindings. Choose `OutboundOnly` for a calling peer or `RelayBacked`
to accept inbound calls.

For an app that supplies a custom protocol in the same Wasm module, run
[Web Echo](../../../examples/portable-echo/web/README.md).
See [authentication](../../../../docs/how-to/authenticate.md) and the
[networking reference](../../../../docs/reference/networking.md).
