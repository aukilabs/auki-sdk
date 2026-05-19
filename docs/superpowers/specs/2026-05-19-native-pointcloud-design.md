# Native Auki Pointcloud Design

Status: approved design for implementation planning.

Date: May 19, 2026.

## Goal

Refactor the SDK pointcloud contract from a ROS-shaped live stream into a native Auki pointcloud record that works uniformly for robot, ARKit, browser, and future producers.

The demo target is a lockstep SDK + Boosterapp + Park rebuild. Backwards compatibility with the current ROS `PointCloud2` CDR stream is explicitly out of scope.

## Current State

Today the pointcloud surface is split:

- `auki.point_cloud.PointCloudLogEntry { bytes data = 1 }` is used for on-disk sensor logs. It is conceptually generic, with interpretation coming from `SensorBody::PointCloud`.
- `auki.point_cloud_stream.PointCloudFrame { bytes bytes = 1 }` is used on `/auki/stream/0.1.0`. Its contract is documented as raw CDR-encoded ROS `sensor_msgs/PointCloud2`.
- `SensorBody::PointCloud` carries `fields`, `point_step`, `is_bigendian`, `frame_rate_hz`, `frame_id`, and `frame_hash`.

The problem is not that the bytes are packed. The problem is that the live stream contract makes ROS CDR the hidden center of the SDK pointcloud model.

## Design

Use one native pointcloud record for both logs and streams:

```proto
package auki.point_cloud;

message PointCloudFrame {
  uint32 point_count = 1;
  bytes data = 2;
}
```

Retire `PointCloudLogEntry` and `auki.point_cloud_stream.PointCloudFrame`. `auki-network` should re-export `auki_datatypes::point_cloud::PointCloudFrame`, and `StreamDispatch::AcceptPointCloud` should keep its public role while switching to the native payload.

`SensorBody::PointCloud` becomes the reusable point layout header:

- `fields: Vec<PointField>`
- `point_step: u32`
- `frame_rate_hz: u32`
- `frame_id: String`
- `frame_hash: String`

Remove `is_bigendian`. Auki-native pointcloud numeric fields are always little-endian. Producers with big-endian source data convert at the adapter boundary.

Every Auki-native pointcloud layout must begin with:

- `x`: `float32`, count `1`, offset `0`
- `y`: `float32`, count `1`, offset `4`
- `z`: `float32`, count `1`, offset `8`

Extra fields are allowed after the canonical XYZ prefix. The SDK should bless a small v1 vocabulary while still allowing custom field names:

- `r`, `g`, `b`, `a`: `uint8`
- `confidence`: `float32`
- `intensity`: `float32`
- `classification`: `uint16`

Unknown fields are valid if they declare `name`, `datatype`, `count`, and `offset`, but consumers only promise first-class behavior for canonical names.

## Data Flow

Producer flow:

1. A producer receives a pointcloud sample from ROS, ARKit, or another sensor source.
2. The producer or adapter packs native point records as little-endian binary data.
3. The producer emits `PointCloudFrame { point_count, data }`.
4. The stream manifest points to the pointcloud `sensor_id + sensor_hash` and committed `frame_id + frame_hash`.
5. Park fetches the resource and registry metadata, opens the pointcloud stream, and renders by stepping through `data` with `point_step`.

For ROS, `auki-ros-adapter` converts `PointCloud2` into the native packed layout. ROS is an adapter input, not the SDK stream contract.

For ARKit later, the iPhone bridge will pack ARKit feature points or scene-depth points directly into the same `PointCloudFrame`.

For logs, the same `PointCloudFrame` is written into the sensor log. Stream capture and disk replay use the same record shape.

## Validation And Errors

The generic stream pump remains type-agnostic and fast. It should continue to move protobuf payloads without resolving pointcloud registry details.

Pointcloud validation happens at helper and adapter boundaries where both the registry entry and frame are available:

- registry validation requires canonical XYZ fields with the required type, count, and offsets;
- `point_step` must be at least `12`;
- fields must not overlap;
- every field must fit inside `point_step`;
- frame validation requires `data.len() == point_count * point_step`.

Adapter failures should return typed errors before producing a frame. Consumers that encounter invalid registry/frame pairs should treat the stream as a producer error and stop rendering or decline the stream.

## Implementation Scope

In scope:

- `auki-datatypes`: replace `PointCloudLogEntry` with shared `PointCloudFrame`; remove `point_cloud_stream.proto`; update generated module exports and locked vectors.
- `auki-registry`: remove `PointCloud.is_bigendian`; add pointcloud layout validation for canonical XYZ and field bounds.
- `auki-network`: re-export `auki_datatypes::point_cloud::PointCloudFrame`; keep `AcceptPointCloud` but make it native; update tests and resource payload naming.
- `auki-domain`: update resource payload mapping and docs to describe native pointcloud frames.
- `auki-network-py` and `auki-domain-py`: expose `PointCloudFrame(point_count, data)` so Python producers can emit native frames.
- `auki-ros-adapter`: convert ROS `PointCloud2` into native `PointCloudFrame`.
- docs, parking lots, and changelogs: call out the intentional wire break and native pointcloud invariant.
- tests: proto locked vectors, registry validation tests, stream runtime pointcloud end-to-end tests, Python surface tests, and ROS adapter conversion tests.

Out of scope:

- backwards-compatible ROS CDR pointcloud streaming;
- rich point iterator and accessor APIs;
- generic pointcloud math;
- iOS or Swift bridge work;
- Park or Boosterapp implementation changes inside this repository.

## Demo Readiness

This refactor is sufficient for an end-to-end demo after SDK consumers are rebuilt in lockstep:

- Boosterapp emits native `PointCloudFrame { point_count, data }` using the updated Python/Rust SDK surface.
- Park resolves the pointcloud sensor registry, reads `point_step` and fields, and renders the canonical XYZ prefix from the frame data.
- Existing resource and stream discovery surfaces remain the route by which Park finds and subscribes to the producer.

The expansive helper layer can come later. The demo only needs the native record contract, strict layout validation, and the ROS adapter conversion path.
