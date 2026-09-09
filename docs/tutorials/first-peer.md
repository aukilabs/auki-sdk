# Connect two peers and exchange a message

Run two apps and send `hello from Auki` from one to the other. The receiving
app sends the same message back using the example's Echo protocol.

You need Rust 1.89 or newer, this repository checked out, two terminals, and an
Auki User with access to a Domain. Have its Domain UUID ready.

## Start the first peer

From the repository root in terminal A, set your credentials and Domain:

~~~sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='<your password>'
export AUKI_DOMAIN_ID='<your Domain UUID>'
export AUKI_IDENTITY_FILE='/tmp/auki-echo-a/peer.identity'

cargo run --locked -p auki-sdk --example generate_identity -- /tmp/auki-echo-a/peer.identity
cargo run --locked -p auki-portable-echo-native
~~~

The first command creates the identity file and its parent directory, or
reuses the saved identity if it already exists. It prints the Peer ID.

The app signs in, connects through a relay, and publishes its Echo address in
Auki's DDS discovery service. After startup you will see:

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

cargo run --locked -p auki-sdk --example generate_identity -- /tmp/auki-echo-b/peer.identity
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

Open the [application source](../../core/examples/portable-echo/native/src/main.rs)
to see how it signs in, starts `AukiPeer`, registers Echo, and sends a request.
Both apps use the same User account; the separate identity files give them
different Peer IDs.

For another language, use the same Echo implementation through its
[Python](../../core/examples/portable-echo/python/README.md),
[Web](../../core/examples/portable-echo/web/README.md), or
[Swift](../../core/examples/portable-echo/swift/README.md) app.

Continue with [your own protocol](../how-to/protocols.md), or consult
[connection troubleshooting](../reference/networking.md#errors-and-recovery)
if the exchange fails.
