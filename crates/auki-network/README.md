# auki-network

Networking substrate for the Auki SDK.

This crate owns the low-level network pieces: wallet-derived libp2p peer identity, reachability records, named capabilities, the libp2p swarm builder, the `NetworkRuntime` task that drives the swarm, wire protocols for cluster membership/data exchange, and the Discovery HTTP client. Cluster lifecycle policy lives one layer up in [`auki-domain`](../auki-domain): app daemons should normally construct a [`ClusterManager`](../auki-domain/README.md), not wire these pieces together directly.

## Current Shape

The default feature set is intentionally small and WASM-friendly:

- `PeerIdentity` derives a libp2p ed25519 peer key from `Wallet::derive_child("peer/v1")`.
- `ReachabilityRecord` is a serializable "how to dial me" shape: peer id, multiaddrs, capabilities, last-seen timestamp.
- `Capability` is an open namespaced string with four canonical `networking:*` constants.
- `ParticipantInfo` is the SDK-provided `/api/info` JSON shape: daemon identity, session clock binding, peer id, app instance, `is_manager`, and `manager_peer_id`.

The `swarm` feature adds native libp2p support:

- `swarm::build_swarm(identity, SwarmConfig)` builds TCP + QUIC + Noise + Yamux, always with identify, ping, relay-client, optional relay-server, and the raw-substream behaviour used by SDK protocols.
- `swarm::collect_routable_listen_addrs` and `resolve_advertise_multiaddrs` are the SDK path for deciding which listen addresses a daemon advertises to Discovery.
- `NetworkRuntime::spawn(swarm, allowed_peers, stream_provider)` owns the swarm event loop and exposes small methods for updating peers, opening streams, sending join requests, requesting peer info/catalogs, broadcasting membership, and shutting down.

The `discovery_client` feature adds `DiscoveryClient`, the HTTP client for [`aukilabs/discovery`](https://github.com/aukilabs/discovery):

| Method | Discovery endpoint | Purpose |
|---|---|---|
| `list_clusters()` | `GET /clusters` | Directory snapshot sorted newest-first |
| `create_cluster(name, manager_peer_id, manager_multiaddrs)` | `POST /clusters/{name}` | Atomic create; caller becomes initial Manager |
| `liveness_check(name, peer_count)` | `POST /clusters/{name}/liveness` | Manager push that resets Discovery's sweep window |
| `rotate_manager(name, manager_peer_id, manager_multiaddrs)` | `POST /clusters/{name}/manager` | Successor publishes the new Manager hint |
| `deregister(name)` | `DELETE /clusters/{name}` | Graceful removal when the last member exits |

Discovery v1 is deliberately shape-checked rather than signature-verified. Successor-token and challenge/response hardening are future work.

## Protocols

All cluster peer-to-peer protocols ride on the same libp2p swarm. The runtime keeps handshakes open enough for new peers to join; membership/trust enforcement happens inside the protocol handlers.

| Protocol | Module | Purpose |
|---|---|---|
| `/auki/join/0.0.1` | `join_protocol` | Non-member asks the current Manager to admit it; response carries membership JSON + successor token |
| `/auki/heartbeat/0.0.1` | `heartbeat_protocol` | Bidirectional heartbeat carrier frames; cluster liveness semantics live in `auki-domain` |
| `/auki/membership/0.0.1` | `membership_protocol` | Manager gossips fresh membership JSON to members |
| `/auki/info/0.0.1` | `info_protocol` | Cluster peer asks another peer for its `ParticipantInfo` |
| `/auki/sensors/0.0.1` | `sensors_protocol` | Cluster peer asks another peer for its current sensor catalog, optionally embedding Sensor / Frame Registry JSON |
| `/auki/registries/0.0.1` | `registries_protocol` | Cluster peer fetches a hash-pinned Sensor / Clock / Frame Registry entry |
| `/auki/stream/0.1.0` | `stream_protocol` / `stream_runtime` | Typed live sensor streams |

The stream runtime is a typed API layered over the `/auki/stream/0.1.0` prost envelope. Producer callbacks are `StreamProvider = Arc<dyn Fn(PeerId, StreamRequest) -> StreamDispatch + Send + Sync>`, so producers can enforce per-requester policy. `StreamDispatch` currently supports `AcceptJpeg`, `AcceptPointCloud`, `AcceptJointEncoders`, `AcceptAudio`, and `Decline`. Each substream is mono-`T`; the consumer calls `open_stream::<T>(peer_id, request)` with the payload type it expects.

## Trust Boundary

The connection layer is not the main trust boundary anymore. The swarm uses a block-list for evicting misbehaving peers, while routine membership checks live in the protocol handlers:

- `/auki/join/0.0.1` intentionally accepts first contact from non-members.
- `/auki/stream/0.1.0`, `/auki/info/0.0.1`, `/auki/sensors/0.0.1`, `/auki/registries/0.0.1`, heartbeat, and membership paths are gated against the runtime's current allowed-peer set and silently drop outsiders where appropriate.
- Heartbeat carrier opening is steered by the domain layer via `set_heartbeat_targets`: the runtime opens `/auki/heartbeat/0.0.1` only to the explicit peers it is given and reports frame/closure events upward.
- `auki-domain::ClusterManager` owns the membership document, heartbeat topology, heartbeat timeout/loss decisions, election, Discovery liveness checks, and updates to the runtime's allowed-peer set.

## Not Here

- No `cluster.json` static-config loader.
- No public `ClusterRuntime` or `cluster.spawn` path.
- No mDNS-based cluster discovery.
- No `convert_time` or `convert_pose`; this crate only transports the data and metadata those operations need.

See [`src/readme.md`](src/readme.md) for the implementation map and current public surface.
