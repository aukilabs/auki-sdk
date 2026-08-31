# Portable echo — native AukiPeer adapter

This thin host authenticates and starts a native `AukiPeer`, then mounts the
shared [`auki-portable-echo-adapter`](../adapter) implementation. The adapter
owns the exact protocol registration, five-second operation deadlines,
conversation, stream cleanup, and bounded inbound observations on every Rust
target. This executable supplies only credentials, peer configuration, console
output, and shutdown.

## Run two peers

Both processes need access to the same dev Domain. Use separate state
directories so they have distinct persistent Peer IDs.

Terminal A:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_STATE_DIR='/tmp/auki-portable-echo-a'
cargo run --locked -p auki-portable-echo-native
```

Copy the printed `PEER_ID` and complete `RELAY_ROUTE`, then start terminal B:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_STATE_DIR='/tmp/auki-portable-echo-b'
export AUKI_REMOTE_PEER_ID='<PEER_ID from terminal A>'
export AUKI_REMOTE_ROUTE='<RELAY_ROUTE from terminal A>'
export AUKI_ECHO_MESSAGE='hello from the shared Rust protocol'
cargo run --locked -p auki-portable-echo-native
```

Terminal B prints `ECHO_OK`, closes its stream, and shuts down. Stop terminal A
with Ctrl-C. Both processes await the facade's ordered shutdown.

## Round trip with the browser playground

Start a browser peer from the
[Peer Playground](../web/README.md), then copy its Peer ID and the `tcp` route
from its public Peer Card. Run the native peer with those values and keep it
serving after its first outbound echo:

```sh
export AUKI_REMOTE_PEER_ID='<browser Peer ID>'
export AUKI_REMOTE_ROUTE='<browser Peer Card tcp route>'
export AUKI_KEEP_RUNNING=1
cargo run --locked -p auki-portable-echo-native
```

The terminal first prints `ECHO_OK`, then `WAITING_FOR_PEER`. Paste its printed
`PEER_CARD` into the playground and send an echo back. The browser currently
dials through its own DMS-confirmed relay, so this manual v0.1 round trip
requires both peers to receive a slot on the same relay. Stop the terminal with
Ctrl-C when finished.

A trusted native application can use App credentials instead:

```sh
unset AUKI_EMAIL AUKI_PASSWORD
export AUKI_APP_ACCESS_KEY='...'
export AUKI_APP_SECRET='...'
```

Do not place App credentials in the future browser example.
