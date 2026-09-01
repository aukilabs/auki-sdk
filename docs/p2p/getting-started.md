# Build with an existing Auki protocol

This quickstart consumes the existing `auki-portable-echo` protocol crate. The
application does not implement framing, libp2p, authentication, relay booking,
or protocol cleanup.

By the end, two native peers will have distinct persistent Peer IDs, authority
for the same Domain, confirmed TCP/WSS relay-route pairs, and one authenticated
echo exchange. The same protocol crate also powers the tiny Web, Python, and
Swift/iOS hosts.

To exercise the SDK-owned Info, Catalog, Registry, Blob, Message, and Stream
families instead, use the
[standard protocol playground](../../examples/standard-protocols/README.md).

## Prerequisites

- Rust `1.89.0` or newer
- this SDK checkout
- an Auki User account with access to a DDS Domain
- that Domain's UUID
- two terminals

The copyable native host is
[`examples/portable-echo/native/src/main.rs`](../../examples/portable-echo/native/src/main.rs).
Its protocol dependency is the single
[`auki-portable-echo`](../../examples/portable-echo/README.md) crate.

## 1. Start the serving peer

From the SDK repository root, configure terminal A:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-a/peer.identity'
cargo run --locked -p auki-portable-echo-native
```

Replace the UUID with an accessible Domain. `Identity::load_or_create` creates
the identity file's parent directory and reuses the same Peer ID on later
launches.

Startup returns only after authority, transport, and one relay reservation are
ready. That one provider slot supplies both TCP and WSS circuit routes; the
native host uses and prints its TCP route. The host then mounts
`/example/echo/1.0.0`:

```text
peer: 12D3KooW...
route: /dns4/relay.dev.aukiverse.com/tcp/443/p2p/.../p2p-circuit/p2p/12D3KooW...
serving; press Ctrl-C to stop
```

Keep it running and copy the complete `peer` and `route` values.

## 2. Dial it from a second peer

In terminal B, use a different identity file and pass A's printed values:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-b/peer.identity'
cargo run --locked -p auki-portable-echo-native -- \
  '<PEER_ID from terminal A>' \
  '<complete route from terminal A>'
```

Using the same User is fine; the separate identity files create distinct Peer
IDs. Terminal B authenticates A's expected Peer ID and Domain through the
supplied route, then prints:

```text
echo: hello from Auki
```

It closes the echo endpoint and peer in order. Press Ctrl-C in terminal A to do
the same and release its relay booking.

Environment variables are convenient for this experiment, not a production
secret-management strategy.

## The application code

An application imports the endpoint from the existing protocol crate:

```rust
use auki_portable_echo::EchoEndpoint;
use auki_sdk::{AukiPeerBootstrap, Credentials, DomainSelection};
```

It then chooses credentials, Domain, native identity path, whether to mount the
protocol, and the remote peer information. Rust's bootstrap facade owns the
mechanical authentication, identity proof, authority preparation, and peer
startup sequence. The failure-safe lifecycle is:

```rust
let bootstrap = AukiPeerBootstrap::dev(
    Credentials::user_password(email, password),
)
.await?;
let peer = bootstrap
    .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
    .await?;

let operation = async {
    let echo = EchoEndpoint::mount(peer.protocols())?;
    let exchange = echo
        .send_exact(remote_peer_id, remote_route, b"hello from Auki")
        .await;
    let endpoint_cleanup = echo.close().await;

    let receipt = exchange?;
    endpoint_cleanup?;
    Ok::<_, anyhow::Error>(receipt)
}
.await;

let peer_cleanup = peer.shutdown().await;
let receipt = operation?;
peer_cleanup?;
```

The result is captured before cleanup, so an exchange failure still attempts
endpoint close and every outcome still attempts peer shutdown. Endpoint close
always precedes peer shutdown. The
[native reference](../../examples/portable-echo/native/src/main.rs) uses this
same pattern around both client and serving modes.

