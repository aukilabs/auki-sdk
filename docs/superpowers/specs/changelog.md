# Changelog — docs/superpowers/specs

Append-only timeline of design spec changes. Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup design spec, locking the breaking full rename to `CameraFrame`, `DetectionFrame`, and `SensorBody::Camera` with no compatibility aliases or legacy registry tags.

### Nils's codex · May 19, HKT, 2026

Added the native Auki pointcloud design spec, capturing the approved breaking refactor from ROS CDR pointcloud streams to a shared `auki.point_cloud.PointCloudFrame { point_count, data }` record for logs and streams.
