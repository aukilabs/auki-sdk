# auki-domain-relay

Domain Relay capability for browser-reachable Auki Domains.

**Status:** Initial scaffold. The crate can start a libp2p Circuit Relay v2 server and emit Discovery-ready relay multiaddrs. Domain-scoped reservation grants and policy enforcement are still pending.

## Public Surface

- `DomainRelayConfig` — listen addresses and libp2p agent version.
- `DomainRelay` — owns the relay swarm and yields lifecycle events.
- `DomainRelayEvent::Listening` — emitted with a `/p2p/<relay-peer-id>` suffixed relay multiaddr suitable for Discovery's `relay_multiaddrs`.

## Example

```sh
cargo run -p auki-domain-relay --example domain_relay -- /ip4/0.0.0.0/tcp/4002/ws
```

## Depends On

- [`auki-network`](../auki-network) — for SDK peer identity.
