# auki-domain-relay

A small native libp2p Circuit Relay v2 server for Auki Domain reachability.

The crate owns only the relay server swarm. It is not a Domain Manager and does
not publish membership, elect a leader, call Discovery, or decide which peers
belong to a Domain. Hosts distribute relay routes and credentials through their
own control plane; authenticated clients book and use those routes through
`auki-p2p`.

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
