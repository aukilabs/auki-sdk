# Portable echo protocol

This is the copyable reference for implementing one application protocol once
in Rust and using it from native, Web, Python, and Swift Auki peers.

```text
src/wire.rs     author-owned ID, wire format, validation, and conversation
src/endpoint.rs author-owned AukiPeer mount, deadlines, cleanup, and events
native/         small Rust app plus a separate protected-interop binary
web/            small browser app plus a protected four-direction smoke proof
python/         small Python app plus its same-module PyO3 adapter
swift/          small iOS app plus its same-artifact UniFFI adapter
```

The private `wire` module has no dependency on Tokio, libp2p, `auki-sdk`,
wasm-bindgen, or browser APIs. The private `endpoint` module implements the
protocol-specific runtime glue once on the canonical cross-target `AukiPeer`
surface. All four platform hosts consume the same crate; none reimplements
the echo wire contract.

Start with [Build with an existing protocol](../../docs/p2p/getting-started.md)
when writing an application. Use
[Author one portable Auki protocol crate](../../docs/p2p/authoring-protocols.md)
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
[Web README](web/README.md) runs the copyable root app in two tabs and keeps the
four-direction protected smoke test as separate test machinery. The
[Python README](python/README.md) runs the same endpoint Python-to-Python or in
either Python/native direction.
The [Swift/iOS README](swift/README.md) builds the umbrella XCFramework and
runs the same endpoint bidirectionally against native Rust in a simulator.

Each host enables DDS discovery at the application boundary. The portable
protocol crate remains discovery-agnostic: a host refreshes candidates for the
exact Echo protocol, a developer selects one, and the existing exact-route
operation authenticates the expected Peer ID and Domain before bytes flow.
`DiscoverAndAdvertise` is the example default; every host also exposes
`DiscoverOnly`, and manual exact targets remain an explicit fallback.
