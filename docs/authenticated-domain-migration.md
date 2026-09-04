# Migrate Manager-era networking to AukiPeer

The current SDK has one application networking model: an `AukiPeer` is one
authenticated Peer ID inside one exact DDS Domain, with explicitly mounted
protocol endpoints.

This is a wire and lifecycle break. There is no Manager, cluster bootstrap,
implicit protocol bundle, or compatibility runtime in the current workspace.
Upgrade communicating peers together.

## Choose the target

| Host | Current path |
| --- | --- |
| Native Rust User | `auki-auth` User credentials → `PreparedPeer` → `AukiPeer::start` |
| Trusted native/headless App | App credentials → `PreparedPeer` → `AukiPeer::start` |
| Robot or Compute product | product-managed authority → `AukiPeer::start_external` |
| Web browser | `AukiUserSession` → selected Domain → ephemeral relay-backed peer |
| Python | `AukiSession` → selected Domain → persistent relay-backed peer |
| Swift/iOS | `AukiSession` → selected Domain → ephemeral or app-persisted relay-backed peer |

`auki-session::Peer` and `Session` remain the network-free recording model.
They do not authenticate or join a Domain.

## Replace the old ownership model

An application now supplies:

- credentials or externally managed authority;
- one selected Domain UUID;
- a persistent native identity path, or an intentional ephemeral identity;
- the exact protocols it serves;
- product authorization policy; and
- either opt-in DDS discovery or an explicitly exchanged remote Peer ID and route.

The SDK owns:

- authority renewal and expiry fencing;
- mutually authenticated transport;
- default DMS relay booking and renewal;
- optional DDS discovery publication and fresh lookup;
- route validation and exact-peer dialing;
- protocol registration and handler lifecycle; and
- bounded shutdown.

## Concept map

| Old assumption | Current replacement |
| --- | --- |
| one Manager or leader | no topology authority; DDS credentials authenticate each peer |
| cluster roster grants access | application policy over `AuthenticatedPeer` |
| bootstrap discovers members | authentication starts one peer; discovery is separate |
| route knowledge implies trust | routes are untrusted dial hints |
| built-in protocols always run | mount exact `Endpoint`s explicitly |
| one object handles inbound and outbound operations | `Endpoint` serves; `Client` calls |
| network-derived Domain time | explicit Clock Registry entries and TimeTransform Logs |
| implicit shutdown | close endpoints, then await `AukiPeer::shutdown` |

A DDS Domain remains the physical-space and authority concept. It is not a
runtime class or peer collection.

## Native Rust shape

```rust,ignore
let identity = Identity::load_or_create(identity_file)?;
let auth = AuthClient::new(environment)?
    .authenticate(Credentials::user_password(email, password))
    .await?;
let prepared = auth
    .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
    .await?;
let peer = AukiPeer::start(identity, prepared, AukiPeerConfig::dev()).await?;

let provider = MyProvider::new();
let endpoint = MyEndpoint::mount(peer.protocols(), provider)?;
let client = MyClient::new(peer.protocols());

let operation = run_product(&client).await;
let endpoint_cleanup = endpoint.close().await;
let peer_cleanup = peer.shutdown().await;

operation?;
endpoint_cleanup?;
peer_cleanup?;
```

Capture all three results so a product failure does not skip endpoint or peer
cleanup.

## Move protocol logic, not application policy

For every application protocol:

1. assign one immutable, versioned protocol ID;
2. keep bounded wire types and codec portable;
3. expose a cloneable outbound `Client`;
4. expose an inbound/lifecycle `Endpoint` using an application Provider;
5. pass `AuthenticatedPeer` to policy decisions; and
6. lock representative encoded bytes and failure cases.

SDK-owned protocol families already follow this shape in `auki-protocols`.
Catalog v2 is wire-only because v3 embeds its locked row shape; Registry begins
at v3. Mount current endpoints explicitly rather than assuming fallback.

For local recorded data, compose:

- `SessionProtocolProvider` with `CatalogEndpoint` and `StreamEndpoint`;
- `FsRegistryProvider` with `RegistryEndpoint`; and
- `FsBlobProvider` with `BlobEndpoint`.

These adapters are mechanical. Wrap or replace them when product authorization
must filter an authenticated caller.

## Robot and Compute hosts

Use `AukiPeer::start_external` when the product already obtains and refreshes
machine authority. The product sends complete authority updates and responds
to refresh requests. It keeps task, capability, heartbeat, and safety policy;
the SDK keeps relay, routes, protocol hosting, fencing, and shutdown.

Do not duplicate relay allocation or route validation in the product runtime.

## Web migration

The Web facade authenticates a User, lists accessible Domains, and starts an
ephemeral Peer ID. Relay-backed startup reserves one relay over WSS and exposes
the same provider slot's atomic TCP/WSS route pair. Outbound-only startup skips
the booking and exposes no local route while retaining authenticated outbound
dials to remote WSS relay routes.
Protocol adapters remain Rust code compiled into the same Wasm module.

Pass `AukiPeerReachabilityMode.OutboundOnly` as the optional final argument to
`startPeer` or `startPeerWithDiscovery`; omitting it remains relay-backed. Web
consumers must now handle `tcpRoute` and `wssRoute` as `string | undefined` and
can inspect `relayBacked` before publishing a peer card.

The browser does not accept App secrets or persist identity in the first
iteration. A reload therefore produces a new Peer ID and, in relay-backed mode,
a new route.

## Reachability and discovery

Relay-backed reachability remains the default. Native applications can choose
direct-only operation, and Web applications can choose outbound-only operation.
An inbound direct peer needs a listener and a dialable route that the
application shares; an outbound-only browser cannot advertise itself through
DDS because it has no public route.

The SDK can optionally publish and query short-lived DDS discovery records.
Applications may still exchange Domain ID, Peer ID, supported protocols, and
complete TCP/WSS routes through configuration, a product control plane, or a
manual peer card. See [Discover peers](p2p/discovery.md).

On native Rust, `known_peers()` only reports successful authenticated
connectivity. It is not discovery or authorization.

## Migration checklist

1. Persist one native identity and run one live owner of that Peer ID.
2. Authenticate and select one explicit Domain.
3. Start `AukiPeer` through prepared or external authority.
4. Mount only the exact protocol endpoints the product serves.
5. Move outbound calls to protocol Clients.
6. Keep commands, capabilities, and safety policy in the product.
7. Replace implicit time with named clocks and recorded transforms.
8. Choose opt-in DDS discovery or exchange Peer IDs and routes explicitly.
9. Close endpoints before the peer and attempt every cleanup barrier.
10. Test wrong Peer ID, wrong Domain, expiry, rotation, malformed frames, and
    unavailable relay behavior.

Use the [P2P guide](p2p/README.md) for the current architecture and the
[portable echo walkthrough](p2p/getting-started.md) for an end-to-end proof.
