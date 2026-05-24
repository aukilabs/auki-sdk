# auki-network

The libp2p substrate for the SDK and the Discovery HTTP client. Behind the `swarm` feature: TCP/QUIC transport, Noise, Yamux, Circuit Relay v2, identify, ping. On top: the typed `/auki/stream/0.1.0` stream protocol and the peer-to-peer control protocols (join, heartbeat, membership, info, resources, sensors, registries). The Discovery client carries both Manager multiaddrs and optional Relay multiaddrs for browser-compatible reachability hints.

Peer identity is derived from a wallet: `Wallet::derive_child("peer/v1")`. The `app_instance` value is MAC-derived per machine.

**Status:** Shipped.

## Public surface

- Types: `PeerIdentity`, `ParticipantInfo`, `ReachabilityRecord`, `Capability`
- Modules: `swarm`, `network_runtime`, `join_protocol`, `heartbeat_protocol`, `membership_protocol`, `info_protocol`, `resources_protocol`, `sensors_protocol`, `stream_protocol`, `stream_runtime`, `app_instance`, `discovery_client`
- Discovery: `ClusterEntry.relay_multiaddrs`, `create_cluster_with_relay_multiaddrs`, `rotate_manager_with_relay_multiaddrs`
- Constants: `PEER_DERIVATION_LABEL = "peer/v1"`
- Locked vectors pin `seed → PeerId` and the stream wire bytes across language reimplementations.

## Depends on

- [`auki-identity`](../auki-identity) — for `Wallet` and child derivation.
- [`auki-datatypes`](../auki-datatypes) (optional) — for stream payload types.
- [`auki-time`](../auki-time) (optional) — for clock-stamped peer messages.
- [`auki-jcs`](../auki-jcs) (optional) — for canonical-JSON peer protocol bodies.
