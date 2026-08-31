# Auki SDK

On-device SDK for the Auki spatial-computing protocol — a Cargo workspace of
Rust crates plus Python (PyO3), Swift (UniFFI), and Web/Wasm source bindings.

See [VISION.md](VISION.md) for the aspirational spec; this file describes
what's actually in the repo today. [GLOSSARY.md](GLOSSARY.md) defines the domain
terms. New P2P applications and Manager-era migrations should start with the
[Auki P2P guide](docs/p2p/README.md); the
[Manager migration guide](docs/authenticated-domain-migration.md) also records
the retained low-level compatibility path.

## Stage 1 release line

The authenticated peer-runtime source candidate is the coordinated `0.1.0`
line. The active Cargo workspace declares an MSRV of Rust `1.89.0`, inherited
by every workspace package and checked at that exact toolchain before release.

| Artifact | Stage 1 version/distribution |
|---|---|
| SDK source | coordinated Git tag `v0.1.0` after the release gate |
| Rust `auki-sdk`, `auki-auth`, `auki-p2p`, and `auki-protocols` | source crate version `0.1.0`; one coordinated Git revision, with `auki_sdk::AukiPeer` as the canonical peer facade |
| Web/Wasm `auki-sdk-web` and portable protocols | source version `0.1.0`; working User-authenticated `AukiPeer` source, intentionally `publish = false` |
| Rust `auki-domain` | source crate version `0.1.0`; retained as a legacy/low-level compatibility surface, not the new peer facade |
| Python Domain/Session | source packages at `0.1.0`; retained compatibility and transport-neutral session surfaces |
| Python `AukiPeer` facade | pending |
| Swift `AukiPeer` facade | pending; the current Swift source exposes identity only |

Posemesh canonically owns and versions its `auki-p2p-dataset` application
protocol. That crate depends on an exact SDK `auki-p2p` revision or release; it
is not part of this SDK workspace or release line.

No `v0.1.0` tag, registry crate, Python wheel, Web package, or Swift peer facade
is published by this source checkout. Publishing is a separate release action
after the local gate. Removed Manager-era platforms remain retrievable at
`v0.0.60`, but are not wire-compatible with this source line.

## What this SDK does today

The Auki protocol is built around five questions any node should be able to answer about any other node — **Identity (who am I?)**, **Spatial (where did this happen?)**, **Temporal (when did this happen?)**, **Networking (how do I talk to you?)**, and **Tokenomics (how do I compensate you?)**. The SDK today implements the foundations for each.

### Identity

- **Stable P2P identity** — native hosts use
  `auki_p2p::Identity::load_or_create` for canonical, race-safe persistent
  Ed25519 identity. Products that intentionally bind Peer ID to a wallet may
  instead construct it from `Wallet::derive_child("peer/v1")`; the wallet also
  signs creation certs.
- **`auki-jcs` + `auki-hash`** — RFC 8785 JSON canonicalization + XXH3-128 content-addressing. The hash IS the version; refining an entry is a sibling-write under the same id.
- **Sensor / Clock / Frame Registries** — content-addressed catalogs for every entity referenced by the logs. Logs pin their `sensor_hash` / `clock_hash` / `frame_hash`; spatial sensors pin an exact `frame_id` + `frame_hash`.

### Spatial

- **Pose Logs** — segmented `from → to` transforms keyed per ordered frame pair. The future `convert_pose` composition along a transform path is pending.
- **Sensor + Detection Logs** — per-frame sensor payloads (camera, point cloud, joint encoders, audio) and detector outputs.
- **Maps + Mappers** — content-addressed Map resources, durable Map Logs, and SDK-native producers that transform sensor/pose streams into mergeable Map Updates.
- **Application-controlled components** — bring-your-own Detectors and Mappers run only when an application starts an instance. Live runners isolate blocking work behind bounded latest-wins queues; local Sensor Log detector replay remains ordered and exhaustive. Detection Logs and Map Logs, rather than component instances, are the Resources peers consume.
- **`auki-geometry`** — convention conversion for points, vectors, directions, and `SpatialTransform` poses (the convention-only layer underneath the future full `convert_pose`).

