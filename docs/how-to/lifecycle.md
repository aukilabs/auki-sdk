# Keep a Peer ID and stop cleanly

## Keep the same Peer ID across restarts

Use your authenticated [`bootstrap`](authenticate.md) and a `DomainSelection`
named `selection` to start a native peer with an identity file:

~~~rust
let peer = bootstrap
    .start_persistent_peer(selection, "./state/peer.identity")
    .await?;
~~~

The SDK loads or creates the key and its parent directory. Store the file
privately on persistent storage. Each running peer needs a different key.

Use `start_ephemeral_peer(selection)` when you want a new Peer ID on each start.
Browsers currently use this option. Python supports an identity file; Swift
lets your app save and restore encoded identity bytes.

## Handle connection failures

In native Rust, read `peer.status()` or watch `peer.subscribe_status()`.
Pause new network requests while authentication or the relay is unavailable.
The SDK attempts recovery and reports status changes. See the
[error reference](../reference/networking.md#errors-and-recovery) for when to
retry or sign in again.

Watch `peer.wait_stopped()` alongside your app's event loop so it can react
when the peer stops permanently. The [Echo handler](protocols.md#start-with-echo)
shows this with `tokio::select!`. Web exposes `peer.waitStopped()`.

## Close your handlers, then stop the peer

Stop sending new requests, close your handlers, and await peer shutdown.
For an app using the [Echo endpoint](connect.md):

~~~rust
use auki_portable_echo::EchoEndpoint;
use auki_sdk::AukiPeer;

async fn stop_peer(peer: AukiPeer, endpoint: EchoEndpoint) -> anyhow::Result<()> {
    let endpoint_cleanup = endpoint.close().await;
    let peer_cleanup = peer.shutdown().await;
    endpoint_cleanup?;
    peer_cleanup?;
    Ok(())
}
~~~

Both cleanup steps run even if the first fails. Likewise, call your cleanup
before returning an application error. Awaiting shutdown releases relay
bookings and closes connections; dropping the peer is not sufficient.

When logging out, stop all peers using the session, await
`bootstrap.session().close()`, then erase saved credentials.

To switch Domains, stop the old peer and start one in the new Domain.
