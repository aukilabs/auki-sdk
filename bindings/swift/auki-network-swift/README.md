# auki-network-swift

UniFFI Swift bindings for the prior v0 Manager-era networking surface.

> **Compatibility:** this package is excluded from the active workspace and is
> not wire-compatible with the authenticated Stage 1 Rust/Python Domain. Keep
> using its pinned prior SDK line as a complete group until the Swift Domain
> owner lands; do not mix it with Stage 1 peers.

It produces a `staticlib` for iOS plus a `cdylib` for host `uniffi-bindgen`
introspection on that historical line.

**Status:** Legacy/excluded (v0; authenticated `auki-domain-swift` is future work).

## Public surface

- **Discovery HTTP client** — `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`.
- **`NetworkRuntime`** + `spawn_for_swift` orchestrator — builds the libp2p swarm and wires it to Swift callback interfaces.
- **Peer-liveness observation** — `PeerLivenessListener` (3-variant v0 surface: `Connected` / `Disconnected` / `HeartbeatStreamClosed`).
- **Heartbeat source** — `HeartbeatTimestampProvider`.
- **5-payload stream surface** — `StreamSubscriptionAudio` / `…Camera` / `…PointCloud` / `…JointEncoders` / `…Detection` plus matching `NetworkRuntime.open_*_stream` async methods. Producer side via the `SwiftStreamProvider` callback interface with a two-call protocol (`dispatch_decision` → `SwiftStreamDecision`; on Accept, the runtime calls the matching `*_source` method).

The future authenticated Swift Domain owner replaces these Manager-era
lifecycle surfaces; it does not port `ClusterManager` forward.

## Depends on

- [`auki-network`](../../../crates/auki-network) — upstream crate the UniFFI annotations live in.
- [`auki-identity`](../../../crates/auki-identity) — transitively, for `PeerIdentity` derivation.
