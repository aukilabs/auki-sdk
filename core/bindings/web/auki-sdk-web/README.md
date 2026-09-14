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

## Domain data without a peer

The same session exposes `domains()` for ordinary Domain discovery and portal
metadata, and `data(domain_id)` for data operations and pose reads. Close the data
client before closing the shared session. See [Domain data](../../../../docs/how-to/domain-data.md)
for permission boundaries, streaming limits and migration from Posemesh clients.

`AukiUserSession.loginDev(email, password, clientId?)` accepts a persistent
installation ID; existing calls remain valid. Data methods accept an optional
`AbortSignal`. `readTo` awaits a chunk sink; `writeStream` awaits a bounded source.
Both callbacks receive a signal so they can stop their own pending work.

The [Blob/File round trip](examples/domain-data.ts) accepts an existing session
and chosen Domain, reads portals/poses, uploads a file, streams it to the supplied
destination and deletes its unique test record. Import the generated SDK in your
app, await its initialization, then pass your session and selected file to this
helper. This example contacts the session's configured services.

Compile the binding, declarations and examples locally:

```sh
cd core/bindings/web/auki-sdk-web
npm ci
npm run check
```

From the repository root, run the offline Chromium integration tests with
`WASM_BINDGEN_TEST_RUNNER` pointing to the matching wasm-bindgen 0.2.121 runner:

```sh
bash test-support/run-domain-data-browser-tests.sh
```
