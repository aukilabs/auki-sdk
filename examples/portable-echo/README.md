# Portable echo protocol

This is the copyable reference for implementing one application protocol once
in Rust and using it from native and Web Auki peers.

```text
protocol/       author-owned ID, wire format, validation, and conversation
adapter/        author-owned AukiPeer mount, deadlines, cleanup, and events
native/         small Rust app plus a separate protected-interop binary
web/            small browser app, richer playground, and protected smoke proof
```

`protocol/` has no dependency on Tokio, libp2p, `auki-sdk`, wasm-bindgen, or
browser APIs. `adapter/` implements the protocol-specific runtime glue once on
the canonical cross-target `AukiPeer` surface. Both platform hosts consume that
same adapter; neither reimplements the echo wire contract.

Start with [Author a portable Auki protocol](../../docs/p2p/authoring-protocols.md)
for the create, version, test, mount, dial, advertise, and release workflow.

## Validate the shared implementation

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo test --locked -p auki-portable-echo-adapter
cargo test --locked -p auki-portable-echo-native
cargo clippy --locked \
  -p auki-portable-echo-protocol \
  -p auki-portable-echo-adapter \
  -p auki-portable-echo-native \
  --all-targets -- -D warnings
cargo check --locked \
  -p auki-portable-echo-protocol \
  -p auki-portable-echo-adapter \
  -p auki-portable-echo-web \
  --target wasm32-unknown-unknown
```

The [native README](native/README.md) runs the small two-terminal Rust app and
keeps protected interop output in a separate executable. The
[Web README](web/README.md) distinguishes the minimal browser app from the
richer playground and four-direction protected smoke test.

Peer discovery and automatic route publication remain outside this example.
The apps exchange an expected Peer ID and exact confirmed route explicitly;
the SDK authenticates both peers before echo bytes flow.
