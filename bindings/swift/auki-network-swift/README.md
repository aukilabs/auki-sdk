# auki-network-swift

UniFFI Swift bindings for [`auki-network`](../../../crates/auki-network) — the v0 networking surface for native iOS / Swift consumers. Produces a `staticlib` for iOS plus a `cdylib` for host `uniffi-bindgen` introspection.

**Status:** Shipped (v0; cluster lifecycle is `auki-domain-swift` future work).

## Public surface

- **Discovery HTTP client** — `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`.
- **`NetworkRuntime`** + `spawn_for_swift` orchestrator — builds the libp2p swarm and wires it to Swift callback interfaces.
- **Peer-liveness observation** — `PeerLivenessListener` (3-variant v0 surface: `Connected` / `Disconnected` / `HeartbeatStreamClosed`).
- **Heartbeat source** — `HeartbeatTimestampProvider`.
- **5-payload stream surface** — `StreamSubscriptionAudio` / `…Camera` / `…PointCloud` / `…JointEncoders` / `…Detection` plus matching `NetworkRuntime.open_*_stream` async methods. Producer side via the `SwiftStreamProvider` callback interface with a two-call protocol (`dispatch_decision` → `SwiftStreamDecision`; on Accept, the runtime calls the matching `*_source` method).

Cluster lifecycle (`ClusterManager` etc.) is the future `auki-domain-swift`, mirroring the Python `auki-network` / `auki-domain` split.

## Depends on

- [`auki-network`](../../../crates/auki-network) — upstream crate the UniFFI annotations live in.
- [`auki-identity`](../../../crates/auki-identity) — transitively, for `PeerIdentity` derivation.
