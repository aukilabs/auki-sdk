# Auki P2P

Build authenticated peer-to-peer applications for robots, spatial tools, and
native services without assembling the networking runtime yourself.

**[Get started with Rust](getting-started.md)** ·
**[Run the local transport demo](../../examples/diagnostic-app/README.md)**

> **Current scope:** The high-level `AukiPeer` facade is available in Rust.
> User and trusted App peers get automatic authority renewal and relay-backed
> reachability by default. Peer discovery and route distribution are separate
> work; applications still provide the expected remote Peer ID and route.

## The mental model

An Auki peer is one stable cryptographic identity operating inside one
authorized Domain.

```text
User password or App credentials
              │
              ▼
          auki-auth ─────► PreparedPeer
                                │
persistent Identity ─────────────┤
app config + protocol opt-ins ───┤
                                ▼
                           AukiPeer
                                │
                 ┌──────────────┼──────────────┐
                 ▼              ▼              ▼
             authority        routes        protocols
              renewal       and relay      and app data
```

The distinction between authority and reachability is important:

- A credential proves which Peer ID may participate in which Domain.
- A route only tells the transport where to dial that peer.
- A relay makes a peer reachable; it does not discover other peers.
- Application policy still decides who may invoke a command or capability.

## The normal Rust path

Most applications perform five explicit steps:

1. Authenticate a User or trusted App with `auki-auth`.
2. Select one currently accessible Domain.
3. Load one persistent identity and prove ownership of its Peer ID.
4. Pass the resulting `PreparedPeer`, identity, and `AukiPeerConfig` to
   `AukiPeer::start`.
5. Register or open application protocols through
   `peer.protocol_context()` and finish with `peer.shutdown().await`.

`AukiPeer::start` owns the mechanical work between steps four and five:

- the SDK `Peer` and `Session`;
- the authenticated Domain runtime;
- verification keys and credential renewal;
- local route state;
- DMS relay booking and reservation recovery; and
- readiness monitoring and ordered shutdown.

Relay-backed reachability is required by default. Startup returns only after
the Domain and authority are ready and at least one confirmed relay route is
available. `AukiPeerConfig::direct_only()` is the explicit opt-out and makes no
DMS relay-booking calls.

## What your application still owns

The facade deliberately leaves product decisions visible:

| Application provides | SDK owns |
| --- | --- |
| Credentials and exact Domain selection | Authentication proof and renewable authority |
| Stable identity storage location | Domain and transport lifecycle |
| Application ID and data directory | SDK `Peer` and `Session` |
| Exact inbound protocol opt-ins | Protocol hosting and authenticated streams |
| Initial remote Peer IDs and route hints | Direct-first dialing and local relay recovery |
| Capability or command policy | Mutual Peer-ID and Domain authentication |

Discovery is not hidden inside authentication. Until a directory or
rendezvous layer is added, exchange the remote Peer ID and its complete direct
or relay route through configuration or an application control plane.

## Protocols are explicit

A new peer serves no built-in protocol by default. Select exact built-in
versions with `AukiPeerConfig::with_served_protocols(...)`, or register a
versioned product protocol through:

Product owners choose bounded, explicitly versioned IDs shaped like
`/<name>[/<name>...]/<version>`; for example, `/posemesh/store/v1`. The
top-level `/auki/` namespace is reserved; retained SDK protocols use
`/auki/auth/1/...`.
`/auki-p2p/dataset/0` remains a valid product protocol, not a required prefix.

```rust
let context = peer.protocol_context();
let registration = context.protocols().register(spec, handler)?;
```

Keep the returned registration alive for as long as the handler should remain
mounted. Client-side protocol opens do not require mounting the corresponding
inbound handler locally.

The protocol context intentionally exposes only:

- authenticated protocol registration and opening;
- a read-only view of published local routes;
- non-secret local identity and authorization metadata.

It does not expose the raw Domain, transport node, authority installer, or
relay reservations.

## Safe defaults

- Identity corruption fails closed; the SDK never silently creates a new Peer
  ID over invalid material.
- Routes and `known_peers()` never grant authority.
- Remote operations authenticate the expected Peer ID and exact Domain.
- Relay-backed startup is the default; direct-only operation is explicit.
- Credential renewal and authority expiry are supervised by the facade.
- Explicit `shutdown().await` drains reservations and requests DMS booking
  deletion before leaving the Domain.
- App secrets belong only in trusted native or headless processes—not browsers
  or distributed mobile applications.

## One identity, one live runtime

Version `0.1.0` supports one live `AukiPeer` for a given Peer ID. Reusing the
same identity after awaited shutdown is supported; running the same identity
in simultaneous processes or pods is not.

For Kubernetes, use one replica per persisted identity, such as a
single-replica StatefulSet or a Recreate rollout. Parallel replicas need
distinct identities.

## Advanced machine authority

Robot and Compute hosts may already receive authority through a product
control plane. Those integrations use `AukiPeer::start_external` and feed
complete replacements through `ExternalAuthorityControl`. The host retains
its task or heartbeat policy; the facade still owns the Domain, routes,
protocol context, fencing, and shutdown.

This is an integration boundary, not the recommended first experiment. User
and App developers should begin with `auki-auth` and `AukiPeer::start`.

## Platform status

| Platform | High-level authenticated peer facade |
| --- | --- |
| Rust | Available: User/App auth, renewal, relay, status, protocols, shutdown |
| Python | Pending |
| Swift/iOS | Pending |
| Web | Pending; browser-safe auth and transport are required |

## Start here

- **First authenticated experiment:** follow
  [Getting started with Rust](getting-started.md).
- **No service credentials yet:** run the
  [local two-peer transport demo](../../examples/diagnostic-app/README.md).
- **Migrating Manager-era code:** read the
  [authenticated Domain migration guide](../authenticated-domain-migration.md).
- **Building a custom protocol:** use the safe surface described in
  [`AukiPeerProtocolContext`](../../crates/auki-sdk/src/context.rs).

## Keep these rules

> A route tells you where to dial. A credential tells you who may enter.

> A relay makes your peer reachable. It does not tell you which peers exist.

> `known_peers()` reports authenticated connections observed now. It is not a
> Domain roster, discovery service, route source, or authorization cache.
