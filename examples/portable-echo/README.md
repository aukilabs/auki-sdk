# Portable echo protocol

This example proves that one application protocol can be implemented once in
Rust and reused by multiple Auki peer runtimes.

```text
protocol/       exact ID, wire format, validation, and conversation
native/         AukiPeer adapter and runnable two-peer example
web/            Rust/Wasm adapter (browser/native relay proof follows)
```

`protocol/` deliberately has no dependency on Tokio, libp2p, `auki-sdk`,
`wasm-bindgen`, or browser APIs. The native and Web adapters each provide an
authenticated stream and call those unchanged protocol functions.

Run the portable gate with:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo clippy --locked -p auki-portable-echo-protocol --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo-protocol --target wasm32-unknown-unknown
```

See [`native/README.md`](native/README.md) for the relay-backed two-peer proof.
See [`web/README.md`](web/README.md) for the ephemeral browser Peer adapter.
