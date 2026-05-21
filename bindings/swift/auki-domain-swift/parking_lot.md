# auki-domain-swift parking lot

Open questions for this crate. Resolved items are deleted (per auki-sdk convention) and recorded in `changelog.md`.

## Open

- **`BootstrapSwiftError` vs. typed `*Error` mapping.** Swarm-build and 32-byte seed length failures inside the bootstrap orchestrators are currently folded into existing variants of `BootstrapError` / `CreateClusterError` / `JoinClusterError` (e.g. `Rejected(message)` or `Discovery(InvalidPeerId)`). Cleaner would be adding new variants upstream: `*Error::SwarmBuild { message: String }` and `*Error::InvalidSeed { actual: u32 }`. v0 ships with the fold mapping; future expansion adds the upstream variants.
- **`StreamSubscription*` cross-crate UniFFI propagation.** PR B's auki-network-swift defines the 5 subscription Objects. Plan C re-exports them via `pub use`; verify the Swift surface includes them under the `AukiDomain` xcframework's `auki_network.swift` namespace file.
- **`open_stream` generic resolver.** Python has a single `open_stream(peer_id, sensor_id)` that fetches the resource catalog, finds the payload kind, and dispatches to the typed opener. Skipped in Plan C — Swift consumers do this dispatch in Swift code by calling `fetch_resources_catalog` then the matching typed opener.
- **Heartbeat-detail variants of `SwiftPeerLivenessEvent`.** Inherited from PR B's parking lot — `HeartbeatReceived` / `HeartbeatNtpSampleObserved` upstream variants are dropped at v0.
- **`TransformEdgeResource::source` type change.** If Task 6 chose path (a) — change upstream from `Option<serde_json::Value>` to `Option<String>` — verify nothing in `auki-domain` or downstream consumers relied on programmatic JSON-value manipulation of this field. Audit periodically.
- **Single shared tokio runtime.** Three binding crates each drive their own. Consolidate if profiling shows pain.
