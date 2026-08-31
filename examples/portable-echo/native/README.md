# Portable echo — native AukiPeer adapter

This example mounts the shared
[`auki-portable-echo-protocol`](../protocol/README.md) implementation on a real
native `AukiPeer`. It adds authentication, relay-backed reachability, operation
timeouts, and peer lifecycle without reimplementing the echo wire format.

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

A trusted native application can use App credentials instead:

```sh
unset AUKI_EMAIL AUKI_PASSWORD
export AUKI_APP_ACCESS_KEY='...'
export AUKI_APP_SECRET='...'
```

Do not place App credentials in the future browser example.
