# auki-p2p

Low-level identity, mutual authentication, and libp2p transport used by
`auki_sdk::AukiPeer`.

Most applications should use `AukiPeer` and portable protocol Clients /
Endpoints instead of assembling this crate directly.

## Responsibilities

`auki-p2p` owns:

- the Ed25519 identity and libp2p Peer ID;
- DDS token verification and renewable local authority installation;
- bounded mutual authentication for one exact Domain;
- authenticated application streams;
- exact direct and relay-circuit route opening;
- relay reservation transport primitives; and
- native route and authenticated-peer observations.

It does not authenticate User/App credentials over HTTP, call DDS or DMS,
allocate relay slots at the product layer, discover peers, schedule tasks, or
define product wire formats. Those responsibilities belong to `auki-auth`,
`auki-sdk`, discovery/control-plane code, and protocol crates respectively.

## Stable identity

Native hosts should use the canonical race-safe store:

```rust
let identity = auki_p2p::Identity::load_or_create("./state/peer.identity")?;
```

Existing corrupt, noncanonical, unsafe, or wrong-algorithm material fails
closed and is never replaced silently. Generating a new identity changes the
Peer ID and invalidates peer-bound credentials.

A product deliberately deriving identity from a Wallet may pass a stable
32-byte child seed to `Identity::from_ed25519_seed`.

Only one live runtime should own a native Peer ID. Replicas need separate
identity files and credentials.

## Authentication boundary

Before application bytes flow, both sides prove:

- the expected libp2p Peer ID;
- signed authority for the same exact Domain;
- acceptable credential lifetime and verification-key state; and
- protocol-specific session requirements.

Handlers receive `AuthenticatedPeer`, not an unauthenticated transport stream.
Routes and connection observations never grant application permission.

## Native and browser transport

Native builds expose TCP/DNS transport, route catalogs, direct routes, relay
circuits, and `Node`.

Wasm builds expose the browser WSS/Noise/Yamux/Relay v2 runtime used by the Web
`AukiPeer`. Browser APIs are executor-local and use exact WSS circuit routes.
The shared identity, token, authentication, and application-protocol contracts
compile on both targets.

## Application protocols

The low-level extension boundary is `ApplicationProtocol` /
`ApplicationProtocolSpec`. A protocol declares an exact ID, handler concurrency
bound, and frame requirement. Native `Node` or browser `BrowserNode` opens and
serves authenticated streams.

Application code should normally use the safer wrapper in `auki-sdk`:

```rust,ignore
let protocols = peer.protocols();
let registration = protocols.register(spec, handler)?;
let stream = protocols.open_exact(expected_peer, route, protocol_id).await?;
```

SDK-owned portable implementations live in
[`auki-protocols`](../auki-protocols). Product-owned protocols may live in
their own repositories and pin the SDK revision they use.

See the [P2P guide](../../docs/p2p/README.md) before depending on this crate
directly.
