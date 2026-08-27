# auki-session

Identity and log lifecycle for a single peer — the network-free core of the SDK's app-facing API. Apps construct a long-lived `Peer`, register the sensors / frames / detectors it owns, then mint a `Session` (one timeline) from it and register the logs that session writes. Putting the pair on the network is **not** this crate's job — [`auki-domain`](../auki-domain) composes both through `Domain::builder(&peer, &session, config).authority(keys, credential).join()`.

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
- `Peer::register_map(map_id, MapBody)` — writes the immutable Map contract and returns its content-addressed `RegistryRef`.
- `Peer::register_device_model(device_model_id, DeviceModelBody)` / `Peer::register_urdf_package(id, urdf_path, root_convention, package_root, mesh_substitutions)` — blob + register a URDF device model; optional substitutions remap mesh filenames at advertise (e.g. STL→Draco GLB) without parsing manufacturer JSON in Rust.
- `Peer::start_session()` → `Session` — mints a fresh ULID `session_id` and auto-registers the session's clock pair (below).
- `Peer::registries()` → `PeerRegistries` — read handle (`sensor(id)`, `frame(id)`, `detector(id)`, `map(id)`); consumed by `auki-domain` to resolve entries at catalog-build time.

All IDs are validated — `>`, `@`, and whitespace are rejected.

### Session — one timeline born from a Peer

There is no public `Session` constructor: sessions come from `Peer::start_session()`. Starting a session mints a ULID `session_id` and registers the session's two clocks on disk — a monotonic clock and a UTC clock, with SDK-owned ids `{peer_id}/{session_id}/monotonic` and `…/utc` (#284: clock identity is SDK-minted, not daemon convention).

- Read accessors: `peer_id()`, `app_id()`, `session_id()`, `storage_root()` — read live through the shared peer state.
- `Session::monotonic_clock()` / `utc_clock()` → `RegistryRef` — the auto-minted pair; pass these into log specs.
- `Session::register_clock(clock_id, ClockBody)` — additional session-scoped clocks.
- `Session::logs()` → `SessionLogs` — read handle (`sensor_logs()`, `pose_logs()`, `time_logs()`, `detection_logs()`, `map_logs()`); consumed by `auki-domain` for live catalog snapshots.

### Log registration

Each returns a typed handle carrying `resource_id`, `log_ref: LogRef`, and the full manifest. Duplicate `(source_peer_id, resource_id)` pairs are rejected with `SessionError::DuplicateLog`.
A sensor producer opens its `auki-logs` `Log<T>` at `handle.root()` and appends source samples there.

- `Session::register_sensor_log(SensorLogSpec)` → `SensorLogHandle` — `resource_id` is `sensor.id`.
- `Session::register_pose_log(PoseLogSpec)` → `PoseLogHandle` — `resource_id` is `"<from_frame.id>-><to_frame.id>"`.
- `Session::register_time_transform_log(TimeTransformLogSpec)` → `TimeTransformLogHandle` — `resource_id` is `"<from_clock.id>-><to_clock.id>"`.
- `Session::register_detection_log(DetectionLogSpec)` → `DetectionLogHandle` — `resource_id` is the application-chosen `instance_id`; the manifest binds that instance to an exact detector, input log, input sensor contract, clock, and cadence.
- `Session::register_map_log(MapLogSpec)` → `MapLogHandle` — `resource_id` is the Map id. This handle owns the durable `Log<MapUpdate>` writer and provides append, replay, live subscription, an atomic replay/live boundary, and `persisted_bytes()` diagnostics.

### Detector execution

`RegisteredCameraDetector::register` is the bring-your-own detector entry point. A developer supplies a `CameraDetector` factory plus the registry body, accepted camera contracts, and declared output types. The SDK creates a fresh detector value for every started instance, validates the selected sensor against the registered contracts, owns cadence and provenance, and rejects any emitted output type that was not declared. Third-party implementations use `DetectorBody::Custom(CustomDetector { .. })`; `kind` is an open namespaced identifier and the configuration participates in the content-addressed registry hash. Built-in bodies such as `Qr` are conveniences, not a closed implementation list.

```rust,ignore
let registered = RegisteredCameraDetector::register(
    &peer,
    "my-detector",
    DetectorBody::Custom(CustomDetector {
        kind: "com.example.my-detector".into(),
        configuration: serde_json::json!({"model": "v2"}),
    }),
    vec![camera_input_contract],
    vec!["example.result".into()],
    || MyDetector::new(),
)?;

let task = registered.start(&session, instance, &sensor_log)?;
```

