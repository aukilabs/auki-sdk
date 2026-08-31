# Portable echo — shared AukiPeer adapter

This crate mounts the transport-neutral
[`auki-portable-echo-protocol`](../protocol/README.md) on the canonical
cross-target `AukiPeer` protocol surface.

It owns the protocol registration, five-second stream-operation deadlines,
the SDK registration shutdown barrier, exact-route and configured-route
clients, stream cleanup, and a bounded nonblocking queue of inbound results. A
slow event consumer receives an explicit `Lagged` event; it never stalls a
protocol handler or prevents buffered events from making progress.

The native and Web hosts only need to:

1. authenticate and start an `AukiPeer`;
2. call `EchoEndpoint::mount(peer.protocols())`;
3. call `send_exact` with an advertised peer route; and
4. close the endpoint before the peer.

The crate contains no Tokio, wasm-bindgen, browser API, credential, or UI
policy. It compiles unchanged for native and `wasm32-unknown-unknown`.
