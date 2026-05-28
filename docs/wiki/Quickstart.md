# Quickstart

Boot an `auki-session` peer, declare its sensors / clocks / frames, register a sensor log, and inspect what the SDK writes to the catalog and to disk. ~10 minutes.

This page covers what the Session API exposes **today** (SDK v0.0.55). The remaining pieces — publishing frames into the log, joining a domain so other peers can see your catalog, materializing a remote peer's log — are tracked separately; see [Next steps](#next-steps).

## What you'll build

A single peer that:

- declares its peer / app / session identity
- registers a reference frame, a camera sensor, and a clock
- registers a sensor log (one peer-owned data product)
- emits a catalog row for that log
- writes a manifest to disk

## Install

### Rust

In `Cargo.toml`:

```toml
[dependencies]
auki-session  = { git = "https://github.com/aukilabs/auki-sdk.git", tag = "v0.0.55" }
auki-registry = { git = "https://github.com/aukilabs/auki-sdk.git", tag = "v0.0.55" }
```

### Python

From source for now (PyPI publishing is on the roadmap):

```bash
pip install "auki-session @ git+https://github.com/aukilabs/auki-sdk.git@v0.0.55#subdirectory=bindings/python/auki-session-py"
```

## Boot a session

A `Session` is constructed with the three identities a peer needs to answer "who am I":

- `peer_id` — your network identity. Stable across boots for this device.
- `app_id` — the app running on this peer (e.g. `galbot-ctrl` vs `galbot-teleop`).
- `session_id` — fresh ULID generated per `Session` instance. Filled in by the SDK.

The storage root is where the SDK writes its registry entries and log manifests.

**Rust**

```rust
use auki_session::Session;

let s = Session::new("galbot-01", "galbot-ctrl")
    .with_storage_root("/data/auki/galbot-01");

println!("session_id: {}", s.session_id());
```

**Python**

```python
from auki_session import Session

s = Session("galbot-01", "galbot-ctrl").with_storage_root("/data/auki/galbot-01")
print("session_id:", s.session_id)
```

## Register a reference frame

A sensor log carries spatial data, so the data needs to be expressed in some frame of reference. The SDK ships four presets covering the common conventions:

```
FrameDef::ros_body()      // x forward, y left, z up
FrameDef::ros_optical()   // x right, y down, z forward (camera-frame default)
FrameDef::opengl()        // x right, y up, z back
FrameDef::unity()         // x right, y up, z forward (left-handed)
```

**Rust**

```rust
use auki_session::FrameDef;

let frame = s.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap();
```

**Python**

```python
from auki_session import FrameDef

frame = s.register_frame("head_left_camera_optical", FrameDef.ros_optical())
```

The return value is a `RegistryRef { peer_id, id, hash }` — pass that to anything that needs to reference this frame.

## Register a sensor

The sensor declares what the data looks like (camera intrinsics, format, etc.). The closed enum `SensorKind` (`Camera`, `Imu`, ...) keeps the family tight; the open `type` string differentiates within a family.

**Rust**

```rust
use auki_registry::{Camera, SensorBody};

let sensor = s.register_sensor("head_left_rgb", SensorBody::Camera(Camera {
    r#type: "rgb".into(),
    width: 1920,
    height: 1200,
    frame_rate_hz: 30,
    pixel_format: "rgb8".into(),
    color_space: "srgb".into(),
    intrinsics_model: "pinhole".into(),
    distortion_model: "brown_conrady".into(),
    frame: frame.clone(),
})).unwrap();
```

**Python**

```python
sensor = s.register_sensor("head_left_rgb", {
    "kind": "camera",
    "type": "rgb",
    "width": 1920,
    "height": 1200,
    "frame_rate_hz": 30,
    "pixel_format": "rgb8",
    "color_space": "srgb",
    "intrinsics_model": "pinhole",
    "distortion_model": "brown_conrady",
    "frame": {"peer_id": frame.peer_id, "id": frame.id, "hash": frame.hash},
})
```

## Register a clock

