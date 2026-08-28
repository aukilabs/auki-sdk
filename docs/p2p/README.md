# Auki P2P

Build authenticated peer-to-peer applications for robots, spatial tools, and
native services.

**[Get started with Rust](getting-started.md)** ·
**[Run the local two-peer demo](../../examples/diagnostic-app/README.md)**

> **Current scope:** Native Rust works today. User/App authentication is
> available. Applications still supply direct or confirmed relay routes;
> automatic User/App relay allocation and peer discovery are not yet part of
> the app-facing runtime.

## What you can build

- Publish robot resources and live streams to authenticated consumers.
- Read catalogs, metadata, blobs, messages, and streams from an expected peer.
- Add a versioned product protocol without building another P2P runtime.

## The idea

An Auki peer is not a username, a socket, or a row in a peer list. It is a
stable cryptographic identity operating inside one authorized Domain. A Domain
is the security boundary shared by peers that are allowed to communicate.

```text
stable identity ─┐
                 ├─► authenticated Domain ─► application protocols
Domain authority ┤
reachable routes ┘
```

Four inputs stay explicit:

1. **Identity — who am I?**

   A persistent Ed25519 key produces a stable Peer ID. Reuse it on every
   launch; changing it creates a different peer.

2. **Authority — which Domain may I enter?**

   Auki's Domain service (DDS) signs a short-lived credential for one Domain
   and one exact Peer ID. Native User and App flows are available through
   `auki-auth`.

3. **Reachability — how can another peer dial me?**

   A peer can listen directly. A relay integration can reserve a relay and
   distribute its complete circuit route; dialers install that confirmed
   route. A route is only a dial hint—it never grants access.

4. **Protocols — what does my application serve?**

   Catalogs, registries, blobs, messages, and streams are explicit opt-ins.
   A new Domain serves nothing by default.

This separation is intentional. Authentication does not choose topology;
discovery does not grant authority; connecting to a peer does not authorize an
application command.

## The path most applications use

```text
User/App credentials
      │
      ▼
  auki-auth ─────► validated Domain authority
                         │
persistent Identity ─────┤
explicit routes ─────────┤
                         ▼
                    auki-domain
                         │
                         ▼
             catalogs · blobs · messages · streams
```

| Layer | Use it for |
| --- | --- |
| `auki-auth` | User/App login, Domain selection, Peer-ID proof, and an explicit authority-renewal operation |
| `auki-domain` | One authenticated peer lifecycle, routes, status, known peers, and hosted protocols |
| `auki-session` | Local peer metadata, registries, one recording timeline, clocks, and logs |
| `auki-protocols` | Exact versioned wire contracts; usually consumed through `auki-domain` |
| `auki-p2p` | Stable identity and advanced transport for custom runtimes or relay integrations |

Most applications should begin with `auki-auth` + `auki-domain`. Register an
application protocol through `domain.protocols()`. Reach for `auki-p2p`
directly only when implementing a custom runtime or advanced transport
integration.

In the Rust API, `auki_session::Peer` owns local application data and
registries; `auki_domain::Domain` owns the actual network lifecycle.

## What starting a peer does

1. Load or create one persistent identity.
2. Authenticate a User or trusted App.
3. Select an authorized Domain.
4. Prove that the process owns the selected Peer ID.
5. Start one Domain with listeners, routes, and explicit protocol opt-ins.
6. Communicate only after both peers authenticate the same Domain and expected
   Peer IDs.
7. Renew authority before it expires and leave with an ordered shutdown.

The SDK never exposes application bytes before mutual authentication succeeds.

## Safe defaults

- Identity corruption fails closed; the SDK does not silently replace the key.
- A Domain serves no built-in protocol unless the application opts in.
- Routes and `known_peers()` are never authorization.
- Remote operations target an expected Peer ID, not merely an address.
- `leave().await` owns listener and task cleanup.
- App secrets belong only in trusted native or headless processes—not browsers
  or distributed mobile applications.

## What is available today

| Platform | Current authenticated P2P surface |
| --- | --- |
| Rust | Identity, User/App auth, Domain lifecycle, protocols, direct and relay transport primitives |
| Python | Rust-owned Domain lifecycle and protocols; high-level auth/bootstrap facade is still pending |
| Swift/iOS | Wallet binding only; authenticated P2P facade is still pending |
| Web | Authenticated browser transport and facade are still pending |

The current Rust source line does not automatically discover peers or allocate
a relay for User/App peers. Applications supply exact-peer routes from
configuration or their own control-plane adapter. A complete confirmed relay
route can be installed through the same route API as a direct route.

## Start here

- **First experiment:** follow [Getting started with Rust](getting-started.md).
- **No credentials yet:** run the
  [local two-peer diagnostic](../../examples/diagnostic-app/README.md).
- **Migrating old Manager code:** read the
  [authenticated Domain migration guide](../authenticated-domain-migration.md).
- **Writing a custom protocol:** start with
  [`domain.protocols()`](../../crates/auki-domain/README.md#public-lifecycle-and-transport-views).

## Keep these rules

> A route tells you where to dial. A credential tells you who is allowed in.

> `known_peers()` tells you who is authenticated and connected now. It is not
> a Domain roster or discovery service.

> Domain access permits authenticated communication. Each application still
> decides who may invoke a command or capability.
