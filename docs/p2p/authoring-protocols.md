# Author one portable Auki protocol crate

Put one versioned wire contract and its small `AukiPeer` endpoint in one Rust
crate. Native and Web hosts then depend on that crate and stay small.

The reference is
[`examples/portable-echo`](../../examples/portable-echo/README.md):

```text
my-protocol/
├── Cargo.toml
├── src/
│   ├── lib.rs       private modules and the small public API
│   ├── wire.rs      ID, types, bounded codec, conversation
│   └── endpoint.rs  mount, dial, deadlines, cleanup, events
└── tests/
    └── locked_wire.rs
```

`wire.rs` stays transport-neutral. `endpoint.rs` connects it to the canonical
cross-target `AukiPeer` surface. These are private implementation modules in
one product-owned package and release, not separate crates applications must
assemble.

That crate can live in the product repository or in this workspace's example
or protocol area. Product-specific endpoints do not belong in the generic
`crates/auki-protocols` package, which retains SDK-owned wire contracts.

## Ownership at a glance

| Owner | Writes and decides |
| --- | --- |
| Protocol author | Exact ID, wire types, framing, bounds, conversation, endpoint API, deadlines, cleanup, events, contract tests |
| Application developer | Credentials, Domain, native identity path, protocol opt-in, peer-information source, product policy, UI |
| SDK runtime | Authority renewal, relay booking, confirmed routes, mutual authentication, exact-route validation, fencing, shutdown |

Authentication is not application authorization. A peer authenticated in the
same Domain is not automatically allowed to drive a robot or invoke every
capability; the product still enforces that policy.

## 1. Create one crate with two private modules

Keep `lib.rs` boring:

```rust
mod endpoint;
mod wire;

pub use endpoint::{MyEndpoint, MyError, protocol_spec};
pub use wire::{PROTOCOL_ID, Request, Response};
```

The crate as a whole depends on `auki-sdk` because its endpoint mounts on
`AukiPeer`. The wire module itself should not depend on authentication, Tokio,
libp2p, `auki-sdk`, wasm-bindgen, browser APIs, credentials, UI, or persistence.
Its client and server functions accept a portable asynchronous duplex stream,
so the same conversation runs on native and Wasm.

Compare the reference modules:

- [`src/wire.rs`](../../examples/portable-echo/src/wire.rs)
- [`src/endpoint.rs`](../../examples/portable-echo/src/endpoint.rs)
- [`src/lib.rs`](../../examples/portable-echo/src/lib.rs)

## 2. Assign one immutable protocol ID

Product IDs use `/<namespace>[/<name>...]/<version>`, with lower-case ASCII
name components and a numeric version, for example:

```text
/my-product/robot-state/1.0.0
```

The top-level `/auki/` namespace is reserved for SDK-owned protocols. Product
protocols do not need an `/auki-p2p/` prefix.

Treat the complete ID as immutable wire identity:

- Never assign two codecs or conversations to the same ID.
- Change the ID when framing, schema, bounds, ordering, or observable semantics
  become incompatible.
- Mount multiple exact IDs explicitly only when a transition genuinely needs
  multiple versions.

The Cargo version is distribution metadata. It may advance for a compatible
implementation fix without changing the wire ID.

## 3. Implement and lock the wire contract

A portable wire module normally exposes bounded client and server
conversations:

```rust
pub const PROTOCOL_ID: &str = "/my-product/robot-state/1.0.0";
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

pub async fn run_client<S>(stream: &mut S, request: Request) -> Result<Response, Error>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    // Encode one bounded request, read one bounded response, validate it.
}

pub async fn run_server<S>(stream: &mut S) -> Result<AcceptedRequest, Error>
where
    S: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    // Read one bounded request and perform the exact server conversation.
}
```

Lock the exact ID and representative encoded bytes. Prove the conversation and
reject empty, malformed, mismatched, and oversized input before unbounded
allocation. The echo vectors live in
[`tests/locked_wire.rs`](../../examples/portable-echo/tests/locked_wire.rs).

