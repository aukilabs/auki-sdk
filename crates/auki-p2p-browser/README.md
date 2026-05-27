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
WASM initialization, a js-libp2p transport factory, and a high-level peer
shell. Protocol frame and message validation is intentionally routed through
`auki-protocol-wasm` instead of a TypeScript reimplementation.
