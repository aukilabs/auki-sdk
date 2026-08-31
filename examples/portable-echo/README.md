# Portable echo protocol

This is the copyable reference for implementing one application protocol once
in Rust and using it from native and Web Auki peers.

```text
src/wire.rs     author-owned ID, wire format, validation, and conversation
src/endpoint.rs author-owned AukiPeer mount, deadlines, cleanup, and events
native/         small Rust app plus a separate protected-interop binary
web/            small browser app, richer playground, and protected smoke proof
```

The private `wire` module has no dependency on Tokio, libp2p, `auki-sdk`,
wasm-bindgen, or browser APIs. The private `endpoint` module implements the
protocol-specific runtime glue once on the canonical cross-target `AukiPeer`
surface. Both platform hosts consume the same crate; neither reimplements the
echo wire contract.

Start with [Author a portable Auki protocol](../../docs/p2p/authoring-protocols.md)
for the create, version, test, mount, dial, advertise, and release workflow.

## Validate the shared implementation

```sh
cargo test --locked -p auki-portable-echo
cargo test --locked -p auki-portable-echo-native
cargo clippy --locked \
  -p auki-portable-echo \
  -p auki-portable-echo-native \
  --all-targets -- -D warnings
cargo check --locked \
  -p auki-portable-echo \
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