### Temporal

- **TimeTransform Logs** — segmented, application-produced offsets between two
  explicitly identified clocks. They preserve recorded temporal lineage
  without electing a Domain clock or deriving time authority from networking.
- **`auki-time`** — `SessionClock` and pure affine `TimeTransform` math. The
  `convert_time` operation that consumes recorded transforms is pending.

### Networking

- **`auki_sdk::AukiPeer` is the canonical Rust peer facade.** It composes one
  identity and one exact DDS Domain authority with the authenticated transport,
  credential renewal, DMS relay allocation, local routes, explicit application
  protocols, peer observations, lifecycle status, and ordered shutdown.
- **Authentication is platform-appropriate.** Trusted native Rust applications
  can prepare a peer from User email/password or App access-key/secret
  credentials. Web/Wasm exposes User authentication only; App secrets must not
  ship to a browser. Machine-managed Robot and Compute integrations can supply
  externally refreshed authority while `AukiPeer` continues to own networking.
- **Identity lifetime is explicit.** Native applications normally persist an
  Ed25519 identity with `Identity::load_or_create` and run only one live peer for
  that Peer ID. The current Web facade intentionally generates a fresh,
  session-ephemeral identity whenever it starts a peer.
- **Relay-backed reachability is the default.** Native peers receive a confirmed
  DMS relay route unless they explicitly choose direct-only operation; browser
  peers always receive one relay. Applications dial an exact route compatible
  with their target transport—for example TCP from native Rust and WSS from a
  browser—and the authenticated stream verifies the expected Peer ID and Domain.
- **Direct-only may be outbound-only.** A native `direct_only()` peer can start
  with no listener or local route and dial other peers. Inbound direct
  connections require a listener plus a dialable route shared by the
  application. Configure an advertised direct route only when the application
  publishes it from the SDK's local route catalog.
- **Discovery and route distribution are not implemented yet.** The SDK exposes
  confirmed local routes, but it does not automatically publish them to other
  peers or discover remote peers. Applications currently exchange the remote
  Peer ID and complete compatible route through configuration, a peer card, or
  their own control plane.
- **Application protocols are explicit and portable.** `AukiPeer` serves no
  product protocol until the application registers an exact version through
  `peer.protocols()`. One product-owned Rust crate can keep its wire contract
  and `AukiPeer` endpoint in private modules shared by native and Web/Wasm
  hosts; target-specific code only selects compatible routes and presents the
  shared conversation.
- **`auki-p2p` remains the lower transport layer.** It owns mutual Domain/Peer
  authentication, exact direct and relay streams, and authenticated-peer
  observations. Most applications should use it through `AukiPeer` rather than
  assemble its lifecycle directly.
- **`auki-protocols` owns retained wire contracts, not a runtime.** Its opt-in
  features provide exact IDs, bounded codecs, validation, and transport-neutral
  types for resource catalogs, registries, blobs, messages, and typed streams.
- **`Peer` / `Session` / `Domain` are retained compatibility surfaces.**
  `auki-session` remains the network-free recording model, while `auki-domain`
  exposes the earlier low-level authenticated Domain owner and retained
  protocols. They are still available to existing consumers, but new P2P
  applications should build on `auki_sdk::AukiPeer`.
- The resource catalog exposes rows discriminated by `variant`
  (`sensor_log` | `pose_log` | `time_transform_log` | `detection_log`) and is
  sampled live from the provider served by the retained Domain protocol stack.
  Registry/blob reads are content-pinned, messages are receiver-owned, and typed
  streams bind the expected authenticated producer before application bytes
  flow.
