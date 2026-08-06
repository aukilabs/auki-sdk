# Crate map

The SDK is a Cargo workspace of ~15 Rust crates plus per-language bindings. The [top-level README's table](https://github.com/aukilabs/auki-sdk/blob/develop/README.md#crate-map) is the at-a-glance summary; this page walks the same set in narrative form, organized by layer, so you can build a mental dependency graph.

Reading direction: lower layers know nothing about higher layers. A crate in the **Foundations** layer can be used by anything above it; a crate in the **App surface** layer can use anything below.

---

## Foundations

Pure primitives. No SDK-specific concepts, no I/O. Anything in the SDK can depend on these.

### [`auki-hash`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-hash)

XXH3-128 wrapper used everywhere the SDK content-addresses something. The hash *is* the version: refining a registry entry produces a new hash, sibling-stored under the same id.

### [`auki-jcs`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-jcs)

RFC 8785 JCS (JSON Canonicalization Scheme). Every JSON the SDK hashes — registry entries, manifests, catalog rows — is canonicalized first so the bytes are stable across languages and pretty-printers.

### [`auki-identity`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-identity)

Wallet primitive: ed25519 keypair with deterministic label-based child derivation (BIP32-like). `Wallet::derive_child("peer/v1")` produces the libp2p `PeerId`; future payment/billing rails will derive their keys here too. WASM-friendly.

---

## Shared schemas

Two crates own the SDK's data shapes. They're parallel: `auki-datatypes` owns wire/disk **payload** shapes (segment bytes); `auki-manifests` owns the **manifest** sidecar shapes that describe those payloads.

### [`auki-datatypes`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-datatypes)

`.proto` schemas + prost-generated Rust for every cross-language payload: `camera::CameraFrame`, `point_cloud::Data`, `audio::Data`, `joint_encoders::Data`, `detection::DetectionFrame`, `pose::SpatialTransform`, `time_transform::TimeTransformEntry`, plus the `/auki/stream/0.2.0` `StreamMessage` envelope. One `Data` message per modality used on both disk segment and wire substream — the dual `*_stream` packages were collapsed in #176.

### [`auki-manifests`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-manifests)

JCS-canonical JSON manifests for the four log types. `SensorLogManifest`, `PoseLogManifest`, `TimeTransformLogManifest`, `DetectionLogManifest`. Post-#216 each carries `source_peer_id` (canonical) + `writer_peer_id` (this file's writer) + registry refs + writer-local rollover params. Builders for each plus enums (`PoseSource`, `PoseWriterMode`, `TimeTransformSource`) live here.

---

## Storage and identity catalogs

The on-disk primitives. Logs and registries.

### [`auki-layout`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-layout)

Disk-path helpers — single source of truth for where a sensor log / pose log / time-transform log / detection log / registry entry lives under `<app_root>`. Pure path computation; no I/O.

### [`auki-logs`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-logs)

Generic segmented append-only log primitive. `Log<T>` over a `LogPayload` trait: encoder-agnostic, segment rollover is time-based (`segment_duration_ns`), eviction is retention-based (`retention_ns`), and the two are independent. Every on-disk log type (sensor, pose, time-transform, detection) is a `Log<T>` over a different payload from `auki-datatypes`.

### [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry)

The five registries: Sensor, Clock, Frame, Detector, and Map. Each entry post-#216 carries an explicit `peer_id` top-level field; the key is `(peer_id, id, hash)` where the hash is XXH3-128 over the entry's JCS-canonical JSON. Frame Registry ships four preset constructors (`ros_body`, `ros_optical`, `opengl`, `unity`); `SensorBody` is a closed enum (`Camera`, `Rangefinder`, `Rf`, `Audio`, `JointEncoders`, `Scalar`). Every sensor has an open `type: String`; spatial variants pin a Frame Registry reference, while `Scalar` pins its unit and expected rate without a frame.

### [`auki-time`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-time)

Clock primitives, TimeTransform math, NTP-style samplers. `SessionClock` for per-process monotonic time, `TimeTransform` math for the (planned) `convert_time` operation, `NtpExchange` / `NtpSample` / `compute_ntp_sample` for the heartbeat-driven domain-clock convergence, and the 1 Hz `local_clock_read` sampler that writes a `MonotonicClock ↔ UtcClock` TimeTransform Log on each device.

### [`auki-geometry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-geometry)

Pure spatial math. Phase 1 ships convention conversion via `convert_pose_convention` (plus `_point_`, `_vector_`, `_direction_` siblings) — the convention-only layer underneath the full `convert_pose` (path-walking composition) that's still pending. No I/O, no network.

---

## Network

The libp2p substrate and the cluster lifecycle layer.

### [`auki-network`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-network)

libp2p substrate: TCP/QUIC, Noise, Yamux, Circuit Relay v2, identify, ping. On top of that, the seven peer protocols (`/auki/join`, `/auki/heartbeat`, `/auki/membership`, `/auki/info`, `/auki/resources/0.2.0`, `/auki/registries/0.2.0`, `/auki/stream/0.2.0`), the typed stream payload registry, the Discovery HTTP client (with relay-address hints for browser-compatible reachability), and the `SessionHandle` trait that breaks the would-be `auki-domain ↔ auki-session` cycle.

### [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain)

`Domain::join(&peer, &session, config)` — the app-facing network-presence API (#282) — on top of `ClusterManager`: cluster bootstrap, Manager election, membership exchange, Discovery liveness checks, relay-hint preservation, resource catalog exchange, registry-entry fetch, stream serving, graceful shutdown. App code never constructs `ClusterManager` directly; `Domain::join` builds and owns one internally. Re-exports `ResourceEntry` from `auki-network` so consumers of either crate see the same type.

### [`auki-domain-relay`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain-relay)

Domain Relay capability — runs a native- and browser-compatible Circuit Relay v2 server so Managers reachable only over TCP can advertise WebSocket-circuit addresses for browser peers to dial. WIP; domain-scoped reservation policy and grants are pending.

---

## App surface

One crate. The entry point for app code.

### [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session)

Declarative app-facing API shipped in #216, split into `Peer` / `Session` in #282. Apps construct a long-lived `Peer` and declare what it has (`register_sensor` / `register_frame` / `register_detector`), then mint sessions via `Peer::start_session()` — which auto-registers the session's monotonic + UTC clocks (#284) — and register the logs each session writes (`register_clock`, `register_sensor_log` / `register_pose_log` / `register_time_transform_log` / `register_detection_log`). The SDK handles registry I/O, manifest persistence, and session-clock registration internally; going on-network is [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain)'s `Domain::join`. `materialize_remote_log` and `resolve_static_transform` ship as `NotImplemented` stubs — Phase 5 of #216.

---

## Adapters

External-system bridges. Each adapter targets one foreign data plane (ROS 2, browser, etc.).

### [`auki-ros-adapter`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-ros-adapter)

ROS 2 → SDK translator: `CameraInfo` / `Image` / `PointCloud2` → registry entries + sensor log entries. `frame_id` + `frame_hash` thread through both builders so sensor entries commit to an exact Frame Registry version. Currently **broken** at the `r2r` 0.9.5 transport layer — fix in flight.

### [`auki-network-browser-wasm`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-network-browser-wasm)

Browser/WASM libp2p transport probe. WIP (v0.0.0); the production browser story currently runs through the TypeScript `auki-domain-browser` package and Domain Relay reachability.

---

## Bindings

Per-language wrappers. The pattern is **per-component naming** — no umbrella `auki-py` package; each Rust crate gets its own `<crate>-py` mirror so consumers pull exactly what they need.

### Python (PyO3)

- [`auki-identity-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-identity-py) — Wallet + `app_instance.derive`
- [`auki-datatypes-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-datatypes-py) — betterproto dataclasses for the shared protobuf types
- [`auki-logs-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-logs-py) — `Log<T>` with opaque-bytes payload
- [`auki-registry-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-registry-py) — registry IO + `RegistryRef` / `LogRef`
- [`auki-manifests-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-manifests-py) — manifest builders
- [`auki-layout-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-layout-py) — path helpers
- [`auki-geometry-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-geometry-py) — convention conversion math
- [`auki-network-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-network-py) — Discovery client value types + shared stream pyclasses
- [`auki-domain-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-domain-py) — `ClusterManager` facade with typed stream openers including `open_pose_stream`; resurrected for the post-#216 schema in #231
- [`auki-session-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-session-py) — `Session` + register_* + log specs/handles + `catalog()`; re-exports `RegistryRef` / `LogRef` from `auki-registry-py` for type sharing

### Swift (UniFFI)

- [`auki-identity-swift`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/swift/auki-identity-swift) — Wallet + `PeerIdentity`
- [`auki-network-swift`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/swift/auki-network-swift) — Discovery client + `NetworkRuntime` + 5-payload streams

### Browser (TypeScript)

- [`auki-domain-browser`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain-browser) — TypeScript browser `Peer` contract types. WIP.

---

## Examples

- [`examples/diagnostic-app`](https://github.com/aukilabs/auki-sdk/tree/develop/examples/diagnostic-app) — end-to-end demo that opens a cluster, joins a domain, logs diagnostic flash timestamps. Useful as a worked reference for runtime wiring.

---

## A consumer's likely subset

If you're building a robot data-plane producer, you probably pull:

- `auki-session` + `auki-registry` (Rust) — what the [Quickstart](Quickstart) uses.
- `auki-session-py` + `auki-registry-py` (Python) — equivalent.

You generally don't touch `auki-logs` / `auki-network` / `auki-domain` directly any more — `Session` owns the I/O and the cluster lifecycle. The lower-level crates remain available for unusual cases (running a peer without `Session`, integrating with a non-libp2p transport, processing logs offline).

For visualizers consuming other peers' data (Park, browser dashboards), the path is:

- Today: `auki-domain-py` or `auki-network-py` for stream consumption against a known peer (typed openers exist for camera / point-cloud / joint-encoders / audio / pose).
- After Phase 5 of #216 lands: `Session::materialize_remote_log` for persistent local replicas with custom retention.

---

## See also

- [The Five Questions](The-Five-Questions) — which crates address which architectural question
- [Quickstart](Quickstart) — the minimum producer-side surface in action
- [Release history](Release-History) — what shipped in each tag
- [Top-level README](https://github.com/aukilabs/auki-sdk/blob/develop/README.md) — the canonical crate-map table with per-crate status icons

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Release history →](Release-History)
