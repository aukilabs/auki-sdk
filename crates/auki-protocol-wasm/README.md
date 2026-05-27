# auki-protocol-wasm

Thin `wasm-bindgen` adapter over `auki-protocol`.

This crate exists so browser SDK code can use the same RFC validators,
constructors, frame helpers, and failure-code mapping as the Rust runtime
without reimplementing protocol rules in TypeScript.

Status: WIP (v0.0.0). The current surface wraps v1 JSON frames, peer/domain
authority objects, handshakes, offer catalogs, Get, Subscribe, spatial
messages, error objects, status snapshots, and selected constructor/verification
helpers needed by browser peers.
