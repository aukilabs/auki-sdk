# Relay Smoke

This directory contains the browser-side half of the relay-backed libp2p
decision gate.

## Native Target

Start the Rust-backed native target first:

```bash
cargo run -p auki-network --features swarm --example relay_native_target_smoke
```

By default the native smoke creates an in-process Rust relay over TCP, reserves a
circuit, writes the expected browser target address to
`examples/relay-smoke/target-addr.txt`, and waits for an inbound browser peer.

For the real browser gate, pass a public browser-usable relay address instead:

```bash
AUKI_RELAY_ADDR=/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id> \
  cargo run -p auki-network --features swarm --example relay_native_target_smoke
```

The smoke succeeds only after the native target observes an inbound connection
from a non-relay peer.

## Browser Dialer

Install the smoke dependencies once:

```bash
npm install --prefix examples/relay-smoke
```

Then dial the address printed by the native target:

```bash
AUKI_RELAY_TARGET_ADDR=/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/webrtc/p2p/<target-peer-id> \
  node examples/relay-smoke/browser-smoke.mjs
```

If `AUKI_RELAY_TARGET_ADDR` is unset, the script reads
`examples/relay-smoke/target-addr.txt`.

The browser smoke intentionally rejects relay paths that are not browser-usable
`/ws` or `/wss` paths. A Rust-only `/tcp` relay proves Circuit Relay locally, but
does not satisfy the browser/mobile interop gate.
