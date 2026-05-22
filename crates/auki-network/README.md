# auki-network

Networking substrate for the Auki SDK.

This crate owns the low-level network pieces: wallet-derived libp2p peer identity, reachability records, named capabilities, the libp2p swarm builder, the `NetworkRuntime` task that drives the swarm, wire protocols for cluster membership/data exchange, and the Discovery HTTP client. Cluster lifecycle policy lives one layer up in [`auki-domain`](../auki-domain): app daemons should normally construct a [`ClusterManager`](../auki-domain/README.md), not wire these pieces together directly.

## Current Shape

The crate now follows the SDK multiplatform crate pattern:

- The binding-free Rust surface lives in `core.rs` and is re-exported from the crate root.
- The default native feature set enables `uniffi` so ordinary native builds exercise the native adapter.
- Rust consumers that want only the direct Rust API should depend on `auki-network` with `default-features = false`.
- Browser builds use `--no-default-features --features wasm`; the Rust wasm layer exposes identity and protocol helpers, while browser transport is implemented in the generated JavaScript package with jslibp2p.

The native UniFFI adapter is intentionally small. It does not replace the existing PyO3 `auki-network-py` runtime package, which still owns Python access to the richer async swarm/discovery surface.

The binding-free core surface is intentionally small and WASM-friendly:

- `PeerIdentity` derives a libp2p ed25519 peer key from `Wallet::derive_child("peer/v1")`, and can export libp2p private-key protobuf bytes so jslibp2p can construct the same PeerId in browser JavaScript.
- `ReachabilityRecord` is a serializable "how to dial me" shape: peer id, multiaddrs, capabilities, last-seen timestamp.
- `Capability` is an open namespaced string with four canonical `networking:*` constants.
- `ParticipantInfo` is the SDK-provided `/api/info` JSON shape: daemon identity, session clock binding, peer id, app instance, `is_manager`, and `manager_peer_id`.

The `swarm` feature adds native libp2p support:

- `swarm::build_swarm(identity, SwarmConfig)` builds TCP + QUIC + Noise + Yamux, always with identify, ping, relay-client, optional relay-server, and the raw-substream behaviour used by SDK protocols.
- `swarm::collect_routable_listen_addrs` and `resolve_advertise_multiaddrs` are the SDK path for deciding which listen addresses a daemon advertises to Discovery.
- `NetworkRuntime::spawn(swarm, allowed_peers, stream_provider, heartbeat_timestamps)` owns the swarm event loop and exposes small methods for updating peers, opening streams, sending join requests, requesting peer info/resource catalogs, broadcasting membership, and shutting down. The caller must provide the sender clock id/hash, monotonic timestamp callback, and optional domain-clock source metadata callback used in heartbeat frames.

