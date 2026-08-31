# Get started with Auki P2P in Rust

This quickstart runs the small native reference application against the Auki
dev environment. It uses the real portable echo protocol and shared adapter;
there is no second tutorial-only wire implementation.

By the end, two peers will have distinct persistent Peer IDs, authority for the
same Domain, confirmed relay routes, and one authenticated echo exchange.

## Prerequisites

- Rust `1.89.0` or newer
- this SDK checkout
- an Auki User account with access to a DDS Domain
- that Domain's UUID
- two terminals

The reference source is
[`examples/portable-echo/native/src/main.rs`](../../examples/portable-echo/native/src/main.rs).
Its developer-facing path is intentionally small; the more verbose interop
binary is protected test machinery.

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
ready. The application mounts `/example/echo/1.0.0`, then prints its public
Peer ID and confirmed native TCP circuit route:

```text
peer: 12D3KooW...
route: /dns4/relay.dev.aukiverse.com/tcp/443/p2p/.../p2p-circuit/p2p/12D3KooW...
serving; press Ctrl-C to stop
```

Keep it running and copy the complete `peer` and `route` values.

## 2. Dial it from a second peer

In terminal B, use a different identity file and pass A's two printed values:

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
IDs. Terminal B opens the exact advertised route, authenticates A's expected
Peer ID and Domain, runs the shared Rust echo conversation, and prints:

```text
echo: hello from Auki
```

It then closes the echo endpoint and the peer in order. Press Ctrl-C in terminal
A to perform the same ordered cleanup and release its relay booking.

Environment variables are convenient for this experiment, not a production
secret-management strategy.

## The application-facing code

The complete composition is:

```rust
let identity = Identity::load_or_create(identity_file)?;
let session = AuthClient::new(AuthEnvironment::dev())?
    .authenticate(Credentials::user_password(email, password))
    .await?;
let prepared = session
    .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
    .await?;
let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;

let echo = EchoEndpoint::mount(peer.protocols())?;
let receipt = echo
    .send_exact(remote_peer_id, remote_route, b"hello from Auki")
    .await?;

echo.close().await?;
peer.shutdown().await?;
```

The application chooses credentials, Domain, identity storage, protocol opt-in,
and the remote peer information. `auki-auth`, `AukiPeer`, and the shared echo
adapter own the API/DDS exchanges, renewable authority, libp2p transport, relay
booking, exact-route authentication, bounded wire conversation, and cleanup.

Production code should preserve the reference application's finally-style
cleanup: attempt endpoint close and peer shutdown even when the operation
fails.

## App credentials

A trusted native or headless process can authenticate with:

```rust
Credentials::app(app_access_key, app_secret)
```

Everything after authentication uses the same `PreparedPeer` and `AukiPeer`
lifecycle. Never embed an App secret in a browser, mobile binary, public
repository, container image, or log.

## Relay is not discovery

Relay allocation made A reachable, but B still needed A's expected Peer ID and
complete route. `0.1` does not automatically discover peers or publish routes.
Applications obtain that information from configuration, a product control
plane, or a manually shared record.

The route remains an untrusted hint. An exchange succeeds only after the SDK
authenticates the expected remote Peer ID in the selected Domain.

## Continue

- [Author a portable protocol](authoring-protocols.md) using the same split as
  echo.
- Run the [minimal Web app](../../examples/portable-echo/web/README.md#copy-the-minimal-app)
  for browser-to-browser communication.
- Run the [protected interop proof](../../examples/portable-echo/web/README.md#live-direction-proof)
  for both browser directions plus native-to-browser and browser-to-native.
- Use [`auki-p2p`](../../crates/auki-p2p/README.md) directly only when building a
  custom runtime or transport integration.
