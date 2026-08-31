# Author a portable Auki protocol

Write an application protocol once in Rust, mount it on native and Web peers,
and keep each application host small.

The reference implementation is
[`examples/portable-echo`](../../examples/portable-echo/README.md). Copy its
split, not its product name:

```text
protocol/       wire ID, types, bounded codec, client/server conversation
adapter/        AukiPeer mount, exact-route client, deadlines, cleanup, events
native/         small application host
web/            thin binding plus small application host
```

The protocol and adapter are product-owned code written once. Applications
consume their small endpoint API. `auki-sdk` remains the generic runtime below
them.

## Ownership at a glance

| Owner | Writes and decides |
| --- | --- |
| Protocol author | Exact ID, wire types, framing, bounds, conversation, behavior, contract tests |
| Shared adapter author | Registration spec, mount/send API, deadlines, stream cleanup, observable results |
| Application developer | Credentials, Domain, native identity path, protocol opt-ins, peer-information source, product policy, UI |
| SDK runtime | Authority renewal, relay booking, confirmed routes, mutual authentication, exact-route validation, fencing, shutdown |

Application authorization remains product policy. An authenticated peer in the
same Domain is not automatically allowed to drive a robot or invoke every
capability.

## 1. Create the portable contract

The protocol crate should depend on portable Rust libraries only. Its public
shape normally includes:

```rust
pub const ID: &str = "/my-product/robot-state/1.0.0";
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

Keep authentication, Tokio, libp2p, `auki-sdk`, wasm-bindgen, browser APIs,
credentials, UI, and persistence out of this crate. The same functions then run
over any authenticated Auki stream on native or Wasm.

The echo contract is a complete example:

- [`protocol/src/lib.rs`](../../examples/portable-echo/protocol/src/lib.rs)
- [`protocol/tests/locked_wire.rs`](../../examples/portable-echo/protocol/tests/locked_wire.rs)

## 2. Assign and version the ID

Product IDs use `/<namespace>[/<name>...]/<version>`. Use lower-case ASCII name
components and a numeric version such as:

```text
/my-product/robot-state/1.0.0
```

The top-level `/auki/` namespace is reserved for SDK-owned protocols. Product
protocols do not need an `/auki-p2p/` prefix.

Treat the complete ID as immutable wire identity:

- Never assign two codecs or conversations to the same ID.
- Change the ID when framing, schema, bounds, ordering, or observable semantics
  become incompatible.
- Mount multiple exact IDs explicitly when a transition genuinely requires
  serving multiple versions.

The Cargo or package version is distribution metadata, not the wire ID. It may
advance for an implementation fix that preserves the locked contract.

## 3. Lock the contract in tests

Before mounting the protocol, test it without a network:

1. Lock the exact ID and representative encoded bytes.
2. Prove the client and server conversation.
3. Reject empty, malformed, and oversized input before unbounded allocation.
4. Prove a mismatched or invalid response fails closed.
5. Compile the portable crate for native and `wasm32-unknown-unknown`.

For echo, run:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo clippy --locked -p auki-portable-echo-protocol --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo-protocol \
  --target wasm32-unknown-unknown
```

## 4. Write the shared Auki adapter once

The adapter turns the portable contract into a small endpoint. Its registration
must declare the same bound that the codec enforces:

```rust
pub fn protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, MAX_CONCURRENCY, MAX_FRAME_BYTES as u32)
}
```

`max_frame_bytes` is a declared host-side requirement; the SDK cannot infer or
enforce an application's framing. The portable codec must reject oversized
frames itself.

The reusable endpoint should own:

- the `AukiProtocolRegistration` returned by `peer.protocols().register(...)`;
- calls to the portable `run_server` and `run_client` functions;
- bounded open, exchange, and close deadlines;
- stream cleanup on success, failure, and cancellation; and
- the small result or event API applications actually need.

Copy the production-shaped
[`EchoEndpoint`](../../examples/portable-echo/adapter/src/lib.rs) rather than
teaching every application to repeat that machinery. It compiles unchanged for
native and Wasm.

## 5. Mount and dial from an application

Mounting is an explicit opt-in:

```rust
let endpoint = MyProtocolEndpoint::mount(peer.protocols())?;
```

Keep the endpoint alive while the peer serves the protocol. A client-only peer
does not need to mount the inbound handler.

Use the remote peer's complete advertised route for the portable dialing path:

```rust
let response = endpoint
    .send_exact(expected_peer_id, advertised_route, request)
    .await?;
```

Native peers normally consume a TCP route. Browser peers consume a WSS route.
The route is an untrusted location hint: the SDK still requires mutual
authentication of the expected Peer ID in the exact selected Domain.

Native adapters may additionally expose configured-route dialing when a product
already maintains a route catalog. Exact-route dialing is the cross-platform
reference because it does not guess that two peers use the same relay.

## 6. Advertise support explicitly

`0.1` has no automatic peer discovery or route publication. An application may
share a small record through configuration, a product control plane, or manual
copy and paste:

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
confirmed routes from `peer.protocol_context().routes()`. The Web facade exposes
its confirmed `wssRoute` and optional `tcpRoute`.

This JSON is illustrative application data, not a stable SDK `PeerCard` type,
a discovery record, or authority. Receiving it never grants permission.

## 7. Close in ownership order

The protocol endpoint is owned above the peer, so close it first:

```rust
endpoint.close().await?;
peer.shutdown().await?;
```

Real applications should attempt both cleanup barriers even if their operation
or endpoint close fails. The
[native reference](../../examples/portable-echo/native/src/main.rs) shows the
small finally-style pattern; the SDK provides bounded fencing underneath it.

## 8. Release the protocol

Before another product depends on the protocol:

- freeze the ID, wire description, and golden vectors;
- keep the portable contract and shared adapter in the same reviewed release;
- run native tests and the Wasm compile gate;
- prove at least one native/native exchange; and
- when Web is supported, prove browser/browser and both native/Web directions.

The portable echo crates use `publish = false` because they are repository
examples. A product may release its contract and adapter from its own repository
or consume an exact Git revision. Do not confuse publishing code artifacts with
future automatic publication of peer routes.
