# auki-session-py

PyO3 bindings for [`auki-session`](../../../crates/auki-session) — Python surface for the SDK's declarative control-plane API. Shipped in #224; tracks the post-#282 `Peer` / `Session` split.

**Status:** Shipped. Tested in `python_tests/`. Network lifecycle is **not** in
this package; the active [`auki-domain-py`](../auki-domain-py) binding composes
these `Peer` and `Session` objects into the authenticated Rust `Domain` owner.

## Public surface

### Peer class

```python
from auki_session import Peer, FrameDef, HeadSpec, SensorLogSpec

peer = Peer("12D3KooW...", "galbot-ctrl").with_storage_root("/data/auki")
frame = peer.register_frame("head_left_optical", FrameDef.ros_optical())
sensor = peer.register_sensor("head_left_rgb", {"kind": "camera", "type": "rgb", ...})
session = peer.start_session()
```

- `Peer(peer_id, app_id)` — long-lived identity; `peer_id` is the libp2p peer-id string. The peer outlives any one session.
- `with_storage_root(path)` — in-place builder; mutates the peer's storage root and returns `self` for chaining. Under the hood, calls Rust's `Peer::set_storage_root` (the binding-friendly sibling of `Peer::with_storage_root(self, root) -> Self`).
- Read accessors: `peer_id`, `app_id`, `storage_root`.
- `register_sensor(sensor_id, body_dict)` — `body_dict` has `"kind"` and `"type"` fields (e.g. `{"kind": "camera", "type": "rgb", ...}`). Non-spatial measurements use `{"kind": "scalar", "type": "battery_charge", "unit": "percent", "expected_rate_hz": 1}`.
- `register_frame(frame_id, FrameDef)` — takes a `FrameDef` preset object. Classmethods: `FrameDef.ros_body()`, `FrameDef.ros_optical()`, `FrameDef.opengl()`, `FrameDef.unity()`.
- `register_detector(detector_id, body_dict, output_types: list[str], input_types: list[dict] | None = None)`.
- `start_session()` → `Session` — mints a ULID `session_id` and auto-registers the session's monotonic + UTC clocks (`{peer_id}/{session_id}/monotonic` / `…/utc`).

Each `register_*` returns a `RegistryRef` instance (`peer_id`, `id`, `hash`). IDs must not contain `>`, `@`, or whitespace.

### Session class

Sessions are born from `peer.start_session()` — there is no Python `Session` constructor.

- Read accessors: `peer_id`, `app_id`, `session_id`, `storage_root`.
- `register_clock(clock_id, body_dict)` — additional session-scoped clocks.

Not yet exposed: the Rust `Session::monotonic_clock()` / `utc_clock()` getters for the auto-minted clock pair — Python apps that need a clock `RegistryRef` for a log spec register their own via `register_clock`.

### Log registration

Each returns a typed handle with `resource_id`, `log_ref`, and canonical session-scoped `root` attributes. Specs take `RegistryRef` instances or dicts.

- `register_sensor_log(SensorLogSpec)` → `SensorLogHandle` — `resource_id` is `sensor.id`. Set `frame=None` for Scalar sensors.
- `register_pose_log(PoseLogSpec)` → `PoseLogHandle` — `resource_id` is `"<from_frame.id>-><to_frame.id>"`.
- `register_time_transform_log(TimeTransformLogSpec)` → `TimeTransformLogHandle` — `resource_id` is `"<from_clock.id>-><to_clock.id>"`.
- `register_detection_log(DetectionLogSpec)` → `DetectionLogHandle` — `resource_id` is the spec's application-selected `instance_id`; the spec also carries its `cadence`.

`HeadSpec` factory methods: `HeadSpec.rolling(retention_ns)`, `HeadSpec.fixed()`.

`LogRef` class: `LogRef(source_peer_id, resource_id)`.

### Async stubs (raise `NotImplementedError`)

- `materialize_remote_log(log_ref, *, retention_ns, segment_duration_ns)` — deferred to Phase 5.
- `resolve_static_transform(log_ref)` — deferred to Phase 5.

### Catalog and domain — not here

`catalog()` and the former Session-level network lifecycle were removed with
the #282 split (they no longer exist on the Rust `Session` either). Resource
catalogs and network lifecycle are exposed by the separate active
[`auki-domain-py`](../auki-domain-py) binding.

## Type sharing

`RegistryRef` and `LogRef` come from [`auki-registry-py`](../auki-registry-py). This package re-exports them in the `auki_session` namespace so callers can import from either package. Input parsing is duck-typed: any object with the right field names (including `auki_registry.RegistryRef` instances, plain dicts, or `SimpleNamespace`) is accepted.

## Depends on

- [`auki-session`](../../../crates/auki-session) — Rust crate it wraps.
- [`auki-registry`](../../../crates/auki-registry) — for `RegistryRef` / `LogRef` Rust types.
- [`auki-registry-py`](../auki-registry-py) — source-of-truth for `RegistryRef` / `LogRef` pyclasses.
- [`auki-manifests`](../../../crates/auki-manifests) — for `PoseSource`, `PoseWriterMode`, `TimeTransformSource`.
