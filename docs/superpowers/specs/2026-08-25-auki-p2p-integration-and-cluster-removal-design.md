# Authenticated P2P Internals for `auki-domain`

**Status:** Approved migration design.

**Date:** 2026-08-26.

**Authoritative decisions:**
[`PLAN_AUKI_P2P_INTEGRATION.md`](./PLAN_AUKI_P2P_INTEGRATION.md).

**Implementation ledger:**
[`TODO_AUKI_P2P_INTEGRATION.md`](./TODO_AUKI_P2P_INTEGRATION.md).

## Decision

Keep `auki-domain` as the application-facing SDK and replace its Manager,
membership, networking, and authorization internals with `auki-p2p`.

This is an internal migration of the SDK we already have:

- preserve `Domain`, `DomainBuilder`, sessions, catalogs, registries, blobs,
  message channels, typed streams, and their useful business logic;
- remove Manager, leader election, cluster admission, membership convergence,
  successor handoff, and Manager-owned relay behavior;
- mutually authenticate every application stream with a DDS P2P Domain token;
- redefine known peers as current observations, never authority;
- accept explicit routes without selecting a discovery system; and
- migrate through one canonical P2P node rather than running old and new swarms
  together.

The detailed D01–D16 contracts in the plan supersede any ambiguity in this
summary.

## Why this direction

Applications are already using useful `auki-domain` concepts. They should not
have to adopt a replacement SDK simply because the implementation below those
concepts changes.

The current problem is concentrated in two internals:

- `ClusterManager` mixes product APIs, topology, election, admission, liveness,
  time sampling, Discovery HTTP, protocol supervision, and shutdown; and
- `auki-network::NetworkRuntime` mixes swarm ownership, reconnection, protocol
  dispatch, heartbeat, and a mutable `known_peers` allow-list.

Resource codecs, registries, blob validation, bounded queues, messages, and
typed streams do not inherently need those systems. We keep that code and
replace how it obtains an authenticated stream.

## The simple product model

```text
Application
    |
    v
+-------------------------------+
| Domain                        |
| one DDS Domain UUID           |
| one stable P2P identity       |
| one internal auki-p2p Node    |
| all retained Domain protocols |
+---------------+---------------+
                |
                v
        mutually authenticated
          application streams
```

One joined `Domain` owns one internal `auki_p2p::Node`. `Domain::join()`
starts it and installs the Domain protocol handlers. `Domain::leave()` stops
those handlers and shuts down the node.

There is no public process runtime, global singleton, shared multi-Domain
registry, or runtime reference counting. Concurrent multi-Domain hosting is
deferred until a real product needs it.

## Identity and Domain authority

### Stable identity

`auki_p2p::Identity` is the only P2P identity type. It owns one Ed25519 libp2p
keypair and Peer ID.

- Existing SDK wallets preserve the current
  `Wallet::derive_child("peer/v1")` derivation.
- Headless processes may load the canonical libp2p protobuf private-key bytes
  from host-owned storage.
- Generating a new identity is explicit; failed production loading never
  silently changes the Peer ID.
- Protocol code never receives raw private-key material.

### Canonical Domain

One `Domain` represents one DDS Domain UUID. Human names and wallet-derived
data identifiers remain metadata or separately typed product identifiers; they
never authorize P2P access.

### Baseline authorization

A new application stream is accepted only after `auki-p2p` verifies:

1. Noise proves the remote libp2p Peer ID.
2. DDS signed the presented ES256 `p2p-access` token.
3. Its issuer and only audience are `dds` and `auki-p2p`.
4. Its `peer_id` exactly matches the Noise Peer ID.
5. Its bounded `domain_ids` contains the exact local DDS Domain UUID.
6. Its issue/expiry profile is current.
7. The local peer also has current authority for that Domain.

That is the complete base SDK authorization rule. Roles, scopes, Manager
approval, known-peer records, application names, and route sources do not add a
second authorization layer.