## 4. Add the endpoint in the same crate

The endpoint declares the same bound enforced by the wire codec:

```rust
pub fn protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    let max_frame_bytes = u32::try_from(MAX_FRAME_BYTES)
        .expect("protocol frame bound must fit in u32");
    AukiProtocolSpec::new(PROTOCOL_ID, MAX_CONCURRENCY, max_frame_bytes)
}
```

`AukiProtocolSpec` accepts a frame bound from 1 byte through 64 MiB, the
underlying authenticated transport limit. The SDK records that host requirement
but cannot infer or enforce an application's framing. The wire codec must
independently reject frames above the same or a smaller bound.

The endpoint owns:

- the registration returned by `peer.protocols().register(...)`;
- calls to the shared `run_server` and `run_client` conversation;
- bounded open, exchange, and close deadlines;
- stream cleanup on success, failure, and cancellation; and
- the small result or event API applications need.

Copy the production-shaped
[`EchoEndpoint`](../../examples/portable-echo/src/endpoint.rs), not its product
name. It compiles unchanged for native and Wasm.

## 5. Mount and dial from an application

Mounting is explicit opt-in:

```rust
let endpoint = MyEndpoint::mount(peer.protocols())?;
```

Keep the endpoint alive while serving. A client-only peer does not need to
mount an inbound handler if the protocol crate exposes a separate client.

Use the remote peer's complete advertised route for portable dialing:

```rust
let response = endpoint
    .send_exact(expected_peer_id, advertised_route, request)
    .await?;
```

Native peers normally consume a TCP route. Browser peers consume a WSS route.
The route is an untrusted location hint: the SDK still mutually authenticates
the expected Peer ID in the exact selected Domain.

A native endpoint may also expose configured-route dialing when a product
maintains a route catalog. Exact-route dialing remains the cross-platform
reference because it does not assume two peers use the same relay.

## 6. Share support and routes explicitly

`0.1` has no automatic peer discovery or route publication. An application
may exchange a small record through configuration, a product control plane, or
manual copy and paste:

```json
{
  "domainId": "<selected Domain UUID>",
  "peerId": "<expected Peer ID>",
  "protocols": ["/my-product/robot-state/1.0.0"],
  "routes": {
    "tcp": "<confirmed native route>",
    "wss": "<confirmed browser route>"
  }
}
```

Include only routes the peer actually received. Native applications read
confirmed routes from `peer.protocol_context().routes()`. The Web facade
exposes `wssRoute` and an optional `tcpRoute`.

This JSON is illustrative product data, not a stable SDK `PeerCard`, a
discovery record, or authority. Receiving it never grants permission.

## 7. Close in ownership order

The endpoint is owned above the peer, so close it first. Capture results so
each cleanup barrier is attempted even if an earlier operation fails:

```rust
let operation = run_with(&endpoint).await;
let endpoint_cleanup = endpoint.close().await;
let peer_cleanup = peer.shutdown().await;

operation?;
endpoint_cleanup?;
peer_cleanup?;
```

The [native reference](../../examples/portable-echo/native/src/main.rs) keeps
the endpoint and peer cleanup at their respective ownership levels while using
this same result-capture pattern.

## 8. Test and release one package

Before another product depends on the protocol:

- freeze the ID, wire description, bounds, and golden vectors;
- review the wire and endpoint modules in the same release;
- prove at least one native/native exchange; and
- when Web is supported, prove browser/browser and both native/Web directions.

The reference's complete package gates are:

```sh
cargo test --locked -p auki-portable-echo
cargo clippy --locked -p auki-portable-echo --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo --target wasm32-unknown-unknown
```

The reference uses `publish = false` because it is a repository example. A
product can publish its one protocol crate or consume an exact Git revision.
Publishing code is separate from future automatic publication of peer routes.
