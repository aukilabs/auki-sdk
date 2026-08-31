# Quickstart

Choose the path you actually need:

- **Authenticated networking:** follow the maintained
  [P2P getting started guide](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/getting-started.md).
  It runs two relay-reachable Rust peers using the same portable echo protocol
  used by the Web demo.
- **Local recording:** continue below to create registries, a session, and a
  peer-owned log without starting any network runtime.
- **Protocol authoring:** use the
  [portable protocol guide](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/authoring-protocols.md).

Python and Swift do not yet expose the canonical authenticated `AukiPeer`
facade. Python component bindings still support the local recording flow.

## Local recording in Rust

`auki_session::Peer` owns durable registries. A `Session` is one recording
timeline with its own clocks and logs.

```rust
use std::time::Duration;
use auki_registry::{Camera, SensorBody};
use auki_session::{FrameDef, HeadSpec, Peer, SensorLogSpec};

let peer = Peer::new("galbot-01", "galbot-ctrl")
    .with_storage_root("/data/auki/galbot-01".into());

let frame = peer.register_frame(
    "head_left_camera_optical",
    FrameDef::ros_optical(),
)?;
let sensor = peer.register_sensor(
    "head_left_rgb",
    SensorBody::Camera(Camera {
        r#type: "rgb".into(),
        width: 1920,
        height: 1200,
        frame_rate_hz: 30,
        image_encoding: "raw".into(),
        pixel_format: "rgb8".into(),
        row_stride_bytes: 1920 * 3,
        color_space: "srgb".into(),
        intrinsics_model: "pinhole".into(),
        distortion_model: "brown_conrady".into(),
        calibration: None,
        frame: frame.clone(),
    }),
)?;

let session = peer.start_session()?;
let log = session.register_sensor_log(SensorLogSpec {
    sensor,
    clock: session.monotonic_clock(),
    frame: Some(frame),
    head: HeadSpec::Rolling {
        retention_ns: 5_000_000_000,
    },
    segment_duration: Duration::from_secs(1),
    retention: Duration::from_secs(5),
})?;

println!("{}", log.log_ref().resource_id);
```

This writes canonical registry entries and a log manifest. It does not
authenticate, book a relay, publish routes, or serve a protocol.

The complete checked example is
[`crates/auki-session/tests/end_to_end.rs`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-session/tests/end_to_end.rs).

## Local recording in Python

```python
from auki_session import FrameDef, HeadSpec, Peer, SensorLogSpec

peer = Peer("galbot-01", "galbot-ctrl").with_storage_root(
    "/data/auki/galbot-01"
)
frame = peer.register_frame(
    "head_left_camera_optical", FrameDef.ros_optical()
)
sensor = peer.register_sensor(
    "head_left_rgb",
    {
        "kind": "camera",
        "type": "rgb",
        "width": 1920,
        "height": 1200,
        "frame_rate_hz": 30,
        "image_encoding": "raw",
        "pixel_format": "rgb8",
        "row_stride_bytes": 1920 * 3,
        "color_space": "srgb",
        "intrinsics_model": "pinhole",
        "distortion_model": "brown_conrady",
        "frame": frame,
    },
)
session = peer.start_session()
clock = session.register_clock(
    "sdk_clock",
    {
        "type": "monotonic_clock",
        "unit": "ns",
        "monotonic": True,
        "scope": "device-local",
    },
)
log = session.register_sensor_log(
    SensorLogSpec(
        sensor=sensor,
        clock=clock,
        frame=frame,
        head=HeadSpec.rolling(5_000_000_000),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
)
print(log.log_ref.source_peer_id, log.log_ref.resource_id)
```

The Python session binding is local-only. A canonical Python `AukiPeer` facade
is pending.

## Put a Rust session on the network

Native Rust can connect the two owners without merging them:

1. Start `auki_sdk::AukiPeer` through `auki-auth`.
2. Build `auki_protocols::session_adapter::SessionProtocolProvider` from the
   exact local `Peer` and `Session`.
3. Mount `CatalogEndpoint` and, when needed, `StreamEndpoint` on
   `network_peer.protocols()`.
4. Keep each endpoint alive while serving and close it before
   `network_peer.shutdown()`.

The adapter projects local logs into the live Catalog v3 response. Existing
sensor, pose, time-transform, and detection rows keep their v2 JSON shape
inside that v3 response; the v2 catalog itself is compatibility wire data and
is not mounted. Map Logs use Catalog v4.

Applications still own visibility and capability policy. An authenticated peer
is not automatically allowed to read every log or operate a robot.

## Next steps

- [Concept: peer-owned logs](Concept-Peer-Owned-Logs)
- [Auki P2P mental model](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/README.md)
- [Web echo demo](https://github.com/aukilabs/auki-sdk/tree/develop/examples/portable-echo/web)

---

[← Back to: For SDK Consumers](For-SDK-Consumers) · [Concept: Peer-Owned Logs →](Concept-Peer-Owned-Logs)
