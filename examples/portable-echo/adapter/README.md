# Portable echo — shared AukiPeer adapter

This crate mounts the transport-neutral
[`auki-portable-echo-protocol`](../protocol/README.md) on the canonical
cross-target `AukiPeer` protocol surface.

For an application, the whole protocol-specific lifecycle is:

```rust
let echo = EchoEndpoint::mount(peer.protocols())?;
let receipt = echo.send_exact(remote_peer_id, remote_route, payload).await?;
echo.close().await?;
```

The reusable adapter behind those calls owns the exact protocol registration,
five-second open/exchange/close deadlines, stream cleanup, an exact-route
cross-target client, a configured-route native client, and a bounded
nonblocking queue of inbound results. A slow event consumer receives an
explicit `Lagged` event; it never stalls a handler.

The adapter is protocol-author code written once, not application glue and not
generic SDK transport code. It contains no Tokio, wasm-bindgen, browser API,
credential, or UI policy and compiles unchanged for native and
`wasm32-unknown-unknown`.

The application still authenticates and starts the peer, obtains remote peer
information, selects which protocols to mount, applies product authorization
policy, and closes the endpoint before the peer. The SDK owns authority, relay,
route authentication, protocol hosting, fencing, and peer shutdown.

See [Author a portable Auki protocol](../../../docs/p2p/authoring-protocols.md)
for the complete ownership and release workflow.
