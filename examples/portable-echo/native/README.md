# Portable echo in native Rust

This is the smallest complete native application built from the high-level
`AukiPeer` facade and the single portable echo protocol crate. It authenticates
a User, authorizes one exact Domain, starts a relay-reachable peer, mounts the
crate's endpoint, and shuts the endpoint and peer down in order.

Application developers should start with
[Build with an existing protocol](../../../docs/p2p/getting-started.md).
Protocol authors can continue with the
[one-crate authoring workflow](../../../docs/p2p/authoring-protocols.md). This
file focuses only on running the native host.

The default application contains the developer-facing path only. The more
verbose machine-readable executable used by the protected Web/native smoke test
is kept separately as `auki-portable-echo-interop`.

## Run two peers

Both peers need User access to the same dev Domain and separate identity files.
`Identity::load_or_create` creates the parent directory and reuses the same Peer
ID on later launches.

In terminal A:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-a/peer.identity'
cargo run --locked -p auki-portable-echo-native
```

The application prints its Peer ID and confirmed TCP relay route, then serves
echo requests until Ctrl-C:

```text
peer: 12D3KooW...
route: /dns4/relay.dev.aukiverse.com/tcp/443/p2p/.../p2p-circuit/p2p/12D3KooW...
serving; press Ctrl-C to stop
```

In terminal B, use a different identity file and pass both values printed by A:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-b/peer.identity'
cargo run --locked -p auki-portable-echo-native -- \
  '<PEER_ID from terminal A>' \
  '<complete route from terminal A>'
```

Terminal B sends `hello from Auki`, prints the echoed payload, closes its
protocol endpoint, and shuts down its peer. Stop terminal A with Ctrl-C; it
performs the same ordered cleanup.

The reference program intentionally shows User credentials only. A trusted
native or headless application can instead authenticate with
`Credentials::app(access_key, secret)`. Never embed an App secret in a browser
or distributed client.

## Web/native interoperability proof

The protected smoke test needs a separate environment contract, peer-card
output, inbound event markers, flexible credential parsing, and keep-running
controls. Those concerns remain in the `auki-portable-echo-interop` binary
instead of obscuring the copyable app. The binary is test machinery; run it
through the [Web/Wasm smoke command](../web/README.md#run-the-protected-direction-proof),
which supplies its required `AUKI_STATE_DIR` and other controls:

```sh
cd examples/portable-echo/web
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
npm run smoke:dev
```

That proof covers browser-to-browser in both directions, native-to-browser, and
browser-to-native using each remote peer's exact advertised route. Peers do not
need to receive reservations on the same relay.

The manual values and smoke-test peer cards are application-owned exchange
records. The SDK does not automatically discover peers or publish their routes
in `0.1`.
