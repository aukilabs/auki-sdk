# auki-domain-relay

Domain Relay capability for browser-reachable Auki Domains.

**Status:** Initial scaffold. The crate can start a libp2p Circuit Relay v2 server on native TCP and WebSocket addresses, then emit `/p2p/<relay-peer-id>`-suffixed relay multiaddrs for Manager reservation and browser Discovery hints. Domain-scoped reservation grants and policy enforcement are still pending.

## Public Surface

- `DomainRelayConfig` — native/browser listen addresses and libp2p agent version.
- `DomainRelay` — owns the relay swarm and yields lifecycle events.
- `DomainRelayEvent::Listening` — emitted with a `/p2p/<relay-peer-id>` suffixed relay multiaddr. Native `/tcp` addresses are suitable as Manager reservation dial addresses; `/ws` addresses are suitable for Discovery's browser-facing `relay_multiaddrs`.

## Example

```sh
cargo run -p auki-domain-relay --example domain_relay -- \
  /ip4/0.0.0.0/tcp/4001 \
  /ip4/0.0.0.0/tcp/4002/ws
```

## Smoke Test

The ignored live-Discovery smoke verifies that a Manager reserves through a native `DomainRelay` address, publishes the browser `/ws` circuit Manager address through `ClusterManager::create_cluster_with_relay_reservation(...)`, and reads both the circuit Manager address and relay hint back from Discovery:

```sh
DISCOVERY_URL=http://127.0.0.1:8080 \
  cargo test -p auki-domain-relay \
    relay_multiaddr_can_be_published_through_cluster_manager_to_discovery \
    -- --ignored --nocapture
```

## Depends On

- [`auki-network`](../auki-network) — for SDK peer identity.
