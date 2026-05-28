# auki-session-py

PyO3 bindings for [`auki-session`](../../../crates/auki-session) — Python surface for the Auki SDK's declarative control-plane API. Shipped in #224 alongside the Rust `auki-session` crate.

Both the Rust crate and the Python binding are live and tested (23 passing Python tests in `tests/`).

**Status:** Shipped.

## Public surface

### Session class

```python
from auki_session import Session, FrameDef, HeadSpec, SensorLogSpec

s = Session("galbot", "galbot-ctrl").with_storage_root("/data/auki")
```

- `Session(peer_id, app_id)` — constructor; generates a ULID `session_id`.
- `with_storage_root(path)` — in-place builder. Mutates the session's storage root and returns `self` for chaining. **Preserves `session_id`** — calling it after the constructor does not regenerate the ULID. Under the hood, calls Rust's `Session::set_storage_root` (the binding-friendly sibling of `Session::with_storage_root(self, root) -> Self`).
- Read accessors: `peer_id`, `app_id`, `session_id`, `storage_root`.

### Registry registration

Each returns a `RegistryRef` instance (`peer_id`, `id`, `hash`). IDs must not contain `>`, `@`, or whitespace.

- `register_sensor(sensor_id, body_dict)` — `body_dict` has `"kind"` and `"type"` fields (e.g. `{"kind": "camera", "type": "rgb", ...}`).
- `register_clock(clock_id, body_dict)`.
- `register_frame(frame_id, FrameDef)` — takes a `FrameDef` preset object.
- `register_detector(detector_id, body_dict, output_types: list[str])`.

`FrameDef` classmethods: `FrameDef.ros_body()`, `FrameDef.ros_optical()`, `FrameDef.opengl()`, `FrameDef.unity()`.

### Log registration

Each returns a typed handle with `resource_id` and `log_ref` attributes. Specs take `RegistryRef` instances or dicts.

- `register_sensor_log(SensorLogSpec)` → `SensorLogHandle` — `resource_id` is `sensor.id`.
- `register_pose_log(PoseLogSpec)` → `PoseLogHandle` — `resource_id` is `"<from_frame.id>-><to_frame.id>"`.
- `register_time_transform_log(TimeTransformLogSpec)` → `TimeTransformLogHandle` — `resource_id` is `"<from_clock.id>-><to_clock.id>"`.
- `register_detection_log(DetectionLogSpec)` → `DetectionLogHandle` — `resource_id` is `"<detector.id>@<input_sensor.id>"`.

`HeadSpec` factory methods: `HeadSpec.rolling(retention_ns)`, `HeadSpec.fixed()`.

`LogRef` class: `LogRef(source_peer_id, resource_id)`.

### Catalog

- `catalog()` → `list[dict]` — one `ResourceEntry` dict per registered log, in the `/auki/resources/0.2.0` wire shape.

### Async stubs (raise `NotImplementedError`)

- `join_domain(config)` — not yet supported from Python; requires a pre-built libp2p swarm.
- `leave_domain()` — not yet supported from Python.
- `materialize_remote_log(log_ref, *, retention_ns, segment_duration_ns)` — deferred to Phase 5.
- `resolve_static_transform(log_ref)` — deferred to Phase 5.

## Type sharing

`RegistryRef` and `LogRef` are defined here as pyclasses that mirror the same-named pyclasses in `auki-registry-py`. Both are constructed from the same underlying Rust `auki_registry::{RegistryRef, LogRef}` types. Cross-package PyO3 type sharing was deemed too friction-heavy; objects can be passed between packages through dict form if needed.

## Depends on

- [`auki-session`](../../../crates/auki-session) — Rust crate it wraps.
- [`auki-registry`](../../../crates/auki-registry) — for `RegistryRef` / `LogRef` Rust types.
- [`auki-manifests`](../../../crates/auki-manifests) — for `PoseSource`, `PoseWriterMode`, `TimeTransformSource`.
