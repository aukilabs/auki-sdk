# Build on Auki P2P

`AukiPeer` is the normal way to build an authenticated native Rust, Web,
Python, or Swift peer.
It hides authority renewal, libp2p setup, relay booking, route fencing, and
shutdown while leaving product policy explicit.

- [Run an existing protocol](getting-started.md)
- [Author a portable protocol](authoring-protocols.md)
- [Discover peers by mounted protocol](discovery.md)
- [Choose or prototype a discovery provider](discovery-providers.md)
- [Network typed Components and Products](component-protocols.md)
- [Exercise all six standard protocols](../../examples/standard-protocols/README.md)
- [Try the Web echo app](../../examples/portable-echo/web/README.md)
- [Try the Python echo app](../../examples/portable-echo/python/README.md)
- [Try the Swift/iOS echo app](../../examples/portable-echo/swift/README.md)

## Mental model

```text
User/App credentials -> PreparedPeer ---------------------+
                                                           |
external authority -> ExternalAuthorityUpdate + control --+-> AukiPeer
                                                                 |
                                              authority · routes · protocols
                                                                 |
                                                        Client / Endpoint
```

A Domain is the DDS physical-space and authority boundary. `AukiPeer` is the
runtime. A route says where to dial; the authenticated stream proves which Peer
ID and Domain answered.

## Start a peer

Native User and trusted App flows use `auki-auth`:

1. authenticate credentials;
2. list and select an accessible Domain;
3. load a persistent `Identity`;
4. authorize that exact Peer ID; and
5. start `AukiPeer`.

Relay-backed reachability is the default. Startup completes only when the peer
has valid authority, transport, and a confirmed relay-provider route pair.
One booking requests `relay_count` provider slots; each slot owns one
reservation and one atomic TCP/WSS pair. Native and Python reserve over TCP;
Web reserves over WSS. The count adds redundant providers, not transports.
Native `direct_only()` is an explicit alternative that makes no relay booking;
an inbound direct peer must also publish a reachable listener route.

Robot and Compute hosts use `AukiPeer::start_external` when product
infrastructure already manages authority. They do not need a second transport
or relay implementation.

Native and Python identities are persistent and single-owner. Do not run two
processes or pods with the same Peer ID. Browser identity is intentionally
ephemeral. Swift exposes identity generation and canonical bytes; each iOS
application decides whether to persist them. The echo example does not.

## Serve and call protocols from Rust

`auki-protocols` has no default features. A wire feature exposes types and
codecs; an `*-endpoint` feature additionally exposes the portable runtime API.

```toml
[dependencies]
auki-protocols = { path = "crates/auki-protocols", features = [
  "catalog-endpoint",
  "stream-endpoint",
] }
```

The API split is consistent:

- `Client` is cloneable and owns outbound operations.
- `Endpoint` owns inbound registration and close.
- `Provider` supplies application data and admission decisions.

```rust,ignore
let provider = MyCatalogProvider::new();
let endpoint = CatalogEndpoint::mount(peer.protocols(), provider)?;
let client = CatalogClient::new(peer.protocols());

let resources = client
    .fetch_resources_exact(remote_peer, route, ResourcesRequest::all())
    .await?;

endpoint.close().await?;
peer.shutdown().await?;
```

A client-only peer does not mount an endpoint. Mounting is the explicit opt-in
to serve that exact protocol.

## SDK-owned protocol families

| Family | Hosted versions | Rust | Web/Wasm | Python |
| --- | --- | --- | --- | --- |
| Info | v1 | call + serve | call + serve | call + serve |
| Catalog | v3 resources, v4 maps | call + serve | call + serve | call + serve |
| Registry | v3 | list/fetch + serve | list/fetch + serve | list/fetch + serve |
| Blob | v1 | fetch + serve | fetch + serve | fetch + serve |
| Message | v1 | send + receive | send + receive | send + receive |
| Stream | v2 | consume + produce | consume + produce | consume + produce |

The [standard protocol playground](../../examples/standard-protocols/README.md)
is the credentialed interoperability proof for this table. It starts Native,
Python, Browser A, and Browser B peers, proves all 12 directed DDS discovery
observations, then exercises all six families through the retained candidates
across eight directed edges: 48 checks in total, including both
browser-to-browser directions. The portable echo remains the small
custom-protocol authoring reference and supports call + serve on all four
hosts.

Typed Component applications can instead use the standalone
`auki-component-protocol` family. It layers revisioned Component/Product
discovery, retained observation reads, remote Product mirrors, and typed
Operable invocation over the same authenticated peer without depending on
`auki-protocols`. See [Network typed Components](component-protocols.md).

