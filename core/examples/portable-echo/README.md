# Portable Echo

A request/response example for Rust, Python, Web, and Swift. Send 1–1,024 bytes
to another peer and receive the same bytes back.

| Run it in | Instructions |
| --- | --- |
| Rust | [Connect two peers](../../../docs/tutorials/first-peer.md) |
| Python | [Python app](python/README.md) |
| Web | [Browser app](web/README.md) |
| Swift | [iOS app](swift/README.md) |

The [message format](src/wire.rs) is shared by all four apps. The
[endpoint](src/endpoint.rs) registers the handler with `AukiPeer` and sets
five-second timeouts for opening, exchanging, and closing each stream.

For your own messages, follow
[Use a custom application protocol](../../../docs/how-to/protocols.md).
