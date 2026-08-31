# Author a portable Auki protocol

Keep one immutable wire contract and its small `AukiPeer` integration in one
Rust crate. Native and Web hosts should consume that same crate; platform code
should not reimplement the conversation.

[`examples/portable-echo`](../../examples/portable-echo/README.md) is the
smallest complete reference.

## Recommended shape

```text
my-protocol/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── wire.rs       ID, messages, bounded codec, conversation
│   └── endpoint.rs   Client, Endpoint, Provider, deadlines, cleanup
└── tests/
    └── locked_wire.rs
```

These are modules in one crate, not separate packages an application must
assemble.

## Assign one immutable ID

Product IDs use `/<namespace>[/<name>...]/<version>`:

```text
/my-product/robot-state/1.0.0
```

The top-level `/auki/` namespace is reserved for SDK-owned protocols. Product
protocols do not need an `/auki-p2p/` prefix.

Change the ID when any observable wire property becomes incompatible:

- framing or encoding;
- request/response ordering;
- message schema;
- size or round bounds; or
- success and failure semantics.

The Cargo package version and protocol ID solve different problems. A bug fix
may change the package version without changing the wire.

## Keep wire code transport-neutral

The wire module owns:

- the exact protocol ID;
- request, response, and event types;
- fixed byte and round limits;
- portable async read/write functions; and
- validation before allocation or application delivery.

```rust,ignore
pub const PROTOCOL_ID: &str = "/my-product/robot-state/1.0.0";
pub const MAX_FRAME_BYTES: u32 = 64 * 1024;

pub async fn run_client<S>(
    stream: &mut S,
    request: Request,
) -> Result<Response, WireError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    // Write one bounded request and validate one bounded response.
}

pub async fn run_server<S>(stream: &mut S) -> Result<Accepted, WireError>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    // Read, validate, and answer the exact conversation.
}
```

Do not put credentials, DDS/DMS clients, libp2p setup, wasm-bindgen, UI, or
filesystem policy in this module.

Lock the ID and representative encoded bytes. Test empty, malformed,
mismatched, truncated, oversized, and out-of-order input.

## Split outbound Client from inbound Endpoint

The public runtime API should be boring:

- `MyClient` is cloneable and performs outbound operations.
- `MyEndpoint` owns inbound registration and close.
- `MyProvider` supplies data or admission decisions to inbound handlers.

```rust,ignore
pub const MY_MAX_CONCURRENCY: usize = 16;

pub fn my_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(PROTOCOL_ID, MY_MAX_CONCURRENCY, MAX_FRAME_BYTES)
}
```

Use prefixed spec and bound names in shared crates. Generic names such as
`protocol_spec` are reasonable only when the crate contains exactly one
protocol and cannot collide.

### Client

The Client stores `AukiPeerProtocols` and exposes:

- configured-route methods on native Rust when useful; and
- exact-route methods for the portable native/Web surface.

```rust,ignore
#[derive(Clone)]
pub struct MyClient {
    protocols: AukiPeerProtocols,
}

impl MyClient {
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    pub async fn request_exact(
        &self,
        expected_peer: PeerId,
        route: Multiaddr,
        request: Request,
    ) -> Result<Response, MyError> {
        let mut stream = self
            .protocols
            .open_exact(expected_peer, route, PROTOCOL_ID)
            .await?;
        // Apply deadlines, run the wire client, and close the stream.
    }
}
```

The route is an untrusted hint. `open_exact` authenticates the expected Peer ID
inside the running peer's Domain before the wire conversation begins.

### Provider

Pass the verified requester to application policy:

```rust,ignore
pub trait MyProvider {
    fn respond(
        &self,
        requester: &AuthenticatedPeer,
        request: Request,
    ) -> Response;
}
```

Native Provider traits normally require `Send + Sync + 'static`. Browser
providers may be executor-local and use `Rc`; keep this target difference
inside the endpoint module.

Authentication is not authorization. Being in the same Domain does not
automatically permit a command or robot capability.

### Endpoint

The Endpoint registers the exact spec, runs bounded inbound handlers, and owns
registration shutdown:

```rust,ignore
let registration = protocols.register(my_protocol_spec()?, move |mut stream| {
    let provider = provider.clone();
    async move {
        let requester = stream.remote_peer().clone();
        // Deadline + bounded wire server + provider + close.
    }
})?;
```

Keep outbound operations on `MyClient`. An Endpoint may expose `client()` as a
convenience, but it should not duplicate every Client method.

## Make limits real

`AukiProtocolSpec` records the handler concurrency and frame requirement. The
wire codec must independently enforce its own frame bound; the transport cannot
infer application framing.

Define fixed deadlines for:

- opening a stream;
- each request/response or streaming phase;
- provider work where it is asynchronous; and
- stream and endpoint cleanup.

Bound queues and multi-round conversations. Treat cancellation as a normal
lifecycle event and never return partial data that failed final validation.

## Mount explicitly

```rust,ignore
let endpoint = MyEndpoint::mount(peer.protocols(), provider)?;
let client = MyClient::new(peer.protocols());
```

A client-only application constructs only the Client. A server mounts only the
versions it intends to serve.

Close in ownership order and attempt all barriers:

```rust,ignore
let operation = run_product(&client).await;
let endpoint_cleanup = endpoint.close().await;
let peer_cleanup = peer.shutdown().await;

operation?;
endpoint_cleanup?;
peer_cleanup?;
```

## Share peer information explicitly

The SDK does not yet discover peers or publish their routes. Applications
currently exchange Domain ID, expected Peer ID, protocol IDs, and complete TCP
or WSS routes through configuration, a product control plane, or a manual peer
card.

Do not bake discovery into the protocol crate. Do not treat a received route as
authority.

## Where the crate belongs

SDK-wide protocol families live in `crates/auki-protocols` behind separate wire
and endpoint features. Product-specific protocols should usually live with the
product and pin the SDK revision they use.

When adding an SDK-owned version:

- add a wire feature with no runtime dependency;
- add a separate `*-endpoint` feature that includes `auki-sdk`;
- retain an older codec only while a current data format or consumer still
  requires it, and never mount fallback implicitly; and
- add native and `wasm32-unknown-unknown` checks.

## Release gate

Before another application depends on the protocol:

```sh
cargo test --locked -p my-protocol
cargo clippy --locked -p my-protocol --all-targets -- -D warnings
cargo check --locked -p my-protocol --target wasm32-unknown-unknown
```

Also prove native/native traffic and, when Web is supported, browser/browser
plus both native/Web directions. Review the wire and endpoint modules as one
release.
