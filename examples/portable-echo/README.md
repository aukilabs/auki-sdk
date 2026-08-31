# Portable echo protocol

This example proves that one application protocol can be implemented once in
Rust and reused by multiple Auki peer runtimes.

```text
protocol/       exact ID, wire format, validation, and conversation
adapter/        shared AukiPeer mount, deadlines, cleanup, and observations
native/         thin runnable native host
web/            browser host and browser/native relay proof (next consumer)
```

`protocol/` deliberately has no dependency on Tokio, libp2p, `auki-sdk`,
`wasm-bindgen`, or browser APIs. `adapter/` provides the protocol-specific
runtime glue once using the canonical cross-target `auki-sdk` surface. The
native host uses it now; the Web host is the next migration.

Run the portable gate with:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo test --locked -p auki-portable-echo-adapter
cargo clippy --locked -p auki-portable-echo-protocol --all-targets -- -D warnings
cargo clippy --locked -p auki-portable-echo-adapter --all-targets -- -D warnings
cargo check --locked -p auki-portable-echo-adapter --target wasm32-unknown-unknown
```

See [`native/README.md`](native/README.md) for the relay-backed two-peer proof.
See [`web/README.md`](web/README.md) for the ephemeral browser Peer adapter.
