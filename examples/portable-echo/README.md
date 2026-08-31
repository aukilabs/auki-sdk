# Portable echo protocol

This example proves that one application protocol can be implemented once in
Rust and reused by multiple Auki peer runtimes.

```text
protocol/       exact ID, wire format, validation, and conversation
native/         AukiPeer adapter (next)
web/            Rust/Wasm adapter (after browser-peer feasibility)
```

Only `protocol/` exists in the first proof. It deliberately has no dependency
on Tokio, libp2p, `auki-sdk`, `wasm-bindgen`, or browser APIs. Native and Web
adapters will provide an authenticated duplex stream and call the same
protocol functions.

Run the portable gate with:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo clippy --locked -p auki-portable-echo-protocol --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo-protocol --target wasm32-unknown-unknown
```
