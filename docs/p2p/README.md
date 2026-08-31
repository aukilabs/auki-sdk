# Build on Auki P2P

`AukiPeer` is the normal way to build an authenticated native Rust or Web peer.
It hides authority renewal, libp2p setup, relay booking, route fencing, and
shutdown while leaving product policy explicit.

- [Run an existing protocol](getting-started.md)
- [Author a portable protocol](authoring-protocols.md)
- [Try the Web echo app](../../examples/portable-echo/web/README.md)

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
has valid authority, transport, and a confirmed relay route. Native
`direct_only()` is an explicit alternative; an inbound direct peer must also
publish a reachable listener route.

Robot and Compute hosts use `AukiPeer::start_external` when product
infrastructure already manages authority. They do not need a second transport
or relay implementation.

Native identity is persistent and single-owner. Do not run two processes or
pods with the same Peer ID. Browser identity is intentionally ephemeral in the
first iteration.

## Serve and call protocols explicitly

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

| Feature | Hosted versions | API |
| --- | --- | --- |
| `info-endpoint` | Info v1 | `InfoClient`, `InfoEndpoint`, `InfoProvider` |
| `catalog-endpoint` | Catalog v3 and v4 | `CatalogClient`, `CatalogEndpoint`, `CatalogProvider` |
| `registry-endpoint` | Registry v3 | `RegistryClient`, `RegistryEndpoint`, `RegistryProvider` |
| `blob-endpoint` | Blob v1 | `BlobClient`, `BlobEndpoint`, `BlobProvider` |
| `message-endpoint` | Message v1 | `MessageClient`, `MessageEndpoint` |
| `stream-endpoint` | Stream v2 | `StreamClient`, `StreamEndpoint`, `StreamProvider` |

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
ephemeral relay-backed peer. Browser peers always use an exact WSS circuit
route and do not accept App secrets.

Protocol logic remains Rust. Compile the protocol crate and its thin
wasm-bindgen adapter into the same Wasm module as the peer; live Rust handles
cannot cross independently instantiated Wasm modules.

Python and Swift/iOS do not yet expose the canonical peer facade. Their current
bindings cover data and identity pieces only.

## Discovery is still application-owned

Relay makes a peer reachable but does not tell another peer that it exists.
Until discovery and route publication are designed, exchange:

```json
{
  "domainId": "<Domain UUID>",
  "peerId": "<Peer ID>",
  "protocols": ["/example/echo/1.0.0"],
  "routes": {
    "tcp": "<native circuit route>",
    "wss": "<browser circuit route>"
  }
}
```

This record is application data, not authority. The SDK still verifies the
expected Peer ID and Domain before exposing protocol bytes.

## Lifecycle rules

- Bound every frame, queue, concurrent handler count, and deadline.
- Keep product authorization separate from Domain authentication.
- Use exact routes for portable native/Web dialing.
- Close mounted endpoints before shutting down the peer.
- Attempt cleanup even when the application operation fails.
- On native Rust, treat `known_peers()` as observation only.

Use `auki-p2p` directly only when implementing transport/runtime machinery.
Applications should normally stay on `AukiPeer`.