- **HTTP control API** for daemons that produce SDK sessions — see [`docs/control-api.md`](docs/control-api.md).

### Tokenomics

- Not implemented. The `Wallet` under Identity is the on-device primitive future payment / billing rails will bind to.

The first live pose-stream hardware target is Galbot G1 using RoboStreamer to publish `base_link -> head_left_rgb_optical` pose logs into Park. Tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5).

## Crate map

### `crates/` — Rust workspace

| Crate | Purpose | Status |
|---|---|---|
| [`auki-hash`](crates/auki-hash) | XXH3-128 wrapper for content-addressing | ✓ |
| [`auki-jcs`](crates/auki-jcs) | RFC 8785 JSON canonicalization | ✓ |
| [`auki-identity`](crates/auki-identity) | ed25519 wallet + child derivation + signed creation certs | ✓ |
| [`auki-time`](crates/auki-time) | `SessionClock`, fixed affine `TimeTransform` math, clock traits, and local sampling primitives; no network-derived Domain clock | ✓ |
| [`auki-logs`](crates/auki-logs) | Generic segmented append-only log primitive | ✓ |
| [`auki-registry`](crates/auki-registry) | Sensor / Clock / Frame / Map identity catalogs + IO | ✓ |
| [`auki-datatypes`](crates/auki-datatypes) | Shared protobuf segment + wire payload schemas | ✓ |
| [`auki-manifests`](crates/auki-manifests) | JCS-JSON log-manifest builders (sensor / pose / TT / detection / map) | ✓ |
| [`auki-layout`](crates/auki-layout) | On-disk path helpers for session/log layout | ✓ |
| [`auki-geometry`](crates/auki-geometry) | Convention conversion for points / vectors / poses | ✓ |
| [`auki-maps`](crates/auki-maps) | Deterministic voxel Map accumulation + renderer-neutral chunk updates | ✓ |
| [`auki-mappers`](crates/auki-mappers) | SDK-native Map producers; point-cloud + pose voxel Mapper | ✓ |
| [`auki-p2p`](crates/auki-p2p) | Cross-target authenticated libp2p transport, stable identity primitive, exact native TCP/browser WSS routes, relay circuits, and peer observations | ✓ |
| [`auki-auth`](crates/auki-auth) | Bounded User/App API + DDS authority preparation for one selected Domain; App credentials are native-only | ✓ (source, unpublished) |
| [`auki-sdk`](crates/auki-sdk) | Canonical `AukiPeer` facade for authority renewal, authenticated transport, default DMS relay allocation, explicit protocols, routes, status, and shutdown on native and Web/Wasm targets | ✓ (source, unpublished) |
| [`auki-protocols`](crates/auki-protocols) | Exact authenticated-protocol IDs, bounded codecs, validation, and transport-neutral wire types; no runtime | ✓ |
| [`auki-session`](crates/auki-session) | Legacy-compatible, network-free recording model: `Peer`, `Session`, registries, and log registration | ✓ |
| [`auki-domain`](crates/auki-domain) | Legacy/low-level authenticated Domain owner with retained catalogs, registries, blobs, messages, and streams; existing consumers remain supported | Compatibility |
| [`auki-domain-relay`](crates/auki-domain-relay) | Earlier Domain relay capability retained in source; not the canonical `AukiPeer` relay path | WIP (v0.0.0) |
| [`auki-ros-adapter`](crates/auki-ros-adapter) | ROS2 → SDK glue for `Image` / `CameraInfo` / `PointCloud2` | ⚠ broken at the `r2r 0.9.5` transport layer |

### `bindings/web/` — wasm-bindgen

| Package | Purpose | Status |
|---|---|---|
| [`auki-sdk-web`](bindings/web/auki-sdk-web) | Generic Web facade for User login, accessible-Domain selection, one ephemeral `AukiPeer`, mandatory DMS relay reachability, exact WSS routes, lifecycle barriers, and portable Rust protocols | ✓ (source-only, unpublished) |