Every timestamp the SDK records is qualified by a named clock, so consumers can transform between clocks rather than assuming a canonical one.

**Rust**

```rust
use auki_registry::{ClockBody, ClockMeta, Scope};

let clock = s.register_clock("session/sdk_clock", ClockBody::MonotonicClock(ClockMeta {
    unit: "ns".into(),
    monotonic: true,
    epoch: None,
    scope: Scope::DeviceLocal,
})).unwrap();
```

**Python**

```python
clock = s.register_clock("session/sdk_clock", {
    "type": "monotonic_clock",
    "unit": "ns",
    "monotonic": True,
    "scope": "device-local",
})
```

## Register a sensor log

The log is the actual peer-owned data product. The spec ties together the sensor, the clock its samples are stamped against, the frame the data lives in, and a head window that controls retention.

**Rust**

```rust
use std::time::Duration;
use auki_session::{HeadSpec, SensorLogSpec};

let log = s.register_sensor_log(SensorLogSpec {
    sensor: sensor.clone(),
    clock,
    frame: Some(frame),
    head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
    segment_duration: Duration::from_secs(1),
    retention: Duration::from_secs(5),
}).unwrap();

println!("log_ref: {}/{}", log.log_ref().source_peer_id, log.log_ref().resource_id);
```

**Python**

```python
from auki_session import HeadSpec, SensorLogSpec

log = s.register_sensor_log(SensorLogSpec(
    sensor=sensor,
    clock=clock,
    frame=frame,
    head=HeadSpec.rolling(5_000_000_000),
    segment_duration_ns=1_000_000_000,
    retention_ns=5_000_000_000,
))
print("log_ref:", log.log_ref.source_peer_id, "/", log.log_ref.resource_id)
```

## Inspect the catalog

`Session::catalog()` returns one `ResourceEntry` row per registered log, in the `/auki/resources/0.2.0` wire shape. This is the same payload a peer publishes over the network so others can discover what it owns.

**Rust**

```rust
for row in s.catalog() {
    println!("{} owns {} ({})", row.source_peer_id, row.resource_id, row.state);
}
```

**Python**

```python
for row in s.catalog():
    print(f"{row['source_peer_id']} owns {row['resource_id']} ({row['state']})")
```

Both print:

```
galbot-01 owns head_left_rgb (live)
```

## Inspect the manifest on disk

```bash
$ cat /data/auki/galbot-01/logs/galbot-01/head_left_rgb/manifest.json
{
  "source_peer_id": "galbot-01",
  "writer_peer_id": "galbot-01",
  "sensor": { ... },
  "clock": { ... },
  ...
}
```

`source_peer_id == writer_peer_id` says this is a locally-written, locally-owned log. When another peer materializes a copy, the materialized manifest preserves `source_peer_id == "galbot-01"` and sets `writer_peer_id` to the materializing peer — ownership stays with the source. See [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs).

## Next steps

The Session API doesn't yet expose, at this layer:

- **Publishing frames into the log.** A `SensorLogHandle::append`-style surface is planned. The underlying `auki-logs::Log::append` exists; lifting it onto the Session handle is the natural next step.
- **Joining a domain so other peers see your catalog.** `Session::join_domain` works in Rust but requires building a libp2p swarm yourself; the Python binding currently raises `NotImplementedError`.
- **Materializing a remote peer's log.** `Session::materialize_remote_log` returns `NotImplementedError` — this is the Phase 5 deliverable in the #216 plan.

This page will grow to cover each as it lands.

## Worked example

The full working version of everything above lives in [`crates/auki-session/tests/end_to_end.rs`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-session/tests/end_to_end.rs). Copy it, swap the `peer_id`, and `cargo test`.

For the Python equivalent, see [`bindings/python/auki-session-py/python_tests/test_session.py`](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-session-py/python_tests/test_session.py) — specifically `test_register_sensor_log_end_to_end` and `test_catalog_resource_id_and_shape`.

---

[← Back to: For SDK Consumers](For-SDK-Consumers) · [Concept: Peer-Owned Logs →](Concept-Peer-Owned-Logs)
