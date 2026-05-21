# Stream Naming Cleanup Design

Date: 2026-05-21
Status: Approved for implementation

## Goal

Make the SDK's sensor stream vocabulary domain-level and symmetrical across live streams, retained logs, registries, and bindings. This is a breaking cleanup. There are no compatibility aliases, fallbacks, legacy JSON tags, or old public type names.

## Canonical Vocabulary

SDK stream payloads:

- `CameraFrame`
- `PointCloudFrame`
- `JointEncodersFrame`
- `AudioFrame`
- `DetectionFrame`

SDK producer dispatch:

- `AcceptCamera`
- `AcceptPointCloud`
- `AcceptJointEncoders`
- `AcceptAudio`
- `AcceptDetection`

Sensor registry bodies:

- `Camera`
- `PointCloud`
- `Audio`
- `JointEncoders`

Sensor registry JSON tags:

- `"camera"`
- `"point_cloud"`
- `"audio"`
- `"joint_encoders"`

## Renames

- Legacy JPEG/camera log-entry payload identifiers are consolidated into `CameraFrame`.
- Legacy detection log-entry payload identifiers are consolidated into `DetectionFrame`.
- Legacy RGB-camera registry body identifiers are consolidated into `Camera`.
- The sensor registry tag is `"camera"`.

The old names may remain only in append-only changelog history. They should not remain in current public API, active docs, examples, tests, generated binding surfaces, or Park integration code.

## Semantics

`CameraFrame` is the SDK's camera payload record. It is valid both as a live stream payload and as a retained sensor-log entry. The payload keeps the existing structure: optional dynamic intrinsics plus frame bytes. The type name does not promise JPEG specifically; the registered `Camera` sensor body owns camera metadata such as width, height, pixel format, color space, intrinsics model, distortion model, frame id, and frame hash.

`DetectionFrame` is the SDK's per-frame detector output payload. It is valid both as a live detection stream payload and as a retained detection-log entry. It keeps the existing opaque detector-owned bytes plus source sensor hash and detection type discriminator.

`PointCloudFrame`, `JointEncodersFrame`, and `AudioFrame` already fit the target vocabulary and remain conceptually unchanged.

## Affected Surfaces

- `auki-datatypes` proto messages, generated Rust types, docs, tests, and Python generated bindings.
- `auki-registry` Rust enum/struct names, registry JSON tags, locked JSON/hash tests, docs, and builders.
- `auki-network` stream re-exports, `StreamDispatch`, typed pump tests, locked wire vectors, docs, and Python bindings.
- `auki-domain` and related bindings wherever stream manifests, sensor catalogs, or registry bodies mention camera or detection types.
- `auki-ros-adapter` camera registry/log builders.
- Park's SDK pin branch once the SDK branch is available: live stream consumers, recordings/materialization, route/type matching, and docs should use `CameraFrame` / `"camera"` / `DetectionFrame`.

## Verification

Implementation is complete only when:

- Workspace Rust tests covering changed crates pass.
- Python binding tests for affected generated/binding surfaces pass where available.
- Locked wire-byte tests are updated intentionally, not accidentally.
- Locked registry JSON/hash tests now pin `"camera"`.
- A repository-wide search finds no active legacy camera/detection payload names or RGB-camera registry names outside append-only changelog history.
- Park builds against the renamed SDK surface after its dependency is moved to the fixed SDK revision/tag.
