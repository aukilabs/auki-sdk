# auki-portable-echo-protocol

Platform-neutral reference protocol used to prove that native Rust and Web
peers can run one shared Rust implementation.

## Contract

- Protocol ID: `/example/echo/1.0.0`
- Conversation: one request followed by one identical response
- Payload: arbitrary non-empty bytes
- Frame: big-endian `u32` payload length followed by the payload
- Maximum payload: 1 KiB

The client rejects a response that differs from its request. Empty and
oversized frames are rejected before payload allocation or I/O respectively.

The crate owns no authentication, transport, runtime, timers, persistence, or
platform bindings. Its client and server functions accept any
`futures::AsyncRead + AsyncWrite` stream, allowing the same code to run over a
native authenticated `AukiPeer` stream or a browser stream adapted in Wasm.
