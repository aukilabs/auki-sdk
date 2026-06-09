# auki-session

Declarative app-facing API for the Auki SDK. Apps construct a `Session`, register their sensors / clocks / frames / detectors and the logs they own, then optionally join a domain to advertise them.

Shipped in SDK #216 (2026-05-27). Previously the SDK had no `Session` abstraction; apps called registry and manifest builders directly.

**Status:** Shipped.

## Public surface

### Session construction and identity

- `Session::new(peer_id, app_id)` — creates a session with a fresh ULID `session_id`.
- `Session::with_storage_root(path)` — take-by-value builder; sets the disk root for registry and log files. Preserves `session_id`.
- `Session::set_storage_root(&self, path)` — in-place mutator equivalent. Same effect as `with_storage_root`, but doesn't consume `self`. Useful for FFI / binding wrappers that can't express take-by-value builder patterns (PyO3, UniFFI). Preserves `session_id`.
- `Session::peer_id()`, `app_id()`, `session_id()`, `storage_root()` — read accessors.

### Registry registration

Each returns a `RegistryRef { peer_id, id, hash }`. All IDs are validated via `validate_registry_id` — `>`, `@`, and whitespace are rejected.

- `Session::register_sensor(sensor_id, SensorBody)` — writes the entry under `registries/sensors/<peer_id>/<sensor_id>/<hash>.json`.
- `Session::register_clock(clock_id, ClockBody)` — writes under `registries/clocks/...`.
- `Session::register_frame(frame_id, FrameDef)` — takes a `FrameDef` preset; session fills in `peer_id` and `frame_id`. Writes under `registries/frames/...`.
- `Session::register_detector(detector_id, DetectorBody, output_types)` — writes under `registries/detectors/...`.

`FrameDef` presets: `FrameDef::ros_body()`, `ros_optical()`, `opengl()`, `unity()`.

### Log registration

Each returns a typed log handle carrying `resource_id` and `log_ref: LogRef`. Duplicate `(source_peer_id, resource_id)` pairs are rejected with `SessionError::DuplicateLog`.

- `Session::register_sensor_log(SensorLogSpec)` → `SensorLogHandle` — `resource_id` is `sensor.id`.
- `Session::register_pose_log(PoseLogSpec)` → `PoseLogHandle` — `resource_id` is `"<from_frame.id>-><to_frame.id>"`.
- `Session::register_time_transform_log(TimeTransformLogSpec)` → `TimeTransformLogHandle` — `resource_id` is `"<from_clock.id>-><to_clock.id>"`.
- `Session::register_detection_log(DetectionLogSpec)` → `DetectionLogHandle` — `resource_id` is `"<detector.id>@<input_sensor.id>"`.

Log spec types (`SensorLogSpec`, `PoseLogSpec`, `TimeTransformLogSpec`, `DetectionLogSpec`) and `HeadSpec` (`Rolling { retention_ns }` / `Fixed`) live in `auki_session::log_specs`.

### Catalog

- `Session::catalog()` → `Vec<ResourceEntry>` — one row per registered log in the `/auki/resources/0.2.0` shape.

### Domain

- `Session::join_domain(DomainConfig)` (async) — bootstraps a `ClusterManager`, wires it a `SessionHandle`, and stores it in the session. `DomainConfig` carries `ClusterTarget`, `PeerIdentity`, multiaddrs, Discovery URL, libp2p swarm, `StreamProvider`, and `DaemonInfo`.
- `Session::leave_domain()` (async) — shuts down the active `ClusterManager`. No-op if none joined.
- `Session::cluster_manager()` — returns the active `ClusterManager` reference if joined.

### Materialization stubs (Phase 5, not yet implemented)

- `Session::materialize_remote_log(log_ref, retention, segment_duration)` (async) — currently returns `MaterializationError::NotImplemented`.
- `Session::resolve_static_transform(log_ref)` (async) — reads a sealed one-sample pose log. Currently returns `MaterializationError::NotImplemented`.

## Depends on

- [`auki-registry`](../auki-registry) — for entry types and `RegistryRef` / `LogRef`.
- [`auki-manifests`](../auki-manifests) — for manifest builders and `PoseSource` / `PoseWriterMode` / `TimeTransformSource`.
- [`auki-network`](../auki-network) — for `SessionHandle` trait and `ResourceEntry` catalog types.
- [`auki-domain`](../auki-domain) — for `ClusterManager`, `ClusterTarget`, `DaemonInfo`.
- [`auki-datatypes`](../auki-datatypes) — for `SpatialTransform` (stub return type).
