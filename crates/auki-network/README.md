# auki-network

The libp2p substrate for the SDK and the Discovery HTTP client. Behind the `swarm` feature: TCP/QUIC transport, Noise, Yamux, Circuit Relay v2, identify, ping, and a native helper for reserving a relay-mediated Manager address through a Domain Relay. On top: the typed `/auki/stream/0.2.0` stream protocol and the peer-to-peer control protocols (join, heartbeat, membership, info, resources, registries). The Discovery client carries both Manager multiaddrs and optional Relay multiaddrs for browser-compatible reachability hints.

Peer identity is derived from a wallet: `Wallet::derive_child("peer/v1")`. The `app_instance` value is MAC-derived per machine.

**Status:** Shipped.

## Public surface

- Types: `PeerIdentity`, `ParticipantInfo`, `ReachabilityRecord`, `Capability`
- Modules: `swarm`, `network_runtime`, `join_protocol`, `heartbeat_protocol`, `membership_protocol`, `info_protocol`, `resources_protocol`, `stream_protocol`, `stream_runtime`, `app_instance`, `discovery_client`
- `SessionHandle` trait — implemented by `auki_session::Session`; called by the resources protocol handler to serve catalog rows to remote peers. Defined here to break the potential `auki-domain` ↔ `auki-session` cycle.
- Resource catalog protocol `/auki/resources/0.2.0`: `ResourceEntry` rows discriminated by `variant` (`sensor_log` | `pose_log` | `time_transform_log` | `detection_log`). Each row has `source_peer_id`, `writer_peer_id`, `resource_id`, `state`, `head` | `extent`, `available`, optional `sensor` block (only on `sensor_log`: `kind`/`type`/`sensor_id`/`sensor_hash`), optional `pose` block (only on `pose_log`: `writer_mode`), and a variant-specific `manifest` pointer. Replaces the old `SensorStreamResource` / `TransformEdgeResource` / `PoseStreamResource` types.
- `/auki/resources/0.2.0` is a live, pollable snapshot of currently requestable resources. Peers may join before resources are ready; producers omit resources that cannot currently accept stream opens; consumers poll and reconcile row additions/removals. `resource_id` values stay stable across temporary outages. The current schema has no unavailable/degraded state, so absence from the catalog is the unavailability signal.
- Stream request type `StreamRequest { source_peer_id, resource_id, read_from }` where `read_from` is a `oneof` (`latest`, `from_start`, `from_timestamp(i64)`).
- Typed stream payloads: camera, point cloud, joint encoders, audio, and pose `SpatialTransform`
- Relay reachability: `swarm::reserve_relay_circuit_addr` reserves `/p2p-circuit` on a relay and returns the Manager circuit address to publish; `discovery_client` derives `relay_multiaddrs` from circuit Manager addresses when creating or rotating Discovery entries.
- Runtime relay reachability: `NetworkRuntimeHandle::reserve_relay_circuit_addr` lets a runtime-owned swarm reserve a Manager circuit address after spawn, for promotion/rotation flows.
- Discovery: `ClusterEntry.relay_multiaddrs`, `create_cluster_with_relay_multiaddrs`, `rotate_manager_with_relay_multiaddrs`
- Constants: `PEER_DERIVATION_LABEL = "peer/v1"`
- Locked vectors pin `seed → PeerId` and the stream wire bytes across language reimplementations.

## Depends on

- [`auki-identity`](../auki-identity) — for `Wallet` and child derivation.
- [`auki-datatypes`](../auki-datatypes) (optional) — for stream payload types.
- [`auki-time`](../auki-time) (optional) — for clock-stamped peer messages.
- [`auki-jcs`](../auki-jcs) (optional) — for canonical-JSON peer protocol bodies.
