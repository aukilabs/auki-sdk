# auki-sdk-web

Generic Rust/Wasm composition for authenticated Auki browser peers.

JavaScript uses `AukiUserSession` to authenticate a User, list accessible
Domains, and start an ephemeral `AukiPeer`. A browser peer always acquires a
relay before startup completes and exposes its confirmed WSS and optional TCP
circuit routes as public peer-card data. `AukiPeer.shutdown()` is the awaited
cleanup barrier, while `AukiPeer.waitStopped()` reports unexpected terminal
transport or relay failure to the application.

The binding delegates authentication, explicit Domain authorization, ephemeral
identity creation, and peer startup to Rust's `AukiPeerBootstrap`. This crate
only maps browser values, object ownership, and Promise errors.

Protocol implementations stay in Rust. An application-specific adapter
compiled into the same Wasm module obtains `AukiPeerProtocols` through the
Rust-only `AukiPeer::protocols()` method. Live Rust handles cannot be shared
between independently instantiated Wasm modules.

Browser identities are intentionally in-memory. This crate does not persist
Peer IDs, expose raw transport streams, reconnect automatically, or accept app
access keys and secrets. A trusted backend can issue short-lived authority for
non-User browser flows later.