Existing operation-specific integrity rules remain: expected producer/owner,
registry hashes, content hashes, frame and size limits, queue bounds, and typed
payload validation.

### Credential ownership

The host application obtains and refreshes signed DDS tokens and DDS
verification keys. It pushes them through a narrow `Domain` authority handle.

`auki-p2p`, `auki-domain`, and protocol crates perform no DDS HTTP. They
parse, verify, install, replace, and expire the supplied signed material
themselves. No new stream opens without current authority. An
already-authenticated bounded stream may finish.

## `Domain` API

Keep the current facade and move useful operations directly onto it.

`DomainConfig` contains:

- the DDS Domain UUID;
- `auki_p2p::Identity`;
- zero or more listen addresses; and
- zero or more explicit peer routes.

`DomainBuilder` retains protocol-specific composition such as providers and
message-channel declarations, and receives the initial token/key material.

`Domain::join()` validates the Peer/Session/P2P identity chain, validates local
Domain authority, starts listeners, and installs protocol handlers. It does not
wait for a remote peer. Zero listeners and zero routes are valid.

The observable Domain status is deliberately small:

- `Ready`;
- `CredentialUnavailable`;
- `Failed`; and
- `Stopped`.

Readiness describes the local service. It does not mean that another peer is
online or reachable.

Remove `Domain::cluster_manager()`, `ClusterTarget`, Manager/membership APIs,
election state, cluster CRUD, Manager relay control, and Domain-time APIs.

## Known peers and routes

### Known peers

A peer appears after its first successful mutual Domain authentication. It
remains present while at least one underlying connection is live and its last
observed credential is current. It disappears at credential expiry or final
connection closure.

The snapshot is keyed by Peer ID and contains only authenticated-until and
useful authenticated participant metadata. `peer_count()` is its current
length. `Appeared`, `Updated`, and `Disappeared` events are observational.

Every new application stream authenticates again. `known_peers` is never an
allow-list or a route database.

### Routes

`Domain` holds bounded, canonical, explicit candidates keyed by expected Peer
ID. Stage 1 supports direct TCP routes and complete CRv2 circuit routes.

Routes may come from configuration, an application, DDS/DMS adapters, or a
future discovery source. They are untrusted dial hints. A successful transport
connection still grants no application access until mutual authentication.

Discovery is outside this migration.

## Relay responsibilities

`auki-p2p` already owns relay mechanics. Keep the control-plane distinction
clear:

| Responsibility | Owner |
| --- | --- |
| Select and authorize a relay provider, limits, and deadline | External product adapter. Posemesh currently does this through the compute node and DMS. |
| Establish, confirm, renew, and cancel the exact CRv2 reservation | `auki-p2p::Node`. |
| Publish/remove the confirmed circuit route | Host/Domain integration. |
| Connect over the circuit and authenticate the application stream | `auki-p2p`. |
| Run relay capacity/admission infrastructure | Standalone relay service. |

`auki-domain` performs no DMS booking HTTP. A later general-SDK relay facade
accepts an externally authorized `RelayProvider` and delegates its reservation
to the Domain-owned node.

Relay booking and browser reachability do not block the first direct-TCP slice.

## Adding authenticated protocols

There are two supported layers:

- standalone protocol crates use the low-level `auki-p2p::Node`, as
  `auki-p2p-dataset` does; and
- protocol crates hosted by a `Domain` use its restricted `DomainProtocols`
  handle.

`DomainProtocols` can register a unique versioned handler and open an
authenticated stream to an expected peer or exact validated route. Its Domain
requirement is fixed. It cannot access credentials, private keys, the raw
swarm, listener control, allow-lists, or node shutdown.

Registration is cancellation-safe and ends automatically with the Domain.
Normal application code receives narrow protocol-specific APIs rather than
this authoring handle.

No plugin framework or one-crate-per-protocol rule is introduced.

