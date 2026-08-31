# Portable echo protocol

This example proves that one application protocol can be implemented once in
Rust and reused by multiple Auki peer runtimes.

```text
protocol/       exact ID, wire format, validation, and conversation
native/         AukiPeer adapter and runnable two-peer example
web/            Rust/Wasm adapter (after browser-peer feasibility)
```

`protocol/` deliberately has no dependency on Tokio, libp2p, `auki-sdk`,
`wasm-bindgen`, or browser APIs. The native example provides an authenticated
`AukiPeer` stream and calls those unchanged protocol functions. The Web adapter
will do the same after browser-peer feasibility is proven.

Run the portable gate with:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo clippy --locked -p auki-portable-echo-protocol --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo-protocol --target wasm32-unknown-unknown
```

See [`native/README.md`](native/README.md) for the relay-backed two-peer proof.
