# auki-portable-echo-protocol

Platform-neutral reference protocol shared unchanged by native Rust and Web
peers.

## Contract

- Protocol ID: `/example/echo/1.0.0`
- Conversation: one request followed by one identical response
- Payload: arbitrary non-empty bytes
- Frame: big-endian `u32` payload length followed by the payload
- Maximum payload: 1 KiB

The client rejects a response that differs from its request. Empty and
oversized frames are rejected before payload allocation or I/O respectively.

The exact protocol ID is immutable wire identity. A change to framing, bounds,
conversation, or observable semantics requires a new ID; a Cargo package bump
alone does not create a new wire protocol.

This crate owns no authentication, transport, runtime, timers, persistence, or
platform bindings. Its client and server functions accept any
`futures::AsyncRead + AsyncWrite` stream, allowing the same code to run over an
authenticated native or browser `AukiPeer` stream.

Its locked tests cover the ID and representative bytes, the client/server
conversation, mismatched responses, and empty or oversized input:

```sh
cargo test --locked -p auki-portable-echo-protocol
cargo check --locked -p auki-portable-echo-protocol \
  --target wasm32-unknown-unknown
```

See [Author a portable Auki protocol](../../../docs/p2p/authoring-protocols.md)
before assigning a new product protocol ID.
