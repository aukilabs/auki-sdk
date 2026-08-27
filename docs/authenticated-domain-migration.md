# Migrating to the authenticated Domain runtime

Stage 1 replaces the native Manager/cluster runtime with one explicit,
authenticated `Domain` owner shared by Rust and Python. This is a breaking
change: there is no compatibility mode and no unauthenticated protocol
fallback.

Upgrade a communicating native Rust/Python group together. Stage 1 peers do not
retry legacy protocol IDs. The old Manager-era Swift network and browser
packages are deleted from HEAD and remain available only at source tag
`v0.0.60`; they cannot join this authenticated runtime. A rollback means
restoring that prior SDK line for the whole communicating group, not enabling a
per-peer compatibility switch.

## Release and toolchain matrix

Stage 1 is the coordinated `0.1.0` native/Python line and has a Rust MSRV of
`1.89.0`. Every active Cargo workspace package inherits that exact
`rust-version`; release gates run on `+1.89.0`, not only on a newer compiler.

| Surface | Coordinated version |
|---|---|
| SDK source | Git tag `v0.1.0` once the release gate is complete |
| canonical transport crate | `auki-p2p==0.1.0` |
| native User/App authority client | `auki-auth==0.1.0` in the coordinated source line |
| application wire-contract crate | `auki-protocols==0.1.0` in the coordinated source line |
| Rust Domain | `auki-domain==0.1.0`, consumed from the coordinated Git tag for Stage 1 |
| Python | `auki-domain-py==0.1.0` plus exact `auki-session-py==0.1.0` |
| unsupported Swift/browser lines | source tag `v0.0.60` until their later stages |

The Rust Domain package still composes in-repository path crates, so the Stage
1 distribution boundary is the coordinated Git tag rather than an independent
`auki-domain` crates.io publish. The canonical `auki-p2p` transport crate is
publishable from this repository. Posemesh owns its separate dataset protocol
and consumes an exact SDK transport revision or release. The Python wheel pair
must be built atomically from the same tag, lockfile, compiler, target,
features, and allocator. This document describes the pending release line; it
does not imply that the tag or registry artifacts have already been published.

## The new ownership model

- `auki-session::Peer` owns stable application identity, peer registries, and
  storage.
- `auki-session::Session` owns one recording timeline, its clocks, and logs.
- `auki-domain::Domain` owns one native P2P node for one exact DDS Domain UUID.
- `auki-auth` optionally turns native User/App credentials plus a stable
  identity into validated authority for one selected Domain.
- `auki-protocols` owns exact application protocol IDs, wire types, bounded
  framing, validation, and locked vectors; it owns no transport or handlers.
- The host acquires DDS verification keys and a signed P2P credential, persists
  the P2P identity, selects listeners, and supplies explicit peer routes.
- `auki-p2p` verifies the signed authority against the Noise Peer ID before any
  application bytes and operates direct or Circuit Relay v2 transport.

Each Domain serves no built-in application protocols by default. The host must
select every exact inbound version with `ServedProtocols`; client operations
remain available regardless of that selection. Compiling an `auki-protocols`
feature means the wire contract is available, not that a handler is installed.

The configured key owner is always `auki_p2p::Identity`. Native hosts should
use `Identity::load_or_create(path)`, which creates only when absent and rejects
corrupt, unsafe, or noncanonical existing files without replacement. A
wallet-backed host may instead pass the 32-byte seed from
`Wallet::derive_child("peer/v1")` to `Identity::from_ed25519_seed`. Never mint a
replacement as fallback for invalid persistent state: it will have a different
Peer ID and will not match the signed credential.

Core `auki-domain` and `auki-p2p` do not call DDS or DMS HTTP. Native User/App
hosts may opt into `auki-auth` for the bounded API/DDS authority exchange;
Robot/Compute hosts use product-owned machine adapters. Route discovery and
authorization remain separate: a configured or discovered address is only a
dial hint; a valid same-Domain credential bound to the Noise Peer ID is
authority.

## Rust breaking changes

| Before | Stage 1 replacement |
|---|---|
| `ClusterManager`, `NetworkRuntime` | `auki_domain::Domain` |
| `auki-network` runtime, identity adapters, and codecs | `auki-p2p::Identity` and transport; `auki-protocols` wire contracts; `auki-domain` hosting policy |
| cluster create/join/bootstrap | `Domain::builder(&peer, &session, config).authority(keys, credential).join().await` |
| `ClusterTarget` or Discovery URL | `DomainConfig::with_peer_routes(...)` initially; `domain.routes().replace(...)` at runtime |
| Manager/leader identity | Removed; applications target an expected authenticated `PeerId` |
| membership roster and roles | Removed; `domain.known_peers()` is a live authenticated observation, never an authorization roster |
| heartbeat liveness | authenticated transport observations and protocol results |
| synchronized Domain time | explicit clock metadata and recorded TimeTransform Logs |
| Manager-owned catalog fetch | `domain.fetch_resources_catalog(expected_peer).await` |
| Manager-owned registry/blob access | `Domain` registry and blob operations against an expected peer |
| Manager stream/message methods | `Domain` typed stream and receiver-owned message APIs |
| implicit shutdown or detached tasks | bounded `domain.leave().await` |

Minimal construction shape:

