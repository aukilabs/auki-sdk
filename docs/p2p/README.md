# Auki P2P

Build authenticated peer-to-peer applications without assembling authority,
libp2p, relay booking, or shutdown machinery yourself.

**[Build with an existing protocol](getting-started.md)** ·
**[Author one protocol crate](authoring-protocols.md)** ·
**[Try the Web peer](../../examples/portable-echo/web/README.md)**

## The mental model

An `AukiPeer` is one cryptographic Peer ID operating inside one authorized DDS
Domain.

```text
credentials + selected Domain + identity proof
                       |
                       v
                  PreparedPeer
                       |
                       v
                   AukiPeer
              /          |          \
       authority       routes       protocols
        renewal       and relay     and app data
```

Native applications normally persist their identity and reuse the same Peer ID
after restart. The Web `0.1` facade intentionally creates an in-memory identity
for each peer start, so a reload or new start gets a new Peer ID.

Authority and reachability are separate:

- A credential proves which Peer ID may participate in which Domain.
- A route tells the transport where to dial that peer.
- A relay makes a peer reachable; it does not discover other peers.
- Application policy still decides who may invoke a command or capability.

## The normal application path

### Native Rust

1. Authenticate a User or trusted App with `auki-auth`.
2. Select one accessible Domain.
3. Load a persistent identity and authorize its Peer ID.
4. Start `AukiPeer` with `AukiPeerConfig`; relay reachability is the default.
5. Mount only the application protocols this peer serves.
6. Dial an expected Peer ID through its exact advertised route.
7. Close mounted protocol endpoints, then await `peer.shutdown()`.

`AukiPeer::start` owns credential renewal, authenticated transport, DMS relay
booking, route state, supervision, fencing, and ordered shutdown. Native hosts
may explicitly choose `AukiPeerConfig::direct_only()`, which makes no relay
booking calls. A direct-only peer may use zero listeners and advertised routes
for outbound-only operation. Inbound direct reachability requires a listener
plus a dialable route shared by the application. Configure an advertised direct
route only when the application publishes it from the SDK's local route
catalog.

### Web

The Rust/Wasm binding follows the same model with a deliberately smaller
surface. `AukiUserSession` authenticates a User, lists accessible Domains, and
starts an ephemeral `AukiPeer`. Browser peers always acquire one confirmed WSS
relay route; there is no direct-only browser mode in `0.1`.

Protocol implementations remain in Rust. A thin binding such as `AukiEcho`
mounts the same Rust endpoint used by native hosts on the generic browser peer.
See the
[minimal browser echo](../../examples/portable-echo/web/README.md#copy-the-minimal-app).
App access keys and secrets are never accepted by the browser facade.

## What remains explicit

| Application or protocol owner provides | SDK owns |
| --- | --- |
| Credentials and exact Domain choice | Authentication proof and renewable authority |
| Native identity storage path | Authenticated transport lifecycle |
| Exact protocol opt-ins and product policy | Mutually authenticated protocol streams |
| Remote Peer ID and complete route | Route validation and exact-peer dialing |
| Where peer information is shared | Relay booking, recovery, fencing, and cleanup |

Discovery and route publication are not hidden inside authentication. In
`0.1`, exchange the remote Peer ID, Domain ID, supported protocol IDs, and
complete TCP or WSS route through configuration, a product control plane, or a
manual peer card. That record is application data, not authority, and the SDK
still authenticates the remote Peer ID and Domain before application bytes
flow.

## Protocols are explicit

A new peer serves no product protocol by default. Product owners choose a
bounded, explicitly versioned ID shaped like
`/<namespace>[/<name>...]/<version>`, for example
`/example/echo/1.0.0`. The top-level `/auki/` namespace is reserved for SDK
protocols.

```rust
let endpoint = MyProtocolEndpoint::mount(peer.protocols())?;
```

Keep the endpoint alive while serving. A client-only peer does not need to
mount the corresponding inbound protocol. One product-owned protocol crate
keeps its wire contract in a transport-neutral private module and its mounting,
dialing, deadlines, events, and cleanup in a private endpoint module. Native
and Web hosts consume that one public endpoint API.

Follow [Author one protocol crate](authoring-protocols.md) before assigning a
new ID. An exact ID must never acquire a second wire format.

## Safe defaults and limits

- Native identity corruption fails closed; it never silently creates a new
  Peer ID over invalid material.
- Routes and `known_peers()` never grant authority.
- Remote operations authenticate the expected Peer ID and exact Domain.
- Relay-backed startup is the native default and the Web requirement.
- Explicit endpoint close and `peer.shutdown()` are the external cleanup
  barriers.
- One live runtime per Peer ID is supported. Native replicas need distinct
  identities; a persisted identity belongs to one live process or pod.
- App secrets belong only in trusted native or headless processes, never a
  browser or distributed mobile application.

Robot and Compute hosts that receive authority from a product control plane may
use native `AukiPeer::start_external`. They retain task or heartbeat policy;
the facade still owns transport, routes, protocols, fencing, and shutdown.

## Platform status

| Platform | High-level authenticated peer facade |
| --- | --- |
| Native Rust | User/App auth, persistent identity, renewal, relay, protocols, shutdown |
| Web/Wasm | User auth, ephemeral identity, mandatory WSS relay, exact-route protocols, shutdown |
| Python | Pending on the canonical `AukiPeer` runtime |
| Swift/iOS | Pending |

## Keep these rules

> A route tells you where to dial. A credential tells you who may enter.

> A relay makes your peer reachable. It does not tell you which peers exist.

> A protocol ID names one immutable wire contract. Change the contract, change
> the ID.
