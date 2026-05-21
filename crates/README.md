# Crates

Quick overview of every crate in the workspace. Each line links to the crate's outer `README.md` (the spec); per-crate implementation status lives in `<crate>/src/readme.md` and current work in `<crate>/src/sprint.md` per the [folder convention](../CONTRIBUTING.md).

For the SDK as a whole, start at the [root `README.md`](../README.md). Cross-crate open questions live in [`parking_lot.md`](parking_lot.md); the cross-crate changelog summary in [`changelog.md`](changelog.md).

## Cryptography & canonicalization

| Crate | What it does |
|---|---|
| [`auki-hash`](auki-hash) | XXH3-128 wrapper. The hash that becomes a registry entry's filename. |
| [`auki-jcs`](auki-jcs) | RFC 8785 JSON canonicalization. Stable bytes for hashing and signing. |
| [`auki-identity`](auki-identity) | Wallet primitive — ed25519 keypairs, deterministic child derivation, signed creation certs. WASM-friendly. |

## On-disk format

| Crate | What it does |
|---|---|
| [`auki-logs`](auki-logs) | Generic segmented ring-buffer log primitive (manifest + segments + retention eviction). Encoder-agnostic via the `LogPayload` trait. |
| [`auki-datatypes`](auki-datatypes) | Single source of truth for cross-language **segment payload** shapes — `.proto` schemas + prost-generated Rust. |
| [`auki-manifests`](auki-manifests) | Single source of truth for **manifest** shapes — JCS-canonical UTF-8 JSON. Symmetric with `auki-datatypes`. |
| [`auki-registry`](auki-registry) | Sensor + Clock + Frame registry types and IO. Identity for sensors, clocks, and coordinate frames. |

## Lifecycle & layout

| Crate | What it does |
|---|---|
| [`auki-layout`](auki-layout) | Path helpers for the on-disk session shape — single source of truth for app/session/recording layout. |
| [`auki-time`](auki-time) | Session-clock, pure time-transform math, NTP-style clock samples, and sampler primitives for the TimeTransform Log. The infrastructure underneath `convert_time`. |
| [`auki-geometry`](auki-geometry) | Pure spatial math: convention conversion for points, vectors, directions, and `SpatialTransform` poses. The convention-only layer underneath future `convert_pose`. |
| [`auki-domain`](auki-domain) | App-facing cluster lifecycle: Discovery bootstrap, membership, Manager election, liveness checks, peer info/resource catalog exchange, transform-edge discovery, stream opening. |

## Networking

| Crate | What it does |
|---|---|
| [`auki-network`](auki-network) | libp2p substrate (TCP/QUIC, Noise, Yamux, Circuit Relay v2), `NetworkRuntime`, cluster peer protocols including `/auki/resources/0.0.1`, typed `Stream<T>` over `/auki/stream/0.1.0`, Discovery HTTP client, and peer identity from `Wallet::derive_child("peer/v1")`. |

## Adapters

| Crate | What it does |
|---|---|
| [`auki-ros-adapter`](auki-ros-adapter) | Translation from ROS2 sensor messages into Auki SDK types. Lets a ROS2 node mint registry entries and write Sensor Logs without knowing the SDK's on-disk format. |

## Python bindings

Python packages live in [`../bindings/python`](../bindings/python), preserving the same per-component package names while keeping this directory focused on Rust crates.

## Swift bindings

UniFFI wrappers for native iOS (`aukilabs/iosapp`), one per Rust component, same per-component rule as the Python bindings (no umbrella `auki-swift`).

| Crate | What it does |
|---|---|
| [`auki-network-swift`](auki-network-swift) | Discovery HTTP client for Swift (`DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`), async via UniFFI/tokio. Stage 1; stream/audio types and a future `auki-domain-swift` (cluster/peer enumeration) are scoped in its `src/sprint.md`. |
