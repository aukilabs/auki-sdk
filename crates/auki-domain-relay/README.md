# auki-domain-relay

A small native libp2p Circuit Relay v2 server for Auki Domain reachability.

The crate owns only the relay server swarm. It is not a Domain Manager and does
not publish membership, elect a leader, call Discovery, or decide which peers
belong to a Domain. A host control plane selects a provider and constructs an
authorized `RelayProvider` assignment. A lower-level `auki-p2p::Node` reserves
it, then the host distributes only the confirmed complete circuit route.
Provider selection and booking HTTP remain external to both SDK crates.

## Public surface

- `DomainRelayConfig` configures native/WebSocket listen addresses and the
  libp2p agent version.
- `DomainRelay` owns the relay swarm and yields lifecycle events.
- `DomainRelayEvent::Listening` reports a bound address suffixed with
  `/p2p/<relay-peer-id>` so it can be distributed as a dialable relay route.

## Example

```sh
cargo run -p auki-domain-relay --example domain_relay -- \
  /ip4/0.0.0.0/tcp/4001 \
  /ip4/0.0.0.0/tcp/4002/ws
```

Run the hermetic relay listener test with:

```sh
cargo test -p auki-domain-relay
```

The relay uses the same canonical `auki-p2p::Identity` as authenticated Domain
participants, so a persisted key produces the same stable libp2p Peer ID.
