# Relay-Backed Libp2p Transport

## Decision

Replace Discovery-signaled WebRTC with relay-backed libp2p transport. Discovery
remains the domain and relay directory. Circuit Relay/WebRTC carries connection
establishment and Auki protocol streams.

The target shape is a browser-dialable manager address:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/webrtc/p2p/<manager-peer-id>
```

The Discovery service should publish relay catalog and cluster manager address
data only. It must not carry SDP or ICE payloads for this transport.

## Decision Gate

The relay-backed replacement is acceptable only if a generated browser peer can
dial a native iOS/Rust-backed peer through a public relay-backed multiaddr and
open `/auki/join/0.0.1` plus `/auki/stream/0.1.0`.

Task 1 of the migration plan owns this gate. The gate fails if any of these are
true:

- The Rust-backed native runtime cannot reserve a relay circuit on a
  browser-usable `/ws` or `/wss` relay.
- The Rust-backed native runtime cannot be dialed as a private `/webrtc` target
  behind that relay.
- The generated/browser-owned js-libp2p runtime cannot open the Auki protocol
  streams once the connection is established.

## Removal Rule

Remove `/auki-webrtc-signaling/.../p2p/...`, `AukiSignaledWebRTCPeer`, and the
Swift `AukiNetworkSignaledWebRTC` / `AukiDomainSignaledWebRTC` support targets
as part of the relay-backed migration. Do not keep them as fallback transports.

## Current Spike Status

Blocked at the Task 1 decision gate on May 28, 2026.

What passed:

- `cargo check -p auki-network --features swarm --example
  relay_native_target_smoke` compiles the native relay target smoke.
- `cargo run -p auki-network --features swarm --example
  relay_native_target_smoke` proves the current Rust swarm can start an
  in-process Rust relay, connect a native target to it, reserve a circuit, and
  produce a candidate target address.

What failed:

- The produced local address uses a Rust-only TCP relay path:

  ```text
  /ip4/127.0.0.1/tcp/<port>/p2p/<relay-peer-id>/p2p-circuit/webrtc/p2p/<target-peer-id>
  ```

- `node examples/relay-smoke/browser-smoke.mjs` rejects that address because the
  browser-visible relay path before `/p2p-circuit` is not `/ws` or `/wss`.

Current architecture blocker:

- `auki-network::swarm::build_swarm` currently builds TCP + QUIC + Circuit Relay
  client/server transport. It does not build a WebSocket/WSS relay transport, so
  the native runtime cannot yet use the required browser relay address:

  ```text
  /dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>
  ```

- The `libp2p-webrtc` crate currently used by `auki-network` is wired as
  `webrtc_direct` and supports `/webrtc-direct` UDP listeners. The migration
  target needs a private relay-signaled `/webrtc` target after `/p2p-circuit`.

Decision:

- Do not proceed to relay catalog, Discovery registration, or generated binding
  migration work until the native Rust runtime can satisfy the browser-usable
  relay/WebRTC target gate.
- Do not preserve or revive the Discovery-signaled Swift WebRTC backend as a
  fallback.
