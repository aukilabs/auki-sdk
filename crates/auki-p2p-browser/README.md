# auki-p2p-browser

RFC-first browser peer package for `auki-p2p`.

This package is intentionally lower level than `auki-domain-browser`: it owns
browser peer identity, browser-compatible libp2p transport setup, native
bootstrap records, and browser peer orchestration. Protocol validation and
message construction should come from `auki-protocol-wasm`, which wraps the
Rust `auki-protocol` crate instead of reimplementing RFC rules in TypeScript.
App code should eventually use one high-level `AukiBrowserPeer` handle instead
of configuring libp2p streams directly.

Status: WIP (v0.0.0). The current surface provides bootstrap parsing,
IndexedDB-backed seed persistence, identity derivation, Rust-backed protocol
WASM initialization, a js-libp2p transport factory, lifecycle handshakes,
remote offer-catalog loading, high-level Get and Subscribe consumption,
generic local offer publication, inbound offer-catalog/Get/Subscribe serving,
and manual selected-transport switching with one retained active path per peer.
Preview publishing is a helper/profile on top of the generic offer API.
Generated frames and camera capture are source choices outside the peer core.
Protocol frame and message validation is intentionally routed through
`auki-protocol-wasm` instead of a TypeScript reimplementation.

The browser demo currently proves browser-to-node over WebSocket and WebRTC
Direct, plus browser-to-browser over Circuit Relay and browser WebRTC through a
native relay/signaling node. Remaining work is mostly broader matrix evidence:
node-to-node demo smokes, browser-to-node relay fallback, relay-shutdown
observation after browser WebRTC setup, camera capture, and richer application
profiles beyond preview frames.