`DetectorTask::start(detector, camera, input, output)` is the detector-agnostic recorded/local runner. It tails an open Camera Sensor Log in order and does not drop accepted samples. `StreamingDetectorTask::start(detector, camera, binding, frames, output)` consumes any asynchronous stream of `CameraFrameSample` values, including a remote `auki-network` subscription mapped by the application. Both paths use the same cadence/provenance/output pipeline: they apply the Detection Log's `EveryFrame` or timestamp-based `Periodic` cadence, invoke the detector, stamp results with the exact input sensor hash, and append them to the Detection Log.

The live streaming runner keeps ingestion on Tokio and runs the synchronous detector plus Detection Log writes on a dedicated blocking worker. It retains one pending frame: when the detector is slower than the stream, a newer frame replaces the stale pending frame and `dropped_frames()` reports the replacement. This latest-wins policy bounds latency and memory without blocking unrelated SDK networking and control-plane tasks. `DetectorTask` deliberately retains exhaustive replay semantics.

The streaming runner remains transport-neutral, preserving this crate's network-free dependency boundary. A network consumer maps each successful `StreamEntry<CameraFrame>` to `CameraFrameSample { timestamp_ns, frame: Arc::new(payload) }` and maps `StreamError` to a string. Dropping `StreamingDetectorTask` requests cancellation; `shutdown().await` performs an observed shutdown and reports ingestion or worker errors. A currently executing detector call is allowed to finish, while pending work is discarded.

`CameraFrameHub::new(capacity)` provides bounded fanout when a viewer, cache, and multiple detectors share one network subscription. Samples hold `Arc<CameraFrame>`, so fanout does not copy image bytes. Slow subscribers skip overwritten frames rather than blocking the publisher; `lagged_frames()` exposes the aggregate drop count. Keep the hub alive across transport reconnects so detector instances can remain subscribed while the network supervisor replaces the underlying subscription.

Detector crates may expose a typed application-facing adapter around `RegisteredCameraDetector`, as the QR reference crate does. `DetectorInstanceSpec::rolling(instance_id, cadence, retention, segment_duration)` contains only choices the application actually owns. The package derives the detector reference from its registered implementation and derives the input log, sensor, and clock references from the selected `SensorLogHandle`.

Remote detector inputs do not require materialization. Their Detection Log manifest binds the remote `LogRef`, Sensor Registry reference, and clock exactly as a local input does; only the frame transport differs.

Log spec types (`SensorLogSpec`, `PoseLogSpec`, `TimeTransformLogSpec`, `DetectionLogSpec`, `MapLogSpec`) and `HeadSpec` (`Rolling { retention_ns }` / `Fixed`) live in `auki_session::log_specs`.

### Materialization stubs (Phase 5, not yet implemented)

- `Session::materialize_remote_log(log_ref, retention, segment_duration)` (async) — currently returns `MaterializationError::NotImplemented`.
- `Session::resolve_static_transform(log_ref)` (async) — reads a sealed one-sample pose log. Currently returns `MaterializationError::NotImplemented`.

### Catalog and domain — moved out in #282

`Session::catalog()`, `Session::join_domain()`, `Session::leave_domain()`, and `Session::cluster_manager()` no longer exist. The equivalents live in [`auki-domain`](../auki-domain): `catalog_of(&peer, &session)` (pure, no network), `Domain::builder(...).authority(...).join()`, `Domain::catalog()`, and `Domain::leave()`.

## Depends on

- [`auki-registry`](../auki-registry) — entry types and `RegistryRef` / `LogRef`.
- [`auki-manifests`](../auki-manifests) — manifest builders and `PoseSource` / `PoseWriterMode` / `TimeTransformSource`.
- [`auki-logs`](../auki-logs) + [`auki-datatypes`](../auki-datatypes) — log primitive + payload types (`SpatialTransform` stub return type).
- [`auki-identity`](../auki-identity), [`auki-time`](../auki-time), [`auki-hash`](../auki-hash), [`auki-jcs`](../auki-jcs).

Deliberately **not** here: `auki-network` and `auki-domain`. The dependency points the other way — `auki-domain` consumes `Peer::registries()` + `Session::logs()` through its catalog bridge (#282 dependency inversion).
