# Preserve identity and manage peer lifetime

## Choose a Peer ID lifetime

Native Rust can preserve an identity across restarts:

~~~rust
let peer = bootstrap
    .start_persistent_peer(selection, "./state/peer.identity")
    .await?;
~~~

The SDK loads the key or creates it and its parent directory. Keep the file in
persistent private storage. Give each simultaneously running peer a different
identity file; one live runtime owns a Peer ID.

Use `start_ephemeral_peer(selection)` for a fresh in-memory identity.
Browser peers currently use ephemeral identities. Python supports a persistent
identity file. Swift exposes encoded identity bytes for app-managed persistence.

## React to readiness and failure

Native Rust exposes `peer.status()` and `peer.subscribe_status()`. Accept new
application work while the peer is ready. Temporary authority or relay
unavailability may recover; a terminal failure needs application handling.

Race your application loop against `peer.wait_stopped()`, or use a cloned
`peer.lifecycle()` observer. A retained observer can outlive the peer owner.
Web exposes `peer.waitStopped()` for terminal failures.

Use the [error reference](../reference/networking.md#errors-and-recovery) to
decide whether to sign in again, fix configuration, or retry. The SDK owns
authority renewal and relay recovery during a peer's lifetime.

## Shut down in ownership order

Stop new application work, close your endpoints, then await peer shutdown.
Attempt cleanup even if application work failed:

~~~rust
let operation = run_application(&peer, &endpoint).await;
let endpoint_cleanup = endpoint.close().await;
let peer_cleanup = peer.shutdown().await;

operation?;
endpoint_cleanup?;
peer_cleanup?;
~~~

Here `run_application` is your app's work loop. Handle Ctrl-C, UI teardown, or
host cancellation there. Await shutdown to release relay bookings and finish
network cleanup; dropping handles alone is not a completed shutdown.

If logging out, stop all peers using the session, await
`bootstrap.session().close()`, and then erase saved credentials. Session close
and peer shutdown are separate operations.

To move an application to another Domain, shut down the old peer and start one
with the new Domain selection.
