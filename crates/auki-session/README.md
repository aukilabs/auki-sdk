# auki-session

Identity and log lifecycle for a single peer — the network-free core of the SDK's app-facing API. Apps construct a long-lived `Peer`, register the sensors / frames / detectors it owns, then mint a `Session` (one timeline) from it and register the logs that session writes. Putting the pair on the network is **not** this crate's job — that's [`auki-domain`](../auki-domain)'s `Domain::join(&peer, &session, config)`, which composes both.

Shipped in SDK #216 (2026-05-27) as a `Session`-centric API; split into `Peer` / `Session` with the network surface inverted out to `auki-domain` in #282/#284 (2026-06).

**Status:** Shipped. Zero networking dependencies — see [Depends on](#depends-on).

## Public surface

### Peer — long-lived identity + registries

- `Peer::new(peer_id, app_id)` — `peer_id` is the libp2p peer-id string the app derived from its wallet (`PeerIdentity::from_wallet(...)`). The peer outlives any one session.
- `Peer::with_storage_root(root)` — take-by-value builder; sets the disk root for registry and log files.
- `Peer::set_storage_root(&self, root)` — in-place equivalent for FFI / binding wrappers that can't express take-by-value builders (PyO3, UniFFI).
- `Peer::peer_id()`, `app_id()`, `storage_root()` — read accessors.
- `Peer::register_sensor(sensor_id, SensorBody)` — writes the entry under `registries/sensors/<peer_id>/<sensor_id>/<hash>.json`; returns a `RegistryRef { peer_id, id, hash }`.
- `Peer::register_frame(frame_id, FrameDef)` — `FrameDef` presets: `ros_body()`, `ros_optical()`, `opengl()`, `unity()`.
- `Peer::register_detector(detector_id, DetectorBody, output_types)`.
- `Peer::start_session()` → `Session` — mints a fresh ULID `session_id` and auto-registers the session's clock pair (below).
- `Peer::registries()` → `PeerRegistries` — read handle (`sensor(id)`, `frame(id)`, `detector(id)`); consumed by `auki-domain` to resolve entries at catalog-build time.

All IDs are validated — `>`, `@`, and whitespace are rejected.

### Session — one timeline born from a Peer

There is no public `Session` constructor: sessions come from `Peer::start_session()`. Starting a session mints a ULID `session_id` and registers the session's two clocks on disk — a monotonic clock and a UTC clock, with SDK-owned ids `{peer_id}/{session_id}/monotonic` and `…/utc` (#284: clock identity is SDK-minted, not daemon convention).

- Read accessors: `peer_id()`, `app_id()`, `session_id()`, `storage_root()` — read live through the shared peer state.
- `Session::monotonic_clock()` / `utc_clock()` → `RegistryRef` — the auto-minted pair; pass these into log specs.
- `Session::register_clock(clock_id, ClockBody)` — additional session-scoped clocks.
- `Session::logs()` → `SessionLogs` — read handle (`sensor_logs()`, `pose_logs()`, `time_logs()`, `detection_logs()`); consumed by `auki-domain` for live catalog snapshots.

### Log registration

Each returns a typed handle carrying `resource_id`, `log_ref: LogRef`, and the full manifest. Duplicate `(source_peer_id, resource_id)` pairs are rejected with `SessionError::DuplicateLog`. Handles are *declarations* — the app still opens the `auki-logs` `Log<T>` at the layout path and runs the append loop.

- `Session::register_sensor_log(SensorLogSpec)` → `SensorLogHandle` — `resource_id` is `sensor.id`.
- `Session::register_pose_log(PoseLogSpec)` → `PoseLogHandle` — `resource_id` is `"<from_frame.id>-><to_frame.id>"`.
- `Session::register_time_transform_log(TimeTransformLogSpec)` → `TimeTransformLogHandle` — `resource_id` is `"<from_clock.id>-><to_clock.id>"`.
- `Session::register_detection_log(DetectionLogSpec)` → `DetectionLogHandle` — `resource_id` is `"<detector.id>@<input_sensor.id>"`.

Log spec types (`SensorLogSpec`, `PoseLogSpec`, `TimeTransformLogSpec`, `DetectionLogSpec`) and `HeadSpec` (`Rolling { retention_ns }` / `Fixed`) live in `auki_session::log_specs`.

### Materialization stubs (Phase 5, not yet implemented)

- `Session::materialize_remote_log(log_ref, retention, segment_duration)` (async) — currently returns `MaterializationError::NotImplemented`.
- `Session::resolve_static_transform(log_ref)` (async) — reads a sealed one-sample pose log. Currently returns `MaterializationError::NotImplemented`.

### Catalog and domain — moved out in #282

`Session::catalog()`, `Session::join_domain()`, `Session::leave_domain()`, and `Session::cluster_manager()` no longer exist. The equivalents live in [`auki-domain`](../auki-domain): `catalog_of(&peer, &session)` (pure, no network) and `Domain::join(&peer, &session, DomainConfig)` / `Domain::catalog()` / `Domain::leave()`.

## Depends on

- [`auki-registry`](../auki-registry) — entry types and `RegistryRef` / `LogRef`.
- [`auki-manifests`](../auki-manifests) — manifest builders and `PoseSource` / `PoseWriterMode` / `TimeTransformSource`.
- [`auki-logs`](../auki-logs) + [`auki-datatypes`](../auki-datatypes) — log primitive + payload types (`SpatialTransform` stub return type).
- [`auki-identity`](../auki-identity), [`auki-time`](../auki-time), [`auki-hash`](../auki-hash), [`auki-jcs`](../auki-jcs).

Deliberately **not** here: `auki-network` and `auki-domain`. The dependency points the other way — `auki-domain` consumes `Peer::registries()` + `Session::logs()` through its catalog bridge (#282 dependency inversion).
