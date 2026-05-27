# auki-p2p-browser

RFC-first browser peer package for `auki-p2p`.

This package is intentionally lower level than `auki-domain-browser`: it owns
browser peer identity, browser-compatible libp2p transport setup, native
bootstrap records, and RFC protocol helpers. App code should eventually use one
high-level `AukiBrowserPeer` handle instead of configuring libp2p streams
directly.

Status: WIP (v0.0.0). The current surface provides bootstrap parsing,
IndexedDB-backed seed persistence, identity derivation, frame helpers, a
js-libp2p transport factory, and a high-level peer shell.