### `bindings/python/` — PyO3 / betterproto

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-py`](bindings/python/auki-identity-py) | Wallet + per-machine identity | ✓ |
| [`auki-datatypes-py`](bindings/python/auki-datatypes-py) | betterproto dataclasses for the shared protobuf types | ✓ |
| [`auki-logs-py`](bindings/python/auki-logs-py) | `Log<T>` with opaque-bytes payload | ✓ |
| [`auki-registry-py`](bindings/python/auki-registry-py) | Sensor / Clock / Frame registry IO | ✓ |
| [`auki-manifests-py`](bindings/python/auki-manifests-py) | Log-manifest builders | ✓ |
| [`auki-layout-py`](bindings/python/auki-layout-py) | On-disk path helpers | ✓ |
| `auki-network-py` | Removed Manager-era networking binding; it has no current `AukiPeer` replacement yet | Removed |
| [`auki-domain-py`](bindings/python/auki-domain-py) | Legacy/low-level Python compatibility binding over the retained authenticated `Domain` owner | Compatibility |
| [`auki-mappers-py`](bindings/python/auki-mappers-py) | Python boundary for SDK-native Mappers; normalized point cloud + registry + pose to `MapUpdate` | ✓ |
| [`auki-session-py`](bindings/python/auki-session-py) | Network-free Python recording surface for `Peer`, `Session`, registries, log specs, and handles | ✓ |
| Python `AukiPeer` facade | Planned binding over the canonical Rust peer lifecycle and portable protocols | Pending |

### `bindings/swift/` — UniFFI

| Package | Purpose | Status |
|---|---|---|
| [`auki-identity-swift`](bindings/swift/auki-identity-swift) | `Wallet` only; no P2P runtime or `PeerIdentity` compatibility type | ✓ |
| Swift `AukiPeer` facade | Planned binding over the canonical Rust peer lifecycle and portable protocols | Pending |

Each package's own `README.md` documents its current state, public surface, and
dependencies. The removed Manager-era `auki-network`, `auki-network-py`,
`auki-network-swift`, `auki-network-browser-wasm`, and `auki-domain-browser`
sources are available only at the prior
[`v0.0.60` tag](https://github.com/aukilabs/auki-sdk/tree/v0.0.60); they are not
wire-compatible with Stage 1. Do not treat the retained Rust/Python `Domain`
surfaces as a new Manager: they exist for low-level and compatibility use. New
Rust and Web P2P applications use `AukiPeer`; Python and Swift must wait for
their dedicated `AukiPeer` facades rather than reviving Manager semantics.

## Examples

- [`diagnostic-app`](examples/diagnostic-app) retains a low-level authenticated
  native Domain transport diagnostic over direct TCP.
- [`portable-echo`](examples/portable-echo) keeps one bounded wire contract and
  its `AukiPeer` endpoint in one Rust crate consumed by both platform hosts.
  - The [native reference app](examples/portable-echo/native) is a small,
    copyable User-authenticated peer with a stable identity, default DMS relay,
    exact-route send, inbound serving, and ordered shutdown.
  - The [Web/Wasm source example](examples/portable-echo/web) provides a minimal
    User-authenticated ephemeral peer and a two-tab playground. It is available
    in this repository but is not a published package.
  - Its protected smoke test proves browser-to-browser in both directions,
    native-to-browser, and browser-to-native using the remote peer's exact TCP
    or WSS route. It does not rely on discovery or automatic route publication.

## Contributing & license

Work is tracked on the [SDK Kanban](https://github.com/orgs/Aukilabs/projects/5). See [CONTRIBUTING.md](CONTRIBUTING.md) for the folder convention, board flow, and git hygiene rules; [CLAUDE.md](CLAUDE.md) is the equivalent for AI agents.

MIT — see [LICENSE](LICENSE). Copyright © 2026 Auki Labs Limited.
