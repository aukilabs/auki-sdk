# Build with an existing Auki protocol

This quickstart runs the existing Portable Echo protocol between two native
Rust peers. The application chooses credentials, a Domain, an identity,
discovery policy, and which protocol to mount. The SDK owns authentication,
relay booking, transport, exact-peer verification, and shutdown.

For the six SDK protocols—Info, Catalog, Registry, Blob, Message, and
Stream—use the [Standard Protocol Playground](../../examples/standard-protocols/README.md).

## Prerequisites

- Rust 1.89.0 or newer
- this SDK checkout
- an Auki User with access to a dev Domain
- that Domain's UUID
- two terminals

The complete application is
[`examples/portable-echo/native/src/main.rs`](../../examples/portable-echo/native/src/main.rs).

## Run two peers

Both terminals use the same User and Domain, but each process must own a
different identity file.

In terminal A:

~~~sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-a/peer.identity'

cargo run --locked -p auki-portable-echo-native
~~~

Terminal A starts a relay-backed peer, mounts `/example/echo/1.0.0`, advertises
the mounted protocol through DDS, and prints its Peer ID. Keep it running.

In terminal B, use another identity file and select A by the Peer ID it printed:

~~~sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-b/peer.identity'

cargo run --locked -p auki-portable-echo-native -- \
  --discover '<PEER_ID printed by terminal A>'
~~~

Terminal B queries DDS for that exact Peer ID and protocol, chooses a compatible
route, authenticates the remote Peer ID and Domain, and prints:

~~~text
echo: hello from Auki
~~~

Using the same User is fine; the separate identity files produce separate Peer
IDs. Stop terminal A with Ctrl-C. Both applications close the Echo endpoint
before shutting down the peer and releasing its relay booking.

The positional `PEER_ID EXACT_ROUTE` form remains available as a manual fallback
when testing without discovery.

## What the application owns

The native host is intentionally small. Its setup is equivalent to:

~~~rust,ignore
use auki_portable_echo::EchoEndpoint;
use auki_sdk::{
    AukiPeerBootstrap, Credentials, DdsTrackerMode, DomainSelection,
};

let bootstrap = AukiPeerBootstrap::dev(
    Credentials::user_password(email, password),
)
.await?
.with_dds_tracker(DdsTrackerMode::DiscoverAndAdvertise);

let peer = bootstrap
    .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
    .await?;

let echo = EchoEndpoint::mount(peer.protocols())?;
~~~

The application then discovers or otherwise obtains a remote Peer ID and route,
and calls the existing protocol API:

~~~rust,ignore
let receipt = echo
    .send_exact(remote_peer_id, remote_route, b"hello from Auki")
    .await?;
~~~

`send_exact` treats the route as a hint. The operation succeeds only after the
running peer verifies the expected remote Peer ID and Domain.

Cleanup follows ownership order and is attempted even after an operation fails:

~~~rust,ignore
let operation = run_application(&peer, &echo).await;
let endpoint_cleanup = echo.close().await;
let peer_cleanup = peer.shutdown().await;

operation?;
endpoint_cleanup?;
peer_cleanup?;
~~~

For a peer that only calls Echo, construct `EchoClient` instead of mounting
`EchoEndpoint`. Mounting is the explicit choice to accept inbound requests.

## Choices to make explicitly

### Credentials

Use User credentials for interactive applications:

~~~rust,ignore
Credentials::user_password(email, password)
~~~

A trusted native or headless service may use:

~~~rust,ignore
Credentials::app(app_access_key, app_secret)
~~~

Never embed an App secret in a browser, mobile binary, public repository,
container image, or log.

Robot and Compute products whose infrastructure already manages renewable
machine authority use `AukiPeer::start_external` rather than creating a second
authentication or transport stack.

### Identity

Native Rust and Python applications normally persist an identity and run one
live process or pod for that Peer ID. Web identity is currently ephemeral.
Swift exposes encoded identity bytes and leaves persistence to the application.

### Reachability

Relay-backed peers accept inbound connections and receive one TCP/WSS route
pair. A browser controller that only initiates calls can start
`AukiPeerReachabilityMode.OutboundOnly`, which skips relay booking and exposes
no inbound route.

Native applications can opt out of relay booking with:

~~~rust,ignore
let bootstrap = bootstrap.without_relay();
~~~

Without a listener and advertised direct route, that native peer is also
outbound-only. Direct inbound operation requires an application-supplied,
dialable route.

### Discovery

DDS discovery is opt-in:

- `DiscoverOnly` queries DDS but does not publish the local peer.
- `DiscoverAndAdvertise` queries DDS and maintains a short-lived advertisement
  containing current routes and mounted protocol IDs.

Reachability and discovery are separate. A relay-backed peer may remain private,
and an outbound-only peer may discover and call others. An outbound-only Web
peer cannot advertise because it has no route for another peer to dial.

Read [Discover peers](discovery.md) for the lookup API and trust boundary.

## Use the same protocol elsewhere

Portable Echo implements its wire contract once in Rust and uses thin host
adapters:

| Runtime | Run |
| --- | --- |
| Web/Wasm | [Open two browser tabs](../../examples/portable-echo/web/README.md) |
| Python | [Run two Python peers](../../examples/portable-echo/python/README.md) |
| Swift/iOS | [Build the iOS app](../../examples/portable-echo/swift/README.md) |

The host language owns UI and lifecycle wiring. Rust continues to own protocol
framing, limits, authentication, and transport.

Next, read [Author a portable protocol](authoring-protocols.md) to create a
product protocol with the same shape.
