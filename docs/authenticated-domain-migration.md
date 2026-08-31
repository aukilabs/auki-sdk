# Migrate Manager-era networking to AukiPeer

Stage 1 removes the Manager and cluster runtime. The canonical replacement for
new native Rust and Web applications is `AukiPeer`: one authenticated Peer ID
inside one exact DDS Domain, with explicit application protocols.

This is a wire break. There is no legacy protocol fallback or compatibility
switch. Upgrade a communicating group together. The removed Manager-era Rust,
Python, Swift, and browser sources remain available at tag `v0.0.60`, but they
cannot join the authenticated `0.1` runtime.

## Choose the migration target

| Consumer | Stage 1 target |
| --- | --- |
| Native Rust User or trusted App | `auki_sdk::AukiPeer::start` — canonical |
| Web User | `AukiUserSession` plus ephemeral `AukiPeer` — canonical |
| Robot or Compute host with product-managed authority | `AukiPeer::start_external` — canonical |
| Existing Rust code using retained catalog, registry, blob, message, or stream APIs | `auki-domain` may remain as a low-level compatibility bridge |
| Existing Python networking | `auki-domain-py` is a compatibility bridge until a Python `AukiPeer` facade exists |
| Swift/iOS networking | canonical `AukiPeer` facade pending; do not revive Manager semantics |

`auki-session::Peer` and `Session` remain the network-free recording and data
model. They are not substitutes for the P2P runtime.

The coordinated source line is `0.1.0` with Rust MSRV `1.89.0`. This checkout
does not imply that the Git tag, registry crates, wheels, Web package, or Swift
facade have been published. Consume one reviewed SDK revision until those
artifacts are released.

## The canonical ownership model

An ordinary application supplies:

- User credentials or trusted native App credentials;
- one selected accessible Domain;
- a persistent native identity path;
- the exact product protocols it mounts;
- the expected remote Peer ID and complete compatible route; and
- product authorization policy.

`auki-auth` converts credentials and an identity proof into a validated
`PreparedPeer`. `AukiPeer` then owns renewable authority, authenticated
transport, DMS relay booking, confirmed routes, protocol hosting, peer
observations, fencing, and bounded shutdown.

Robot and Compute products supply externally managed authority instead. They
still use the same `AukiPeer` runtime for networking; product code retains task,
capability, safety, and heartbeat policy.

`auki-p2p` is the lower transport layer. Most applications should not assemble
its node, authority, route catalog, and relay lifecycle themselves.

## Rust migration map

| Manager-era concept | Canonical replacement |
| --- | --- |
| `ClusterManager`, `NetworkRuntime` | one `AukiPeer` |
| cluster create, join, or bootstrap | authenticate, select a Domain, authorize a stable identity, then `AukiPeer::start` |
| `ClusterTarget` or Discovery URL | expected Peer ID plus an exact TCP or WSS route |
| Manager or leader authority | product capability/JWT policy over authenticated peers |
| membership roster and roles | removed; peer observations are connectivity, never authorization |
| implicit built-in product protocols | one explicitly mounted, versioned product protocol crate |
| Manager-owned catalog, registry, blob, message, or stream methods | product endpoint on `AukiPeer`, or retained `auki-domain` temporarily while that protocol is ported |
| heartbeat-derived Domain time | explicit clock metadata and recorded TimeTransform Logs |
| implicit or detached shutdown | close mounted endpoints, then await `peer.shutdown()` |

The native shape is intentionally mechanical:

```rust
let identity = Identity::load_or_create(identity_file)?;
let session = AuthClient::new(AuthEnvironment::dev())?
    .authenticate(Credentials::user_password(email, password))
    .await?;
let prepared = session
    .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
    .await?;
let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;

let endpoint = MyEndpoint::mount(peer.protocols())?;
// Run product work against an expected Peer ID and exact route.
let endpoint_cleanup = endpoint.close().await;
let peer_cleanup = peer.shutdown().await;
endpoint_cleanup?;
peer_cleanup?;
```

Real code captures the product-operation result before cleanup so both cleanup
barriers are still attempted on failure. See
[Build with an existing protocol](p2p/getting-started.md) for the complete
failure-safe pattern.

One persisted native identity belongs to one live runtime. Sequential restart
is supported; simultaneous processes or pods using the same Peer ID are not.

## Web migration

The current Web/Wasm facade is supported source, not the deleted Manager-era
browser stack. Its model is deliberately narrow:

