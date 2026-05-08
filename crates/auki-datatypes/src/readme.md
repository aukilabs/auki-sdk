# `auki-datatypes/src/`

Implementation status of `auki-datatypes`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It includes three prost-generated modules — `auki.pose`, `auki.joint_state`, `auki.pose_stream` — plus inline tests covering encode/decode round-trips and locked cross-language conformance vectors for each.

The placeholder package is gone — its job (validating the prost-build pipeline) is now done by the real schemas.

## Real schemas defined today (sawslin Phase 1 Lane 0 / PR B)

```rust
pub mod joint_state {
    // auki.joint_state — articulated-joint angle vector (boosterapp's
    // PoseStream payload from sawslin Phase 1; same shape on disk for
    // SensorBody::JointState entries).
    pub struct JointAngles { pub angles: Vec<f32> }
}

pub mod pose {
    // auki.pose — canonical 6-DoF spatial transform primitives.
    pub struct Vec3 { pub x: f32, pub y: f32, pub z: f32 }
    pub struct Quat { pub x: f32, pub y: f32, pub z: f32, pub w: f32 }
    pub struct SpatialTransform {
        pub translation: Option<Vec3>,
        pub orientation: Option<Quat>,
    }
}

pub mod pose_stream {
    // auki.pose_stream — wire envelope for the AcceptPoseStream
    // dispatch variant (sawslin locked decision #7). One oneof carries
    // both joint-angle and spatial-transform shapes; each substream
    // stays mono-shape in practice, but the dispatch variant is the
    // same for both.
    pub struct PoseStreamFrame {
        pub payload: Option<pose_stream_frame::Payload>,
    }
    pub mod pose_stream_frame {
        pub enum Payload {
            JointAngles(super::super::joint_state::JointAngles),
            SpatialTransform(super::super::pose::SpatialTransform),
        }
    }
}
```

These are the **first** real schemas to land. The remaining migration steps (Steps 1, 2, 3, 4, 6 in [`sprint.md`](sprint.md) — `auki.camera`, `auki.point_cloud`, `auki.audio`, libp2p stream types, time-transform) still need to move out of [`auki-registry`](../../auki-registry) and [`auki-network`](../../auki-network)'s `stream_protocol`.

## What's not here yet

Per [`sprint.md`](sprint.md):

- `auki.camera` — `PinholeCameraLogEntry` (renamed from `SensorLogEntry`)
- `auki.point_cloud` — `PointCloudLogEntry`
- `auki.audio` — `AudioLogEntry`
- `auki.time_transform` — `TimeTransformEntry`
- `auki.frame_stream` — `JpegFrame` (libp2p wire)
- `auki.point_cloud_stream` — `PointCloudFrame` (libp2p wire)

Sawslin queue-jumped pose / joint_state / pose_stream past these to avoid landing temporary types that would need to be renamed later. See [`sprint.md`](sprint.md) "Sawslin queue-jump" for the rationale; each remaining step still **moves** a hand-written serde-derived type currently in [`auki-registry`](../../auki-registry) or [`auki-network`](../../auki-network)'s `stream_protocol` into here.

## Tests

| Test | Asserts |
|------|---------|
| `joint_angles_round_trips` | `JointAngles { angles: [..] }` survives prost encode/decode |
| `spatial_transform_round_trips` | Round-trip with non-trivial Vec3 + Quat |
| `pose_stream_frame_round_trips_joint_angles_arm` | Envelope + JointAngles oneof arm |
| `pose_stream_frame_round_trips_spatial_transform_arm` | Envelope + SpatialTransform oneof arm |
| `empty_pose_stream_frame_round_trips_as_neither_arm_set` | Wire-level: empty `payload: None` decodes cleanly (consumer-side malformed-frame policy) |
| `locked_joint_angles_wire_bytes` | Pin specific `[0.0, 0.5, -0.5, 1.0, -1.0]` → 22-byte hex |
| `locked_spatial_transform_wire_bytes` | Pin `Vec3{1,2,3}/Quat{0,0,0,1}` (note: proto3 omits zero-default Quat fields) |
| `locked_pose_stream_frame_joint_angles_arm_wire_bytes` | Oneof field number 1 (`JointAngles` arm) |
| `locked_pose_stream_frame_spatial_transform_arm_wire_bytes` | Oneof field number 2 (`SpatialTransform` arm) |
| `print_locked_pose_vectors` | Debug helper — prints hex of each locked vector for schema-evolution work |

The locked vectors serve as cross-language regression guards. Any reimplementation in another language (Python via `betterproto`, future Sentinel ports, future iOS / ARKit bindings) must reproduce these exact bytes from the same input.

## Consumers

- [`auki-network`](../../auki-network) — `stream_protocol::PoseStreamFrameWire` wraps prost-encoded `PoseStreamFrame` bytes inside the existing length-prefixed JSON framing for the `AcceptPoseStream` dispatch variant. (Transitional adapter — see [`sprint.md`](sprint.md) Step 2 for the eventual native-prost framing.)
- [`auki-network-py`](../../auki-network-py) — `PyJointAngles`, `PyVec3`, `PyQuat`, `PySpatialTransform` Python wrappers; `StreamDecision.accept_pose_stream(...)` and `runtime.open_pose_stream(...)` route through the typed payloads, hiding the wire envelope from the Python caller.

The remaining migration steps will pick up `auki-logs`, `auki-ros-adapter`, and `auki-time-transforms` as each downstream crate moves onto its generated types.
