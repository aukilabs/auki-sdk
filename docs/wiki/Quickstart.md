# Quickstart

Boot an `auki-session` peer, declare its sensors / clocks / frames, register a sensor log, and inspect what the SDK writes to the catalog and to disk. ~10 minutes.

This page covers the current Peer / Session split. Publishing frames into some
local log handles and materializing a remote peer's log remain separate work;
authenticated Domain join is available in Rust and Python.

## What you'll build

A single peer that:

- declares its peer / app identity and registers a reference frame and a camera sensor
- starts a session (the SDK mints the session id and the session's clocks)
- registers a sensor log (one peer-owned data product)
- emits a catalog row for that log
- writes a manifest to disk

## Install

Stage 1 is not yet a published tag. The old v0.0.x tags are Manager-era and are
not wire-compatible with this guide. Use the current source checkout for local
evaluation, then move to the coordinated 0.1 release line when it is published.

From the repository root, prove the Rust surface with:

```sh
cargo test --locked -p auki-session --test end_to_end
cargo test --locked -p auki-portable-echo
```

Build the paired Python bindings from that same checkout with the commands in
the [`auki-domain-py` README](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-domain-py/README.md).

For a native Rust experiment that needs real Domain authority, use
[`auki-auth`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-auth):
email/password or a trusted App key/secret selects an accessible DDS Domain and
returns the Peer-ID-bound authority consumed by `DomainBuilder`. Peer routes
and discovery remain a separate host input.

## Construct a peer, start a session

Since #282 the entry point is a long-lived `Peer` that mints `Session`s. The identities split across the two:

- `peer_id` — your network identity, on the `Peer`. Stable across boots for this device. (Any string works for local experiments; to join a Domain later it must equal the Peer ID of the canonical `auki_p2p::Identity`, commonly constructed from `Wallet::derive_child("peer/v1").seed()`.)
- `app_id` — the app running on this peer (e.g. `galbot-ctrl` vs `galbot-teleop`), on the `Peer`.
- `session_id` — fresh ULID minted by `Peer::start_session()`, on the `Session`.

The storage root is where the SDK writes its registry entries and log manifests. Starting a session also auto-registers the session's monotonic + UTC clock pair (#284) — no hand-rolled session clock.

**Rust**

```rust
use auki_session::Peer;

let peer = Peer::new("galbot-01", "galbot-ctrl")
    .with_storage_root("/data/auki/galbot-01".into());

let session = peer.start_session().unwrap();
println!("session_id: {}", session.session_id());
```

**Python**

```python
from auki_session import Peer

peer = Peer("galbot-01", "galbot-ctrl").with_storage_root("/data/auki/galbot-01")
session = peer.start_session()
print("session_id:", session.session_id)
```

## Register a reference frame

A sensor log carries spatial data, so the data needs to be expressed in some frame of reference. Frames are peer-level (they outlive any one session), so registration happens on the `Peer`. The SDK ships four presets covering the common conventions:

```
FrameDef::ros_body()      // x forward, y left, z up
FrameDef::ros_optical()   // x right, y down, z forward (camera-frame default)
FrameDef::opengl()        // x right, y up, z back
FrameDef::unity()         // x right, y up, z forward (left-handed)
```

**Rust**

```rust
use auki_session::FrameDef;

let frame = peer.register_frame("head_left_camera_optical", FrameDef::ros_optical()).unwrap();
```

**Python**

```python
from auki_session import FrameDef

frame = peer.register_frame("head_left_camera_optical", FrameDef.ros_optical())
```

The return value is a `RegistryRef { peer_id, id, hash }` — pass that to anything that needs to reference this frame.

## Register a sensor

The sensor declares what the data looks like (camera intrinsics, format, etc.). Sensors are peer-level too. The closed enum `SensorKind` (`Camera`, `Imu`, ...) keeps the family tight; the open `type` string differentiates within a family.

**Rust**

```rust
use auki_registry::{Camera, SensorBody};

let sensor = peer.register_sensor("head_left_rgb", SensorBody::Camera(Camera {
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
sensor = peer.register_sensor("head_left_rgb", {
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

## Clocks

Every timestamp the SDK records is qualified by a named clock, so consumers can transform between clocks rather than assuming a canonical one.

Since #284 you usually don't register a clock yourself: `start_session()` already registered the session's monotonic + UTC pair, and Rust code grabs them directly:

**Rust**

```rust
let clock = session.monotonic_clock(); // RegistryRef to the auto-minted session clock
```

The Python binding doesn't expose the `monotonic_clock()` / `utc_clock()` getters yet, so from Python register an additional session-scoped clock (also the path for custom clocks in Rust, via `session.register_clock(...)`):

**Python**

```python
clock = session.register_clock("session/sdk_clock", {
    "type": "monotonic_clock",
    "unit": "ns",
    "monotonic": True,
    "scope": "device-local",
})
```

## Register a sensor log

The log is the actual peer-owned data product, and it belongs to the session. The spec ties together the sensor, the clock its samples are stamped against, the frame the data lives in, and a head window that controls retention.

**Rust**

```rust
use std::time::Duration;
use auki_session::{HeadSpec, SensorLogSpec};

let log = session.register_sensor_log(SensorLogSpec {
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

log = session.register_sensor_log(SensorLogSpec(
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

`auki_domain::catalog_of(&peer, &session)` returns one `ResourceEntry` row per
registered log in the `/auki/auth/1/resources/0.2.0` payload shape — pure and
network-free. A default `DomainBuilder` installs this snapshot provider.
It still serves no protocol by default: select
`ServedProtocols::none().with_resources_v2()` when remote peers should fetch
that catalog. Client operations do not require a matching inbound selection.

Over the network, `/auki/auth/1/resources/0.2.0` is a live snapshot of resources that
are currently requestable. A peer may join before every producer is ready.
Consumers should poll and reconcile rows that appear or disappear; producers
should omit resources that cannot currently accept stream opens and re-add the
same stable `resource_id` when they recover.

**Rust**

```rust
for row in auki_domain::catalog_of(&peer, &session) {
    println!("{} owns {} ({})", row.source_peer_id, row.resource_id, row.state);
}
```

Prints:

```
galbot-01 owns head_left_rgb (live)
```

**Python** — once an authenticated `auki_domain.Domain` is joined, its live
local provider is available through the same owner:

```python
for row in domain.catalog():
    print(row.source_peer_id, "owns", row.resource_id, f"({row.state})")
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

The Peer / Session API doesn't yet expose, at this layer:

- **Publishing frames into the log.** A `SensorLogHandle::append`-style surface is planned. The underlying `auki-logs::Log::append` exists; lifting it onto the session handle is the natural next step.
- **Materializing a remote peer's log.** `Session::materialize_remote_log` returns `NotImplementedError` — this is the Phase 5 deliverable in the #216 plan.

This page will grow to cover each as it lands.

## Worked example

The full working version of everything above lives in [`crates/auki-session/tests/end_to_end.rs`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-session/tests/end_to_end.rs). Copy it, swap the `peer_id`, and `cargo test`.

For the Python equivalent, see [`bindings/python/auki-session-py/python_tests/test_session.py`](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-session-py/python_tests/test_session.py) — specifically `test_register_sensor_log_end_to_end` and `test_catalog_resource_id_and_shape`.

To put that Peer/Session pair on the network, continue with the
[authenticated Domain migration guide](https://github.com/aukilabs/auki-sdk/blob/develop/docs/authenticated-domain-migration.md)
and the [`auki-domain-py` README](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-domain-py/README.md).

---

[← Back to: For SDK Consumers](For-SDK-Consumers) · [Concept: Peer-Owned Logs →](Concept-Peer-Owned-Logs)
