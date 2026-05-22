# auki-ros-adapter

Generic ROS2 → SDK glue. Converts `sensor_msgs/CameraInfo` + `sensor_msgs/Image` into Sensor Registry entries and Sensor Log payloads; same for `sensor_msgs/PointCloud2`, with RGB / RGBA normalization. Both builders thread `frame_id` + `frame_hash` through so the resulting registry entries commit to an exact Frame Registry version.

**Status:** ⚠ Broken at the transport layer. `r2r 0.9.5`'s compile-time-generated `sensor_msgs` typesupport doesn't match the CDR layout some camera drivers publish. Fix in flight.

## Public surface

- Message structs: `StampMsg`, `CameraInfoMsg`, `ImageMsg`, `PointCloud2Msg`, `PointFieldMsg`
- Builders: `build_camera_registry_entry`, `build_sensor_log_entry`, `build_point_cloud_registry_entry`, `build_point_cloud_log_entry`
- Traits + mocks: `CameraSubscriber`, `PointCloudSubscriber`
- `r2r_subscriber` module — the real ROS2 wiring (currently affected by the bug above).

## Depends on

- [`auki-registry`](../auki-registry) — for the Sensor + Frame Registry entry types.
- [`auki-datatypes`](../auki-datatypes) — for the camera / point-cloud segment payloads.
- `r2r 0.9.5` (external) — the only crate in the workspace that pulls this in.
