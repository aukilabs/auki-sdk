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
| `info` | outbound `AukiInfoClient` |
| `catalog` | outbound `AukiCatalogClient` |
| `registry` | outbound `AukiRegistryClient` |
| `blob` | outbound `AukiBlobClient` |
| `finite-protocols` | convenience feature enabling the four clients above |
| `message` | `AukiMessageClient`, `AukiMessageEndpoint`, sender, and receiver |
| `stream` | outbound `AukiStreamClient` and consumer subscription |

Every outbound operation uses an exact `{ peerId, route }` target. Message
senders, receivers, and endpoints expose awaited `close()` barriers. Stream
subscriptions expose awaited `cancel()`; entries contain Rust-validated
protobuf bytes as `Uint8Array`.

This JavaScript surface is intentionally narrower than the portable Rust
surface. Info, Catalog, Registry, and Blob do not expose JavaScript providers,
and Stream does not expose a JavaScript producer or endpoint. A browser that
must serve one of those roles can compile a thin application-specific
`wasm-bindgen` adapter into the same Wasm module, as the portable echo example
does. Live Rust handles cannot cross independently instantiated Wasm modules.

Browser identities are intentionally in-memory. This crate does not persist
Peer IDs, expose raw transport streams, reconnect automatically, or accept app
access keys and secrets. A trusted backend can issue short-lived authority for
non-User browser flows later.
