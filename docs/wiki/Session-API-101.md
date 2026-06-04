# Session API 101

This page expands on the `Session` API surface introduced in the Quickstart and crate README. It fills the documentation gaps around complex areas, full log type coverage, error cases, and Phase 5 stubs. It assumes you have read the Quickstart.

**Target audience:** SDK integrators who need to register multiple log types, handle errors robustly, or understand domain joining and materialization before they are fully wired.

**Status:** Covers SDK v0.0.55 surface. Publishing (`append` on handles) and full materialization are tracked separately.

## Session Construction and Identity

(See Quickstart for `Session::new` + `with_storage_root`.)

Additional notes:

- `storage_root` is the base directory. The SDK creates `registries/`, `logs/<peer_id>/` subtrees under it. It is safe to share the same root across multiple `Session` instances on the same device (they use distinct `session_id` ULIDs).
- All three identity strings (`peer_id`, `app_id`, `session_id`) are exposed via accessors and appear in manifests and catalog rows.
- `set_storage_root` exists for FFI bindings that cannot use the builder pattern.

## Registry Registration (Sensors, Clocks, Frames, Detectors)

All four `register_*` methods:

- Validate IDs via `auki_registry::validate_registry_id` (rejects `>`, `@`, whitespace).
- Return `RegistryRef { peer_id, id, hash }` — the `hash` is content-addressed so the same logical entry always produces the same `RegistryRef`.
- Write JSON under `registries/<kind>/<peer_id>/<id>/<hash>.json`.

`FrameDef` presets are the primary way to avoid hand-crafting `FrameBody`. The four presets cover 95% of robot / vision use cases.

Example registering a detector (the least-documented of the four):

```rust
use auki_session::{Session, DetectorBody, RegistryRef};

let detector = DetectorBody {
    kind: "yolo-v8".into(),
    version: "8.1".into(),
    // ... other fields
};
let det_ref: RegistryRef = s.register_detector("front_yolo", detector, vec!["bbox".into()]).unwrap();
```

Python equivalent uses the same dataclass-style construction.

## Log Registration — All Four Types

`Session` exposes four `register_*_log` methods. Each:

- Takes a `*LogSpec` (defined in `log_specs`).
- Derives a stable `resource_id` from the spec.
- Rejects duplicate `(source_peer_id, resource_id)` with `SessionError::DuplicateLog`.
- Returns a typed handle (`SensorLogHandle`, etc.) that carries `resource_id` and `LogRef` for later use.
- The handle's manifest is used to produce catalog rows; the `HeadSpec` (Rolling vs Fixed) controls retention in the catalog.

### 1. SensorLogSpec

```rust
use auki_session::{SensorLogSpec, HeadSpec, SensorLogHandle};

let spec = SensorLogSpec {
    sensor: sensor_ref,           // RegistryRef from register_sensor
    clock: clock_ref,
    frame: frame_ref,
    head: HeadSpec::Rolling { retention_ns: 30_000_000_000 }, // 30s
    // other fields...
};
let handle: SensorLogHandle = s.register_sensor_log(spec).unwrap();
println!("resource_id: {}", handle.resource_id());
```

`resource_id` == `sensor.id`.

### 2. PoseLogSpec

```rust
let pose_spec = PoseLogSpec {
    from_frame: head_frame_ref,
    to_frame: world_frame_ref,
    clock: clock_ref,
    head: HeadSpec::Fixed,
    // ...
};
let pose_handle: PoseLogHandle = s.register_pose_log(pose_spec).unwrap();
// resource_id == "<from.id>-><to.id>"
```

### 3. TimeTransformLogSpec

```rust
let tt_spec = TimeTransformLogSpec {
    from_clock: wall_clock_ref,
    to_clock: monotonic_clock_ref,
    head: HeadSpec::Rolling { retention_ns: 1_000_000_000 },
    // ...
};
let tt_handle: TimeTransformLogHandle = s.register_time_transform_log(tt_spec).unwrap();
// resource_id == "<from.id>-><to.id>"
```

### 4. DetectionLogSpec

```rust
let det_spec = DetectionLogSpec {
    detector: det_ref,
    input_sensor: rgb_sensor_ref,
    output_type: "bbox".into(),
    head: HeadSpec::Fixed,
    // ...
};
let det_handle: DetectionLogHandle = s.register_detection_log(det_spec).unwrap();
// resource_id == "<detector.id>@<input_sensor.id>"
```

**DuplicateLog error handling**