## Protocol disposition and wire break

Keep the current resource `0.2.0`/`0.3.0`/`0.4.0`, registry
`0.2.0`/`0.3.0`, blob `0.1.0`, message `0.1.0`, and native stream
`0.2.0` payload/business logic. Adapt participant info by removing Manager
fields.

Remove join, membership, heartbeat, diagnostic broadcast, browser Manager
session, browser probe product protocols, and browser stream `0.1.0`. The later
browser SDK uses the stream `0.2.0` contract.

Retained protocols negotiate under `/auki/auth/1/...`, as frozen in D11. There
is no unauthenticated fallback or dual-stack runtime. Unchanged application
payload versions remain unchanged behind their authenticated IDs.

## Domain time

Remove Manager heartbeat and synchronized Domain-time APIs. Keep local,
monotonic, session, and product-data clocks.

If a product later needs synchronized Domain time, it gets an independently
designed authenticated protocol. This migration does not invent a leader,
consensus system, or symmetric clock algorithm.

## Canonical crates

`auki-sdk` becomes the canonical source for publishable `auki-p2p` and
`auki-p2p-dataset` crates. Posemesh consumes an exact revision and then a
pinned release; its source copies are removed.

The dependency direction is:

```text
auki-p2p-dataset -> auki-p2p
auki-domain      -> auki-p2p + retained protocol codecs/business crates
bindings         -> auki-domain
Posemesh         -> versioned auki-p2p crates
```

`auki-p2p` depends on neither `auki-domain` nor `auki-network`.

During migration, `auki-network` may remain as a codec/plain-wire-type crate.
Its swarm, central runtime, allow-list, Discovery, Manager control, join,
membership, heartbeat, and diagnostic ownership are removed. We do not move
every codec merely to improve an architecture diagram.

## Platform and release sequence

1. Native Rust and its Python binding ship first on upstream Rust libp2p `0.56`
   using TCP/DNS, Noise, and Yamux. Explicit CRv2 routes remain supported by
   `auki-p2p` but are not needed by the first slice.
2. Swift/iOS later binds the same native Domain owner.
3. Browser later keeps the `auki-domain-browser` facade with one
   TypeScript/js-libp2p engine and the same auth/protocol vectors. The Rust/WASM
   Domain experiment is retired.

The native/Python line is one documented breaking pre-`1.0` release. Manager
APIs disappear rather than becoming fake compatibility methods. Swift and
browser stay on their prior package lines until their authenticated stages are
ready.

## First vertical slice

The first slice is intentionally small:

1. Start two native Rust `Domain` instances with stable identities, signed
   same-Domain tokens, and explicit direct TCP routes.
2. Serve and fetch the existing resource catalog `0.2.0` through
   `/auki/auth/1/resources/0.2.0`.
3. Observe the authenticated peers.
4. Prove wrong Peer ID, wrong Domain, expired token, and legacy protocol ID
   expose no catalog bytes.
5. Leave cleanly with no runtime task leak.

The refactored `examples/diagnostic-app` is the manual proof application; it no
longer uses diagnostic broadcast.

Canonical crates and private adapters may land incrementally. No executable
mode starts old and new swarms together, and the public Domain cutover does not
ship until its retained protocols use the canonical node.

## Definition of done

The migration is complete when:

- retained Domain capabilities use mutually authenticated streams on one
  Domain-owned `auki-p2p` node;
- useful public Domain APIs and existing protocol business logic remain;
- known peers are only current authenticated observations;
- Manager, membership, election, heartbeat/Domain-time, allow-list, and
  Manager-specific Discovery/control code are gone;
- no legacy unauthenticated protocol fallback exists;
- `auki-sdk` is the sole canonical `auki-p2p` source;
- Stage 1 Rust/Python consumers and examples are migrated; and
- no discovery or general-SDK relay-booking system was required for the direct
  first slice.
