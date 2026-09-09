# Portable Echo

A small request/response protocol shared by Rust, Python, Web, and Swift apps.
It sends a bounded payload and receives the same bytes back.

| Run it in | Instructions |
| --- | --- |
| Rust | [Connect two peers](../../../docs/tutorials/first-peer.md) |
| Python | [Python host](python/README.md) |
| Web | [Browser host](web/README.md) |
| Swift | [iOS host](swift/README.md) |

The [wire module](src/wire.rs) defines `/example/echo/1.0.0` and its 1,024-byte
payload bound. The [endpoint](src/endpoint.rs) registers it with `AukiPeer` and
handles deadlines and cleanup.

For your own messages, follow
[Use a custom application protocol](../../../docs/how-to/protocols.md).
