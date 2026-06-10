# auki-protocol-wasm

Thin `wasm-bindgen` adapter over `auki-protocol`.

This crate exists so browser SDK code can use the same RFC validators,
constructors, frame helpers, and failure-code mapping as the Rust runtime
without reimplementing protocol rules in TypeScript.

Status: WIP (v0.0.0). The current surface wraps v1 JSON frames, peer/domain
authority objects, handshakes, offer catalogs, Get, Subscribe, spatial
messages, error objects, status snapshots, and selected constructor/verification
helpers needed by browser peers.

Design rules:

- Keep protocol behavior in `auki-protocol`.
- Keep this crate as a JS/WASM boundary adapter.
- Do not add libp2p transport, stream orchestration, Discovery, relay, or app
  policy here.
- Prefer one Rust-backed helper over duplicated TypeScript protocol logic.