1. `AukiUserSession` authenticates a User and lists accessible Domains.
2. The application explicitly selects one Domain.
3. `startPeer` creates a fresh in-memory identity and waits for one relay.
4. The application mounts a Rust protocol endpoint through a thin Wasm binding.
5. Browser dialing uses the remote peer's exact WSS route.
6. The endpoint closes before `AukiPeer.shutdown()`.

Each start creates a new Peer ID. The browser does not persist peer identity or
credentials, does not accept App secrets, and has no direct-only mode in `0.1`.
Reloading therefore requires sharing the new Peer ID and route.

## Robot and Compute migration

Use `AukiPeer::start_external` when a product already obtains and refreshes
machine authority. The product provides each complete
`ExternalAuthorityUpdate` and responds to refresh requests. The SDK still owns
the authenticated transport, relay allocation and renewal, route validation,
protocol surface, fencing, and shutdown.

This split keeps product policy outside the generic SDK without duplicating
relay or route machinery in Posemesh or another host.

## Reachability and discovery

Relay-backed reachability is the native default and the Web requirement.
Startup returns only after a confirmed relay route is available.

A native host may explicitly choose `AukiPeerConfig::direct_only()`. That mode
makes no DMS booking calls and may have zero listeners and advertised routes
for outbound-only operation. A direct-only peer that must accept inbound
connections supplies a listener and a matching externally reachable advertised
route.

Neither mode performs automatic peer discovery or route publication. The
application currently receives a remote Peer ID, Domain ID, supported protocol
IDs, and complete route through configuration, manual exchange, or its product
control plane. A route is only an untrusted dial hint: the remote still has to
authenticate the expected Peer ID in the selected Domain.

`known_peers()` is a post-authentication connectivity observation. It is not a
membership roster, authorization source, discovery service, or route catalog.

## Compatibility appendix: retained Domain

`auki-domain` remains available for existing native consumers of the retained
resource catalog, registry, blob, message, and typed-stream protocols. It is a
low-level authenticated Domain owner, not the new application facade and not a
new Manager.

On this path, the host still supplies:

- one persistent `auki_p2p::Identity`;
- DDS verification keys and a signed credential for that exact Peer ID and
  Domain;
- any listeners and explicit remote routes;
- selected inbound `ServedProtocols`; and
- authority rotation and lifecycle policy.

The core `auki-domain` path does not authenticate Users or Apps, call DMS, book
a relay, renew credentials, discover peers, or publish routes. It serves no
built-in inbound protocol unless the host selects one. The identity,
`auki-session::Peer`, `Session`, and signed credential must resolve to the same
Peer ID, and shutdown ends with `domain.leave().await`.

Python currently exposes this same compatibility owner through
[`auki-domain-py`](../bindings/python/auki-domain-py/README.md). It is not yet a
Python `AukiPeer` facade. `auki-domain-py==0.1.0` must be built and distributed
with the exact matching `auki-session-py==0.1.0` wheel from the same commit,
lockfile, Rust toolchain, target, features, and allocator because their private
capsule crosses the Rust ABI boundary.

Do not run a retained `Domain` and an `AukiPeer` concurrently with the same
identity. Migrate one networking owner at a time.

## Prove the migration

The canonical proof is
[`examples/portable-echo`](../examples/portable-echo/README.md):

```sh
cargo test --locked -p auki-portable-echo
cargo run --locked -p auki-portable-echo-native
```

Its protected Web smoke proves browser-to-browser in both directions,
native-to-browser, and browser-to-native over exact relay routes.

[`examples/diagnostic-app`](../examples/diagnostic-app) remains useful for the
retained low-level Domain path. Its local proof uses manually supplied direct
TCP routes; that is a compatibility diagnostic, not the canonical `AukiPeer`
topology.

## Migration checklist

1. Choose `AukiPeer` unless a retained low-level protocol forces a temporary
   Domain compatibility bridge.
2. Persist one native identity and authorize its exact Peer ID for one Domain.
3. Replace Manager bootstrap with `AukiPeer::start` or `start_external`.
4. Put each product wire contract and endpoint in one explicitly versioned
   protocol crate, then mount only the protocols this peer serves.
5. Replace discovery targets with an expected Peer ID and exact compatible
   route until discovery and publication exist.
6. Keep capability, task, safety, and application authorization in the product;
   never infer it from routes or peer observations.
7. Replace Manager time with explicit clock lineage and recorded transforms.
8. Close protocol endpoints before shutting down the peer, and attempt both
   barriers when an operation fails.
9. Test wrong-Peer, wrong-Domain, expired, missing, and rotated authority: none
   may expose application data.