The `discovery_client` feature adds `DiscoveryClient`, the HTTP client for [`aukilabs/discovery`](https://github.com/aukilabs/discovery):

| Method | Discovery endpoint | Purpose |
|---|---|---|
| `list_clusters()` | `GET /clusters` | Directory snapshot sorted newest-first |
| `create_cluster(name, manager_peer_id, manager_multiaddrs)` | `POST /clusters/{name}` | Atomic create; caller becomes initial Manager |
| `liveness_check(name, peer_count)` | `POST /clusters/{name}/liveness` | Manager push that resets Discovery's sweep window |
| `rotate_manager(name, manager_peer_id, manager_multiaddrs)` | `POST /clusters/{name}/manager` | Successor publishes the new Manager hint |
| `deregister(name)` | `DELETE /clusters/{name}` | Graceful removal when the last member exits |

Discovery v1 is deliberately shape-checked rather than signature-verified. Successor-token and challenge/response hardening are future work.

## Browser JavaScript

`just generate-javascript-bindings auki-network` builds the `wasm` feature with `wasm-pack`, renders the crate-owned templates in `bindings/javascript/`, and smoke-tests the generated package.

The generated package has two layers:

- wasm-bindgen exports for canonical peer derivation, libp2p private-key protobuf bytes, the browser-probe protocol id, and JSON request/response helpers;
- an `index.js` wrapper that lazily imports jslibp2p packages and creates a browser `Libp2p` node from the Rust-derived private key.

The browser package starts at the network layer. It is not a resurrected `auki-domain-browser` facade, and it does not implement browser Manager semantics. The first production target is a browser leaf peer dialing a native SDK peer through a browser-compatible multiaddr.

## Protocols

All cluster peer-to-peer protocols ride on the same libp2p swarm. The runtime keeps handshakes open enough for new peers to join; membership/trust enforcement happens inside the protocol handlers.

| Protocol | Module | Purpose |
|---|---|---|
| `/auki/join/0.0.1` | `join_protocol` | Non-member asks the current Manager to admit it; response carries membership JSON + successor token |
| `/auki/heartbeat/0.0.1` | `heartbeat_protocol` | Bidirectional heartbeat carrier frames with sender-clock timestamps, optional NTP echo fields, and optional domain-clock source metadata; cluster liveness semantics live in `auki-domain` |
| `/auki/membership/0.0.1` | `membership_protocol` | Manager gossips its peer id plus fresh membership JSON to members |
| `/auki/info/0.0.1` | `info_protocol` | Cluster peer asks another peer for its `ParticipantInfo` |
| `/auki/resources/0.0.1` | `resources_protocol` | Cluster peer asks another peer what resources it can provide now; v0 rows are `sensor_stream` (optionally with pinhole intrinsics) and `transform_edge` |
| `/auki/sensors/0.0.1` | `sensors_protocol` | Cluster peer asks another peer for its current sensor catalog, optionally embedding Sensor / Frame Registry JSON |
| `/auki/registries/0.0.1` | `registries_protocol` | Cluster peer fetches a hash-pinned Sensor / Clock / Frame Registry entry |
| `/auki/stream/0.1.0` | `stream_protocol` / `stream_runtime` | Typed live sensor streams |

The stream runtime is a typed API layered over the `/auki/stream/0.1.0` prost envelope. Producer callbacks are `StreamProvider = Arc<dyn Fn(PeerId, StreamRequest) -> StreamDispatch + Send + Sync>`, so producers can enforce per-requester policy. `StreamDispatch` currently supports `AcceptCamera`, `AcceptPointCloud`, `AcceptJointEncoders`, `AcceptAudio`, and `Decline`. Each substream is mono-`T`; the consumer calls `open_stream::<T>(peer_id, request)` with the payload type it expects. New payload types ship as coordinated SDK releases because adding a `StreamDispatch` variant is a public API change that consumers opt into.

## Trust Boundary

The connection layer is not the main trust boundary anymore. The swarm uses a block-list for evicting misbehaving peers, while routine membership checks live in the protocol handlers:

- `/auki/join/0.0.1` intentionally accepts first contact from non-members.
- `/auki/stream/0.1.0`, `/auki/info/0.0.1`, `/auki/resources/0.0.1`, `/auki/sensors/0.0.1`, `/auki/registries/0.0.1`, heartbeat, and membership paths are gated against the runtime's current allowed-peer set and silently drop outsiders where appropriate.
- Heartbeat carrier opening is steered by the domain layer via `set_heartbeat_targets`: the runtime opens `/auki/heartbeat/0.0.1` only to the explicit peers it is given and reports frame/closure events upward. `HeartbeatReceived` includes a raw `HeartbeatTimingObservation` with the received frame, local receive timestamp, and both clock identities. When a peer echo matches one of this runtime's recent outbound heartbeat sequences, `HeartbeatNtpSampleObserved` carries an `auki-time::NtpSample` for later transform handling. If the caller supplies `domain_clock` metadata, the runtime copies it into outbound heartbeat frames without validating, storing, or interpreting the cluster clock.
- `auki-domain::ClusterManager` owns the membership document, heartbeat topology, heartbeat timeout/loss decisions, election, Discovery liveness checks, and updates to the runtime's allowed-peer set.

## Not Here

- No `cluster.json` static-config loader.
- No public `ClusterRuntime` or `cluster.spawn` path.
- No mDNS-based cluster discovery.
- No Rust wasm `NetworkRuntime`; browsers use jslibp2p through the generated JavaScript package.
- No browser Manager/create-Domain policy.
- No `convert_time` or `convert_pose`; this crate only transports the data and metadata those operations need.

See [`src/readme.md`](src/readme.md) for the implementation map and current public surface.
