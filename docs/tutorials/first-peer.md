# Connect two peers and exchange a message

Run two applications in the same Domain and send an echo between them. This
uses the small, application-owned Portable Echo protocol.

You need Rust 1.89 or newer, this repository checked out, two terminals, and an
Auki User with access to a development Domain. Have its Domain UUID ready. The
example uses the shared development services and needs network access.

## Start the first peer

From the repository root in terminal A, set your credentials and Domain:

~~~sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='<your password>'
export AUKI_DOMAIN_ID='<your development Domain UUID>'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-a/peer.identity'

cargo run --locked -p auki-portable-echo-native
~~~

The app authenticates, starts a relay-backed peer, mounts Echo, and advertises
it through DDS discovery. After startup you will see:

~~~text
peer: 12D3KooW...
route: ...
~~~

Copy the complete Peer ID. Leave terminal A running.

## Send a message

In terminal B, from the same repository root, set the same credentials and
Domain. Use a different identity file:

~~~sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='<your password>'
export AUKI_DOMAIN_ID='<your development Domain UUID>'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-b/peer.identity'

cargo run --locked -p auki-portable-echo-native -- \
  --discover '<Peer ID from terminal A>'
~~~

Terminal B finds that peer, verifies its identity and Domain, sends a message,
and prints:

~~~text
echo: hello from Auki
~~~

It then shuts down. Run the command again: the saved identity keeps terminal
B's Peer ID the same.

## Stop and inspect the app

Press Ctrl-C in terminal A. The app closes its Echo endpoint and awaits peer
shutdown.

Open the [application source](../../core/examples/portable-echo/native/src/main.rs).
The flow is: authenticate, select a Domain, start the peer, mount an endpoint,
send or serve, and close both owners. The two identity files let the applications
share a User account while remaining distinct peers.

For another language, use the same Echo implementation through its
[Python](../../core/examples/portable-echo/python/README.md),
[Web](../../core/examples/portable-echo/web/README.md), or
[Swift](../../core/examples/portable-echo/swift/README.md) host.

Continue with [your own protocol](../how-to/protocols.md), or consult
[connection troubleshooting](../reference/networking.md#errors-and-recovery)
if the exchange fails.
