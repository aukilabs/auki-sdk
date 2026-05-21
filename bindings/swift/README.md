# Swift Bindings

Swift-facing SDK packages live here. The structure preserves the existing per-component package names: UniFFI wrappers sit alongside (eventually) generated SwiftPM artifacts, while Rust SDK implementation crates remain in [`../../crates`](../../crates) — same per-language rule the Python bindings follow.

| Package | What it does |
|---|---|
| [`auki-network-swift`](auki-network-swift) | UniFFI Swift bindings for `auki-network`'s Discovery HTTP client (Stage 1). Async `DiscoveryClient`, `ClusterEntry`/`CreateClusterOutcome`, flattened `DiscoveryError`. Stream/audio is Stage 2; cluster join + peer enumeration will arrive as a future `auki-domain-swift`. |
| [`auki-uniffi-test`](auki-uniffi-test) | Local Swift package for the `crates/auki-uniffi-test` UniFFI proving crate. Generated Swift, headers, and iOS/macOS XCFramework output live under `generated/`; the package root owns the static `Package.swift`. |
