# auki-p2p

`auki-p2p` is the shared authenticated P2P runtime. It owns:

- the persistent libp2p identity and peer ID;
- DDS-token verification and the process-local credential authority;
- mutually authenticated application streams;
- exact direct/circuit route opening and cancellation-safe circuit cleanup;
- relay reservation transport primitives; and
- the process-shared, authority-fenced `RouteCatalog`.

It deliberately does not own DDS/DMS HTTP clients, task scheduling, or an
application protocol's wire format.

## Stable identity

`auki_p2p::Identity` is the canonical owner of the Ed25519 private key used by
libp2p Noise and DDS proofs. Native hosts should use the race-safe canonical
store directly:

```rust
let identity = auki_p2p::Identity::load_or_create("./state/peer.identity")?;
```

It creates only when absent and rejects corrupt, noncanonical, unsafe, or
wrong-algorithm existing material without replacing it. A wallet-backed host
may instead use one deliberate derivation recipe:

```rust
let peer_seed: [u8; 32] = wallet
    .derive_child("peer/v1")
    .seed()
    .try_into()
    .expect("Wallet seeds are always 32 bytes");
let identity = auki_p2p::Identity::from_ed25519_seed(&peer_seed);
```

Do not call `Identity::generate()` as a recovery path for missing or invalid
persistent identity: that changes the Peer ID and invalidates peer-bound
credentials.

## Adding an application protocol

The SDK's shared authenticated wire contracts live in the sibling
[`auki-protocols`](../auki-protocols) crate. Product-specific protocols may use
their own sibling crate, such as `auki-p2p-example`, and keep all protocol
messages and policy there. The protocol crate should:

1. Define one versioned `ApplicationProtocol` and its inbound
   `SessionRequirements`.
2. Start its inbound endpoint with `Node::serve(ProtocolSpec, ...)`.
3. Open outbound connections with `Node::open_exact_route(...)`; never expose a
   raw `Node` or token to business logic.
4. Consume a shared `RouteCatalog` when it needs advertised direct or relay
   routes.
5. Export a narrow, cloneable service facade for its callers.

The host application remains the composition root: it acquires and refreshes
credentials (native User/App hosts may opt into sibling [`auki-auth`](../auki-auth)),
owns shutdown, constructs one `Node`/`DomainAuthority`/`RouteCatalog`, and gives
those shared capabilities to each protocol crate.

The SDK's `ApplicationProtocol` vectors and authenticated Domain protocol tests
prove this generic boundary without making a product protocol part of the
transport crate. Posemesh's separately owned `auki-p2p-dataset` crate is one
external consumer of the low-level API and pins the SDK transport revision or
release it uses.