```rust
use auki_domain::{Domain, DomainConfig, ServedProtocols};

let config = DomainConfig::new(dds_domain_id, identity)
    .with_listen_addresses(listen_addresses)?
    .with_peer_routes(remote_peer_id, remote_routes)?;

let domain = Domain::builder(&peer, &session, config)
    .authority(dds_verification_keys, signed_p2p_credential)
    .served_protocols(ServedProtocols::none().with_resources_v2())
    .join()
    .await?;

let rows = domain.fetch_resources_catalog(remote_peer_id).await?;
let peers = domain.known_peers().snapshot();
domain.leave().await?;
```

The configured identity, `Peer`, `Session`, and signed credential must resolve
to the same canonical Peer ID. A mismatch fails before the node becomes ready.
Omit `served_protocols(...)` only for a client-only Domain that intentionally
accepts no built-in inbound application protocol.

## Python breaking changes

| Before | Stage 1 replacement |
|---|---|
| `auki_network` / `auki-network-py` | removed; use `auki_domain` / `auki-domain-py` |
| Python `ClusterManager` and cluster targets | `DomainBuilder`, `DomainConfig`, and explicit `DomainRoutes` |
| create/join/list clusters | construct one exact Domain and `await builder.join()` |
| membership, leader, Manager, heartbeat, Domain-time properties | removed |
| implicit runtime shutdown | `await domain.leave()`; cancellation and object cleanup retain an owned native cleanup path |
| networking-owned `Peer`/`Session` copies | live `auki_session.Peer` and `Session` objects bridged into `DomainBuilder` |

The Python surface exposes authority rotation, routes, status, known peers,
catalog/registry/blob operations, messages, and typed streams over the same Rust
owner. See the [`auki-domain-py` guide](../bindings/python/auki-domain-py/README.md)
for a complete join example.

Python uses the same default-none rule. Call only the exact methods the
application hosts, such as `builder.serve_resources_v2()` or
`builder.serve_streams_v2()`, before `await builder.join()`.

`auki-domain-py==0.1.0` requires the exactly paired
`auki-session-py==0.1.0` wheel from the same SDK build. Their private native
Peer/Session capsule carries a Rust ABI value; build and publish the pair from
the same commit, lockfile, Rust toolchain, target, and feature set. The binding
README documents that release constraint in detail.

## Where routes come from

Stage 1 deliberately does not prescribe topology policy. A host may populate
the exact-peer route catalog from:

- static configuration;
- application state;
- a DDS/DMS adapter;
- a local discovery adapter; or
- an externally distributed confirmed relay route.

Changing the source of a route never changes authentication. The remote must
still present a valid signed credential for the selected Domain and its token
Peer ID must equal the Noise Peer ID and the expected dial target.

The accepted Stage 1 candidates are a direct TCP multiaddr (an optional terminal
`/p2p/<expected-peer>` must match) or a complete circuit multiaddr such as
`/dns4/<host>/tcp/<port>/p2p/<relay>/p2p-circuit/p2p/<expected-peer>`. Each
candidate is stored under that expected Peer ID. `known_peers()` is a
post-authentication observation, not discovery and not a source of dial
candidates.

For the concrete Posemesh relay flow, compute-node/DMS owns provider-selection
and booking HTTP and constructs an authorized `RelayProvider`. The lower-level
canonical `auki_p2p::Node` starts, waits for, renews, and cancels the Circuit
Relay v2 reservation. Only the confirmed `snapshot.publishable_route()` is
distributed; a receiving Domain installs that complete circuit candidate under
the reserving peer's expected Peer ID. The public Stage 1 `Domain` facade does
not expose a general booking/reservation service, and the canonical local proof
uses direct TCP.

## Manual proof

[`examples/diagnostic-app`](../examples/diagnostic-app) is a scriptable single-
Domain peer. Its README shows how to run two local instances with supplied
identity, authority, listeners, and reciprocal routes, then observe lifecycle
status, authenticated known peers, and a Resource Catalog v0.2 fetch.

The documented example is part of the workspace. Run the canonical proof with:

```sh
cargo build --locked -p auki-diagnostic-app
./examples/diagnostic-app/scripts/local-demo.sh
```

Success includes `READY` for both processes, bidirectional
`CATALOG ... count=1`, `PEERS count=1`, ordered `LEFT`, and a final `DEMO_OK`.
The script also asserts that wrong-Domain, wrong-Peer, and malformed JWTs each
produce `JOIN_FAILED` and no `CATALOG` output.

## Migration checklist

1. Persist one stable P2P identity and obtain a credential for its exact Peer
   ID and DDS Domain UUID.
2. Move registry ownership to `Peer` and recording/log ownership to `Session`.
3. Replace cluster bootstrap with `DomainBuilder` and host-supplied authority.
4. Select each exact inbound protocol version the application actually serves;
   assume none are installed by default.
5. Replace discovery or target objects with explicit exact-peer route updates.
6. Replace membership/Manager policy with application policy over expected peer
   IDs; do not treat `known_peers()` as authorization.
7. Replace heartbeat-derived time with explicit clock lineage and recorded
   transforms.
8. Await `Domain::leave()` (or Python `Domain.leave()`) on normal shutdown.
9. Test wrong-peer, wrong-Domain, expired, and missing credentials: none may
   expose application data.