The `auki-portable-echo` crate owns its immutable protocol ID, bounded wire
conversation, registration, deadlines, exact-route send, inbound events, and
stream cleanup. `AukiPeerBootstrap` and `AukiPeer` own the API/DDS exchange,
renewable authority, authenticated transport, relay booking, route validation,
fencing, and peer shutdown.

## Use the same protocol from Web

The Web host compiles the same `auki-portable-echo` crate into Wasm. JavaScript
only logs in a User, selects a Domain, starts an ephemeral `AukiPeer`, constructs
`AukiEcho`, and supplies the remote Peer ID plus WSS route. It closes
`AukiEcho` before shutting down the peer.

Echo needs its small `AukiEcho` adapter because it is an application protocol.
SDK-owned Info, Catalog, Registry, Blob, Message, and Stream bindings expose
both client and serving roles. Each is a thin host adapter over the same Rust
protocol implementation.

Run the [browser echo app](../../examples/portable-echo/web/README.md#run-the-web-app)
to try that surface in two tabs. The protected four-direction smoke test drives
that same page while keeping its test machinery out of the application.

## Use the same protocol from Python

The Python extension statically links the generic native peer facade with the
same `auki-portable-echo` crate. Live Rust handles never cross separately
loaded extension modules, and Python does not manage Tokio or raw streams.

The application flow stays small:

```python
session = await AukiSession.login_dev(email, password)
peer = await session.start_peer(domain_id, identity_file)
try:
    echo = await AukiEcho.mount(peer)
    try:
        receipt = await echo.send_exact(remote_peer_id, remote_route, payload)
    finally:
        await echo.close()
finally:
    await peer.shutdown()
```

Run the [Python echo app](../../examples/portable-echo/python/README.md) to try
Python-to-Python or either Python/native direction through exact relay routes.

## Use the same protocol from Swift/iOS

The Apple artifact statically links the generic peer facade and the same Rust
echo endpoint. Swift does not implement authentication, relay booking,
libp2p, or protocol framing:

```swift
let session = try await AukiSession.loginDev(email: email, password: password)
let peer = try await session.startPeer(
    domainId: domainId,
    identity: AukiPeerIdentity.generate()
)
let echo = try await AukiEcho.mount(peer: peer)

let card = try peerCardFromJson(json: remoteCard)
let receipt = try await echo.sendExact(
    target: nativeTarget(card: card),
    payload: Data("hello from Swift".utf8)
)

try await echo.close()
try await peer.shutdown()
```

The example identity is ephemeral. An application that requires a stable Peer
ID may store `AukiPeerIdentity.encoded()` and restore it with `fromEncoded`.
Run the [Swift/iOS echo app](../../examples/portable-echo/swift/README.md) for
the generated-project commands and simulator test.

To consume the built-in protocols instead, run the
[Swift/iOS standard playground](../../examples/standard-protocols/swift/README.md).
It mounts all six families through `AukiStandardProtocols` and keeps endpoint
shutdown ahead of peer shutdown.

## App credentials

A trusted native or headless process can replace User credentials with:

```rust
Credentials::app(app_access_key, app_secret)
```

Everything else uses the same `AukiPeerBootstrap` and `AukiPeer` lifecycle.
Never embed an App secret in a browser, mobile binary, public repository,
container image, or log.

## Relay is not discovery

Relay allocation made A reachable, but B still needed A's expected Peer ID and
complete route. `0.1` does not automatically discover peers or publish routes.
Applications obtain that information from configuration, a product control
plane, or a manually shared record.

The route remains an untrusted hint. An exchange succeeds only after the SDK
authenticates the expected remote Peer ID in the selected Domain.

## Continue

- [Author one portable protocol crate](authoring-protocols.md).
- Run the [Python echo host](../../examples/portable-echo/python/README.md).
- Run the [Swift/iOS echo host](../../examples/portable-echo/swift/README.md).
- Run the [standard protocol matrix](../../examples/standard-protocols/README.md#protected-four-peer-matrix)
  for all six SDK protocol families across Native, Python, and two distinct
  Browser peers, including browser-to-browser in both directions.
- Use [`auki-p2p`](../../crates/auki-p2p/README.md) directly only when building a
  custom runtime or transport integration.