Catalog v2 is wire-only because v3 embeds its locked log-row shape. Registry
support begins at v3. Portable endpoints do not host or negotiate an older
fallback.

## Mechanical native providers

Most applications should not rewrite filesystem or Session projection logic:

- `SessionProtocolProvider` implements `CatalogProvider` and `StreamProvider`
  for one exact local `Peer` + `Session`.
- `FsRegistryProvider` serves validated, content-addressed Registry v3 entries
  from one fixed application root and local Peer ID.
- `FsBlobProvider` serves bounded Blob v1 ranges from one fixed application
  root.

```toml
auki-protocols = { path = "crates/auki-protocols", features = [
  "session-adapter",
  "registry-fs-provider",
  "blob-fs-provider",
] }
```

Providers receive the authenticated requester where authorization may matter.
The built-in mechanical providers serve authenticated callers uniformly; wrap
or replace them when product policy requires filtering.

## Web

`auki-sdk-web::AukiUserSession` logs in a User, lists Domains, and starts one
ephemeral relay-backed peer. Browser peers reserve over an exact WSS circuit
route and expose the same slot's required TCP route for native/Python callers.
They do not accept App secrets.

The opt-in JavaScript facade reuses the portable Rust implementations and
exposes both client and serving roles for Info, Catalog, Registry, Blob,
Message, and Stream. Stream payloads remain Rust-validated protobuf bytes.
Product protocols use a thin `wasm-bindgen` adapter compiled into the same
Wasm module; live Rust handles cannot cross independently instantiated Wasm
modules.

The [standard protocol browser node](../../examples/standard-protocols/web/README.md)
mounts all six families in one page and can probe another browser tab, native
node, or Python node. The matrix proves Browser A → Browser B and Browser B →
Browser A as separate WSS-route cases.

## Python

`auki-sdk-py` binds the canonical native runtime instead of rebuilding it in
Python. `AukiSession` authenticates a User or trusted App, lists Domains, and
starts one persistent relay-backed peer. A protocol adapter compiles into the
same extension module and receives the live Rust protocol handle internally.

The [portable echo Python host](../../examples/portable-echo/python/README.md)
is the reference: Python owns a few lines of application flow while Rust owns
the executor, wire protocol, transport, relay booking, and ordered cleanup.
The built-in binding exposes both client and serving roles for all six standard
protocol families. Provider callbacks receive verified requester metadata, not
credentials or authentication proofs.

The [standard protocol playground](../../examples/standard-protocols/README.md)
drives the Python node against Native and Browser peers using the same six
Rust implementations.

## Swift/iOS

`auki-sdk-swift` exposes the native Rust peer lifecycle through UniFFI: User
login, Domain listing, identity, relay-backed startup, routes, status, and
ordered shutdown. Mobile applications do not accept App secrets.

A product protocol adds a thin UniFFI adapter and links it with the peer facade
into one umbrella XCFramework. Rust owns the protocol and live handles; Swift
owns UI and platform lifecycle. Do not link independent Rust XCFrameworks for
the peer and protocol into one application.

The [portable echo iOS app](../../examples/portable-echo/swift/README.md) is the
small custom-protocol reference. The
[standard protocol iOS app](../../examples/standard-protocols/swift/README.md)
mounts Info, Catalog, Registry, Blob, Message, and Stream through the same Rust
implementations and probes native or browser peers through exact relay routes.
The live physical-device gate currently proves iPhone-to-native and
native-to-iPhone traffic for all six families; Swift/browser remains a separate
manual gate.

## Opt-in discovery

Relay makes a peer reachable; discovery lets other peers find that route. The
optional DDS tracker publishes short-lived Peer ID, route, and exact mounted
protocol hints. Applications explicitly choose `DiscoverOnly` or
`DiscoverAndAdvertise`, then call `discover()` or `discover_protocol()`.

Candidates remain untrusted until an exact-route protocol operation verifies
the expected Peer ID and Domain. Discovery never inserts a candidate into
`known_peers()`. Manual peer cards and product control planes remain supported
when DDS discovery is disabled. See [Discover peers](discovery.md).

## Lifecycle rules

- Bound every frame, queue, concurrent handler count, and deadline.
- Keep product authorization separate from Domain authentication.
- Use exact routes for portable native/Web/Python/Swift dialing.
- Close endpoints, senders, and receivers and cancel subscriptions before
  shutting down the peer.
- Attempt cleanup even when the application operation fails.
- On native Rust, treat `known_peers()` as observation only.

Use `auki-p2p` directly only when implementing transport/runtime machinery.
Applications should normally stay on `AukiPeer`.
