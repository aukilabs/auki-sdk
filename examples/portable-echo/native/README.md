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

## Run two discoverable peers

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

The application receives one atomic TCP/WSS route pair from its relay slot,
advertises the mounted Echo protocol through DDS, refreshes current Echo
candidates, then serves requests until Ctrl-C:

```text
peer: 12D3KooW...
route: /dns4/relay.dev.aukiverse.com/tcp/443/p2p/.../p2p-circuit/p2p/12D3KooW...
discovered Echo peers (untrusted until exact dial):
serving; use --discover <PEER_ID> from another terminal or press Ctrl-C to stop
```

In terminal B, use a different identity file and select A by its discovered
Peer ID. No route is pasted:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-b/peer.identity'
cargo run --locked -p auki-portable-echo-native -- \
  --discover '<PEER_ID printed by terminal A>'
```

Terminal B polls the exact `/example/echo/1.0.0` advertisements until A's
mounted endpoint is visible, tries its native-compatible routes, sends `hello
from Auki`, closes its protocol endpoint, and shuts down its peer. Stop terminal
A with Ctrl-C; it performs the same ordered cleanup.

`DiscoverAndAdvertise` is the default. Set
`AUKI_DISCOVERY_MODE=discover_only` before startup when this peer should find
others but remain absent from DDS. For debugging, the legacy positional
`PEER_ID EXACT_ROUTE` form remains the clearly labeled manual fallback.

The reference program intentionally shows User credentials only. A trusted
native or headless application can instead authenticate with
`Credentials::app(access_key, secret)`. Never embed an App secret in a browser
or distributed client.

## Web/native interoperability proof

The protected smoke test needs a separate environment contract, discovery
polling, inbound event markers, flexible credential parsing, and keep-running
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

The protected proof now polls [DDS discovery](../../../docs/p2p/discovery.md)
for the exact mounted Echo protocol and selects candidates before every dial;
it does not exchange peer cards or route strings between runtimes.
