# auki-sdk-web

Generic Rust/Wasm composition for authenticated Auki browser peers.

JavaScript uses `AukiUserSession` to authenticate a User, list accessible
Domains, and start an ephemeral `AukiPeer`. Relay-backed remains the compatible
default and exposes a TCP/WSS circuit-route pair from one provider slot. The
browser reserves over WSS; its TCP route lets native and Python peers dial the
same reservation. `AukiPeerReachabilityMode.OutboundOnly` skips that booking
and exposes neither route, while still allowing authenticated dials to a remote
peer's WSS route. `AukiPeer.shutdown()` is the awaited cleanup barrier, while
`AukiPeer.waitStopped()` reports unexpected terminal transport, authority, or
relay failure to the application.

The binding delegates authentication, explicit Domain authorization, ephemeral
identity creation, and peer startup to Rust's `AukiPeerBootstrap`. This crate
only maps browser values, object ownership, and Promise errors.

DDS discovery is opt-in. Keep `startPeer(domainId)` for a private peer, or use
`startPeerWithDiscovery(domainId, AukiDiscoveryMode.DiscoverOnly)` to look up
fresh candidates without publishing. Select `DiscoverAndAdvertise` when the
peer should also maintain a short-lived advertisement. `peer.discover()` and
`peer.discoverProtocol(exactId)` return untrusted route hints; protocol dialing
still verifies the expected Peer ID and Domain in Rust.

Reachability is an optional final startup argument. Omitting it keeps the
relay-backed default; pass `OutboundOnly` to opt out:

```ts
const peer = await session.startPeerWithDiscovery(
  domainId,
  AukiDiscoveryMode.DiscoverOnly,
  AukiPeerReachabilityMode.OutboundOnly,
);
```

For an outbound-only peer, `peer.relayBacked` is `false` and `peer.tcpRoute`
and `peer.wssRoute` are `undefined`. `OutboundOnly` may be combined with no DDS
tracker or `DiscoverOnly`; `DiscoverAndAdvertise` is rejected because there is
no inbound route to publish.

## Built-in protocol bindings

Protocol behavior remains implemented in `auki-protocols`; these features
expose selected roles to JavaScript without duplicating the wire contract:

| Feature | JavaScript surface |
| --- | --- |
| `info` | `AukiInfoClient` and provider-backed `AukiInfoEndpoint` |
| `catalog` | `AukiCatalogClient` and provider-backed `AukiCatalogEndpoint` |
| `registry` | `AukiRegistryClient` and provider-backed `AukiRegistryEndpoint` |
| `blob` | `AukiBlobClient` and provider-backed `AukiBlobEndpoint` |
| `finite-protocols` | convenience feature enabling the four families above |
| `message` | `AukiMessageClient`, `AukiMessageEndpoint`, sender, and receiver |
| `stream` | `AukiStreamClient`, producer-backed `AukiStreamEndpoint`, and subscription |

Every client supports peer-configured routing and exact `{ peerId, route }`
dialing. Endpoints and Message handles expose awaited `close()` barriers;
Stream subscriptions expose awaited `cancel()`. Stream producers return an
async iterable, and entries contain Rust-validated protobuf bytes as
`Uint8Array`.

Callbacks receive verified requester metadata, never credentials or proofs.
Info, Catalog, Registry, and Stream admission callbacks are synchronous and
should return promptly. Blob providers may return a value or `Promise`, and
Stream sources are async iterables. A custom product protocol still compiles a
thin `wasm-bindgen` adapter into this same Wasm module, as portable echo does.
Live Rust handles cannot cross independently instantiated Wasm modules.

Browser identities are intentionally in-memory. This crate does not persist
Peer IDs, expose raw transport streams, reconnect automatically, or accept app
access keys and secrets. A trusted backend can issue short-lived authority for
non-User browser flows later.
