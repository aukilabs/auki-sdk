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

## ZITADEL public-client handoff

The host performs PKCE login, obtains a refresh token, and stops all competing
refresh owners (including other tabs and its OAuth library) before importing.
`issuer` and `clientId` must come from trusted application configuration. Import
is synchronous and performs no network I/O: retain the returned handle even if
Domain lookup or peer startup subsequently fails.

```ts
const session = AukiUserSession.importZitadelDev({
  accessToken, refreshToken, clientId, issuer,
  accessTokenExpiresAt, // RFC 3339 string, or null/undefined if unknown
}, async replacement => {
  try {
    await hostSecureStore.saveAtomically({ // your host implementation
      accessToken: replacement.exposeAccessToken(),
      refreshToken: replacement.exposeRefreshToken(),
      clientId: replacement.clientId,
      issuer: replacement.issuer,
      accessTokenExpiresAt: replacement.accessTokenExpiresAt,
    });
  } finally {
    replacement.free();
  }
});
// Keep session in application state BEFORE the first await.
const domains = await session.accessibleDomains();
const peer = await session.startPeer(selectedDomainId); // explicit UI selection
// Observe peer.waitStopped() for unexpected terminal failures.
// Logout: stop your peers, drain storage, then erase the durable session.
await peer.shutdown();
await session.close();
await hostSecureStore.clear();
```

`importZitadelWithEnvironment(apiBase, ddsBase, dmsBase, credentials, store)` is
the equivalent for explicit service bases. Existing password methods are unchanged.

The store **must return a Promise** and acknowledge only after the whole snapshot
is durably, atomically saved. A throw, rejection, or missing Promise is a
`persistence` error; the SDK keeps the replacement in memory and retries saving
that same generation on the next operation. It does not exchange a service bearer
or rotate again before acknowledgement. On success **or failure**, all writes
started by the callback must have settled: no detached writes and no reentry into
this session. A callback that never settles prevents a safe completed logout.
Dropping an operation's Promise is not a way to cancel the session-owned save.

Auth errors have `name: "AukiAuthError"` and a typed `code`:
`authentication_required` means sign in again; `configuration` requires fixing
configuration; `authorization_denied` requires a readable Domain. `persistence`
and `transient` can be retried on the **same handle**. `closed` and `cancelled`
identify lifecycle termination. Terminal peer auth errors reach `waitStopped()`
with the same codes. Non-auth protocol/transport errors remain ordinary errors.
Closing a session is not immediate remote-token revocation and does not itself
shut down all peers; existing expiry delays still apply.

Credential snapshots are opaque/redacted; reading tokens requires the explicit
`expose*` methods. The initial host payload and explicitly extracted JS strings
are secrets: do not log them. Expiry snapshots use RFC 3339 UTC `Z`, preserving
nanoseconds; do not round-trip them through JS `Date` if precision matters.
Other existing timestamps remain RFC 3339 (including the equivalent `+00:00`
suffix). Subject strings are preserved byte-for-byte; never trim, normalize case
or Unicode, or parse human subjects as UUIDs.

See [binding runtime tests](../../../docs/zitadel-binding-tests.md) for real-browser
callback, failure/retry, logout, and timestamp/subject proof.

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
