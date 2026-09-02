# Portable echo in Python

This is the smallest complete Python application built on the native Rust
`AukiPeer` runtime and the shared portable echo protocol. Python owns only the
application flow. Rust owns authentication, Domain authorization, persistent
identity, relay booking, libp2p, protocol framing, deadlines, and cleanup.

The extension exposes `AukiSession`, `AukiPeer`, and `AukiEcho` from one
`auki_portable_echo` module. `AukiEcho` mounts the exact same Rust
`EchoEndpoint` used by the native and Web examples; there is no Python copy of
the protocol or wire format.

## Build the extension

Python 3.8 or newer and Rust 1.89.0 or newer are required. From this directory:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0' pytest
maturin develop --locked
pytest
```

`maturin develop` installs the local native extension into the active virtual
environment. Re-run it after changing Rust code.

## Run two discoverable Python peers

Both peers need User access to the same dev Domain and separate identity files.
The identity file is created on first use and preserves the Peer ID between
runs.

In terminal A:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-python-echo-a/peer.identity'
python main.py
```

The application reads one atomic route snapshot, prints its Peer ID and routes,
refreshes exact Echo advertisements, then serves:

```text
peer: 12D3KooW...
route: /dns4/relay.dev.aukiverse.com/tcp/443/p2p/.../p2p-circuit/p2p/12D3KooW...
wss route: /dns4/relay.dev.aukiverse.com/tcp/4443/wss/p2p/.../p2p-circuit/p2p/12D3KooW...
discovered Echo peers (untrusted until exact dial):
serving; use --discover PEER_ID from another terminal or press Ctrl-C to stop
```

In terminal B, use another identity file and select A by its discovered Peer
ID. No route is pasted:

```sh
export AUKI_EMAIL='you@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
export AUKI_IDENTITY_FILE='/tmp/auki-python-echo-b/peer.identity'
python main.py \
    --discover '<PEER_ID printed by terminal A>'
```

Terminal B polls the exact Echo protocol in DDS until A's mounted endpoint is
visible, tries A's advertised native routes, sends `hello from Auki`, closes the
echo endpoint, and shuts down its peer. Stop terminal A with Ctrl-C. It performs
the same ordered cleanup: protocol endpoint first, then the peer and relay
booking.

The example defaults to `discover_and_advertise`. Set
`AUKI_DISCOVERY_MODE=discover_only` to discover while remaining hidden. The
positional `PEER_ID EXACT_ROUTE` form remains a clearly labeled manual fallback
for debugging.

The example deliberately shows User login only. Trusted native applications
can use `await AukiSession.login_app_dev(access_key, secret)` instead. Never put
an App secret in browser code or another distributed client.

Discovery stays in this small Python host rather than the portable protocol
crate. Candidates remain untrusted hints: the Rust exact-route operation still
authenticates the selected Peer ID and Domain before any Echo payload flows.
