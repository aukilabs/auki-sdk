# Changelog — auki-domain-swift

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 22, HKT, 2026

Initial release. PR C of the Spec 1 SDK Swift binding expansion. Provides full Swift access to `auki-domain::ClusterManager` with parity to `auki-domain-py` plus the upstream-only methods explicitly included by user choice (clock sync, diagnostics). Architecture: upstream UniFFI annotations behind a new `swift-bindings` cargo feature on `auki-domain`; this binding crate is a thin scaffolding host with `PeerId` / `Multiaddr` custom-type registrations, `pub use` re-exports of upstream-annotated types, and three orchestrator functions (`bootstrap_swift`, `create_cluster_swift`, `join_cluster_swift`) that hide libp2p swarm construction from Swift callers. Inherits the 5-payload stream surface + Swift callback-interface traits from `auki-network-swift` (PR B) via cross-crate dep + `pub use`. Registry-entry trees (`SensorBody`, `ClockBody`, `FrameRegistryEntry`, `DetectorRegistryEntry` + nested types) annotated as typed UniFFI records/enums per user choice.
