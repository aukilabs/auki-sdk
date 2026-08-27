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

Wallet primitive: ed25519 keypair with deterministic label-based child derivation (BIP32-like). The seed from `Wallet::derive_child("peer/v1")` constructs the canonical `auki_p2p::Identity` and libp2p `PeerId`; future payment/billing rails will derive their keys here too. WASM-friendly.

---

## Shared schemas

Two crates own the SDK's data shapes. They're parallel: `auki-datatypes` owns wire/disk **payload** shapes (segment bytes); `auki-manifests` owns the **manifest** sidecar shapes that describe those payloads.

### [`auki-datatypes`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-datatypes)

`.proto` schemas + prost-generated Rust for every cross-language payload:
camera, point cloud, audio, joint encoders, detections, pose, time transforms,
and the `/auki/auth/1/stream/0.2.0` envelope. One payload message per modality
is reused on disk and on authenticated streams.

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

Clock traits, `SessionClock`, fixed affine `TimeTransform` math, and local
sampling primitives. Recorded TimeTransform Logs carry explicit clock lineage;
no Domain runtime owns a hidden synchronized clock.

### [`auki-geometry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-geometry)

Pure spatial math. Phase 1 ships convention conversion via `convert_pose_convention` (plus `_point_`, `_vector_`, `_direction_` siblings) — the convention-only layer underneath the full `convert_pose` (path-walking composition) that's still pending. No I/O, no network.

---

## Network

Authenticated transport, transport-neutral wire contracts, and the public
Domain lifecycle.

### [`auki-p2p`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-p2p)

Owns the stable libp2p identity and the single native node: TCP/Noise/Yamux,
DDS-signed Domain credentials, mutual-authentication framing, explicit direct
and relay routes, relay reservations, and authenticated-peer observations. It does
not fetch credentials or routes over HTTP.

### [`auki-protocols`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-protocols)

Owns the exact authenticated `/auki/auth/1/...` IDs, versioned wire types,
bounded framing, validation, and locked vectors for info, catalogs, registries,
blobs, messages, and typed streams. Protocol families are compile-time opt-in;
the crate owns no transport, handlers, or task lifecycle.

### [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain)

The app-facing owner for one exact DDS Domain UUID. `DomainBuilder` binds one
`Peer`/`Session` pair to one `auki-p2p` node with host-supplied credentials,
listeners, explicit routes, and exact-version `ServedProtocols`. The default
serves none; client operations remain available independently. `Domain` exposes
authenticated known peers, catalogs, registry/blob fetches, messages, typed
streams, and ordered leave. There is no Manager, membership roster, election,
or hidden discovery policy.

### [`auki-domain-relay`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain-relay)

Standalone native/WebSocket Circuit Relay v2 server. Hosts distribute relay
routes and authority through their own control plane; the relay owns no Domain
membership or topology policy.

---

## App surface

One crate. The entry point for app code.

### [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session)

Declarative app-facing API shipped in #216, split into `Peer` / `Session` in #282. Apps construct a long-lived `Peer` and declare what it has (`register_sensor` / `register_frame` / `register_detector`), then mint sessions via `Peer::start_session()` — which auto-registers the session's monotonic + UTC clocks (#284) — and register the logs each session writes (`register_clock`, `register_sensor_log` / `register_pose_log` / `register_time_transform_log` / `register_detection_log`). The SDK handles registry I/O, manifest persistence, and session-clock registration internally; going on-network is [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain)'s `Domain::join`. `materialize_remote_log` and `resolve_static_transform` ship as `NotImplemented` stubs — Phase 5 of #216.

---

## Adapters

External-system bridges. Each adapter targets one foreign data plane.

### [`auki-ros-adapter`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-ros-adapter)

ROS 2 → SDK translator: `CameraInfo` / `Image` / `PointCloud2` → registry entries + sensor log entries. `frame_id` + `frame_hash` thread through both builders so sensor entries commit to an exact Frame Registry version. Currently **broken** at the `r2r` 0.9.5 transport layer — fix in flight.

The old Rust/WASM and TypeScript browser runtimes were removed from HEAD. Their
sources remain available only in
[`v0.0.60`](https://github.com/aukilabs/auki-sdk/tree/v0.0.60) and cannot join
the authenticated Stage 1 runtime. Browser support requires a future external
authenticated-engine migration.

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
- `auki-network-py` — removed Manager-era package; use `auki-domain-py`
- [`auki-domain-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-domain-py) — authenticated facade over the same Rust `Domain` owner
- [`auki-session-py`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/python/auki-session-py) — `Peer`, `Session`, registry/log specs and handles; re-exports shared registry references

### Swift (UniFFI)

- [`auki-identity-swift`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/swift/auki-identity-swift) — Wallet only

The old Manager-compatible `auki-network-swift` source was removed from HEAD.
It remains available only in
[`v0.0.60`](https://github.com/aukilabs/auki-sdk/tree/v0.0.60/bindings/swift/auki-network-swift)
and is not compatible with Stage 1.

### Browser (TypeScript)

There is no current browser package in HEAD. The Manager-era browser sources
are available at `v0.0.60`; an authenticated replacement is a future external
migration.

---

## Examples

- [`examples/diagnostic-app`](https://github.com/aukilabs/auki-sdk/tree/develop/examples/diagnostic-app) — scriptable authenticated Domain peer and two-process direct-TCP demo.

---

## A consumer's likely subset

If you're building a robot data-plane producer, you probably pull:

- `auki-session` + `auki-registry` (Rust) — what the [Quickstart](Quickstart) uses.
- `auki-session-py` + `auki-registry-py` (Python) — equivalent.

You generally don't touch `auki-logs` or `auki-protocols` directly: `Session`
owns the recording timeline and `Domain` owns authenticated network I/O. The
lower-level crates remain available for unusual cases such as processing logs
offline or authoring an additional authenticated application protocol.

For visualizers consuming other peers' data (Park, browser dashboards), the path is:

- Rust today: `auki-domain::Domain` for authenticated catalog fetches and typed
  stream opens against an expected peer.
- Python today: `auki-domain-py` exposes the same authenticated Domain owner.
- Browser consumers remain on their last compatible line until the browser
  authenticated-engine stage lands.

---

## See also

- [The Five Questions](The-Five-Questions) — which crates address which architectural question
- [Quickstart](Quickstart) — the minimum producer-side surface in action
- [Release history](Release-History) — what shipped in each tag
- [Top-level README](https://github.com/aukilabs/auki-sdk/blob/develop/README.md) — the canonical crate-map table with per-crate status icons

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Release history →](Release-History)
