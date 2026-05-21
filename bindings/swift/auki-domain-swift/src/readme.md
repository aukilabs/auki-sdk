# auki-domain-swift implementation

`src/lib.rs` hosts:

- `uniffi::setup_scaffolding!()` — the per-crate UniFFI metadata anchor.
- `uniffi::custom_type!(PeerId, String, { remote, ... })` and same for `Multiaddr` — the canonical string FFI seam for libp2p types.
- `pub use` re-exports of all upstream-annotated types from `auki-domain`, `auki-network`, `auki-time`, and `auki-network-swift` (for the binding-crate-side callback-interface traits + StreamSubscription* Swift glue).
- `BootstrapSwiftError` — typed error for the orchestrator pre-flight failures (seed length, swarm build, identity derivation).
- `build_swarm_and_identity` — internal helper that takes Swift-shaped inputs and returns `(PeerIdentity, Vec<Multiaddr>, Swarm<Behaviour>)`.
- `bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift` — three async orchestrators exposed via `#[uniffi::export(async_runtime = "tokio")]`. Each builds the swarm internally, optionally wraps a SwiftStreamProvider, and delegates to the upstream constructor.
- `list_clusters` — static-equivalent free function returning `Vec<ClusterEntry>`.

Upstream-side additions (in `crates/auki-domain` + `crates/auki-network` + `crates/auki-time`):

- `ClusterManager` (Object) + `ClusterTarget` (Enum) + provider traits as callback interfaces.
- ~30 annotated methods on `ClusterManager` — identity, membership, catalogs, registries, streams, clock sync, diagnostics.
- ~20 annotated value records (DaemonInfo, ClusterMember, ClusterMembership, ParticipantInfo, SensorEntry, registry entries + nested types, resource records + geometry, clock sync types, diagnostic types).
- 10+ annotated error enums (flat_error pattern for variants wrapping non-FFI inner errors).