```rust
match s.register_sensor_log(spec) {
    Ok(h) => { /* use handle */ }
    Err(SessionError::DuplicateLog { source_peer_id, resource_id }) => {
        eprintln!("Log already registered: {}/{}", source_peer_id, resource_id);
        // Common when re-running the same app without clearing storage_root
    }
    Err(e) => return Err(e),
}
```

The error is deliberately specific so consumers can distinguish "I already own this resource" from other failures. The `source_peer_id` is always the original owner even after materialization.

## Catalog

`Session::catalog()` returns the live list of `ResourceEntry` rows in the exact `/auki/resources/0.2.0` wire format. This is what remote peers see when they query your catalog.

Key fields on each row:

- `source_peer_id` — stable owner (never changes on materialization)
- `resource_id` — derived as shown above
- `state` — "live" | "sealed" | ...
- `writer_peer_id` — the peer that last wrote the manifest (local or materializer)
- `last_updated_ns`, `head` window, etc.

Consumers should treat the catalog as a live snapshot and poll/reconcile. Producers should remove rows for logs that cannot currently accept new stream opens.

See also the worked example in Quickstart.

## Domain Joining (join_domain complexity)

`Session::join_domain(DomainConfig)` is async and wires your local catalog into the Auki domain so other peers can discover your resources.

`DomainConfig` is intentionally large because the Session crate does **not** own the networking stack:

```rust
pub struct DomainConfig {
    pub target: ClusterTarget,
    pub local_identity: PeerIdentity,
    pub local_multiaddrs: Vec<Multiaddr>,
    pub discovery_url: String,
    pub swarm: Swarm<Behaviour>,
    pub stream_provider: StreamProvider,
    pub daemon_info: DaemonInfo,
}
```

Typical usage flow (high-level):

1. Generate or load ed25519 keypair → `PeerIdentity`
2. Build libp2p `Swarm` + `Behaviour` (see `auki-network` examples)
3. Create `StreamProvider` for substream handling
4. Fill `DaemonInfo` (name, capabilities, etc.)
5. Choose `ClusterTarget` (create vs join)
6. Call `join_domain`

`leave_domain()` shuts it down cleanly.

Because constructing a real `DomainConfig` requires significant libp2p boilerplate, most applications will use a higher-level helper (e.g. from `auki-bootstrap` or a robot-specific crate) rather than building the config by hand. The surface exists so that advanced users and tests can inject custom swarms.

Python binding currently raises `NotImplementedError` for `join_domain`.

## Materialization Stubs (Phase 5)

Two methods are present on the API surface but not yet implemented:

- `Session::materialize_remote_log(log_ref, retention, segment_duration)` → `MaterializedLogHandle`
- `Session::resolve_static_transform(log_ref)` → `SpatialTransform`

Both currently return `SessionError::Materialization(MaterializationError::NotImplemented)`.

The stubs exist so that application code can be written against the final signatures today. When Phase 5 lands, the same calls will perform remote stream open + local materialization while preserving `source_peer_id`.

`MaterializationError` lives in `materialization.rs` and is re-exported.

Until then, any code path that calls these methods must handle the `NotImplemented` case (or gate behind a feature flag).

## Log Handles and Future Publishing

The four `*LogHandle` types currently expose only identity (`resource_id()`, `log_ref()`) and internal manifest/head data used for catalog production.

A future `SensorLogHandle::append(...)` (and equivalents) will be added once the underlying `auki-logs::Log` is fully integrated. Until then the handles are read-only identity carriers.

## RegistryStore and Internal Details

`RegistryStore` is a thin wrapper around a `HashMap` + JSON persistence. It is re-exported for advanced users who need to inspect or mutate the in-memory registry without going through `Session`.

Most consumers never need it directly.

## Summary of Documentation Gaps Addressed

- join_domain complexity and DomainConfig requirements
- materialize stubs and NotImplemented contract
- DuplicateLog error shape and when it fires
- Complete examples for all four log registration paths
- Catalog row semantics and reconciliation guidance
- Relationship between storage_root, manifests, and peer-owned logs (cross-ref to Concept page)

See also:

- [Quickstart](Quickstart) — minimal end-to-end
- [Crate README](../crates/auki-session/README.md) — full public surface table
- [For SDK Consumers](For-SDK-Consumers) — consumer-side catalog usage
- GitHub issues #275–#279 for known structural improvements to the implementation

---

[← Back to For SDK Consumers](For-SDK-Consumers) · [Concept: Peer-Owned Logs →](Concept-Peer-Owned-Logs)
