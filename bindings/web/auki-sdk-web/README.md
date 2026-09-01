# auki-sdk-web

Generic Rust/Wasm composition for authenticated Auki browser peers.

JavaScript uses `AukiUserSession` to authenticate a User, list accessible
Domains, and start an ephemeral `AukiPeer`. A browser peer always acquires a
relay before startup completes and exposes a required TCP/WSS circuit-route
pair from that one provider slot as public peer-card data. The browser reserves
over WSS; its TCP route lets native and Python peers dial the same reservation.
`AukiPeer.shutdown()` is the awaited
cleanup barrier, while `AukiPeer.waitStopped()` reports unexpected terminal
transport or relay failure to the application.

The binding delegates authentication, explicit Domain authorization, ephemeral
identity creation, and peer startup to Rust's `AukiPeerBootstrap`. This crate
only maps browser values, object ownership, and Promise errors.

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
