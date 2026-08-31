# auki-protocols

SDK-owned authenticated application protocols for `auki_sdk::AukiPeer`.

The crate is compile-time opt-in and has no default features. Each family has a
wire feature; active families also have an endpoint feature that adds the
portable runtime integration.

## Feature map

| Wire feature | Runtime feature | Hosted versions |
| --- | --- | --- |
| `info` | `info-endpoint` | Info v1 |
| `catalog` | `catalog-endpoint` | Catalog v3 resources and v4 maps |
| `registry` | `registry-endpoint` | Registry v3 |
| `blob` | `blob-endpoint` | Blob v1 |
| `message` | `message-endpoint` | Message v1 |
| `stream` | `stream-endpoint` | Stream v2 |

Catalog v2 is wire-only because v3 embeds its locked log-row shape. Registry
support begins at v3. No portable Endpoint hosts or negotiates an older
fallback.

```toml
auki-protocols = { path = "../auki-protocols", features = [
  "catalog-endpoint",
  "stream-endpoint",
] }
```

## API shape

Each runtime family separates:

- a cloneable outbound `Client`;
- an inbound/lifecycle `Endpoint`; and
- an application-owned `Provider` where data or admission is needed.

Mounting is explicit:

```rust,ignore
let endpoint = CatalogEndpoint::mount(peer.protocols(), provider)?;
let client = CatalogClient::new(peer.protocols());
```

Providers receive `AuthenticatedPeer` where caller-aware policy is useful.
Endpoints own fixed concurrency, frame limits, deadlines, stream cleanup, and
registration close. Close mounted Endpoints before `AukiPeer::shutdown`.

## Native adapters

Three opt-in features remove common application plumbing:

| Feature | Type | Purpose |
| --- | --- | --- |
| `session-adapter` | `SessionProtocolProvider` | Project one exact local `Peer` + `Session` into Catalog v3/v4 and Stream v2 |
| `registry-fs-provider` | `FsRegistryProvider` | Serve validated Registry v3 entries from one fixed root and local Peer ID |
| `blob-fs-provider` | `FsBlobProvider` | Serve bounded Blob v1 ranges from one fixed root |

These adapters are native-only and read/project existing local state. They do
not choose product authorization; wrap or replace them when requester-specific
policy is required.

## Portability

Wire codecs operate on portable `futures` async I/O. Endpoint implementations
compile for native and Wasm, using `Arc`/`Send + Sync` providers natively and
executor-local `Rc` providers in a browser.

The protocol ID is immutable wire identity. Locked tests pin representative
JSON/protobuf bytes. Incompatible changes require a new protocol ID rather than
a hidden fallback.

See [the P2P guide](../../docs/p2p/README.md) and
[protocol authoring guide](../../docs/p2p/authoring-protocols.md).
