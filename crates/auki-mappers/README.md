# auki-mappers

`auki-mappers` contains SDK-native Map producers. Its first Mapper is a voxel
Mapper which consumes SDK point-cloud and pose logs and writes sparse,
mergeable `MapUpdate`s. It has no robot or ROS API surface.

## Live voxel Mapper flow

1. Merge Resource Catalog rows obtained through the SDK.
2. Fetch the selected Map Registry body with
   `ClusterManager::fetch_map_entry`.
3. Use `VoxelMapperSources::select` to choose the unique live point-cloud and
   pose pair connecting the sensor frame to the selected Map frame. If the Map
   publisher needs to own that frame identity, construct a
   `ValidatedFrameAlias` from both exact Frame Registry entries and call
   `select_with_frame_alias`; this permits only an identity-preserving rebind,
   never an implicit coordinate conversion.
4. Open `point_cloud_request` and `pose_request` against each row's writer
   peer through `Domain`/`ClusterManager`.
5. Pass the accepted subscriptions, the Rangefinder Registry body, the Voxel
   Map Registry body, and a `MapUpdateSink` to `run_sdk_voxel_mapper`.

Selection and stream binding require exact content-addressed frames (or an
explicit alias whose handedness, axes, and units match exactly) and one
shared SDK clock. Accept-time resource/clock mismatches, timestamp regressions,
and sequence gaps fail closed. Point-cloud and pose source peers and the Map
Log destination peer are independent.

`LocalMapLogSink::new` preserves aligned input timestamps and therefore fails
closed unless the input logs and destination Map Log declare the same exact
clock. `LocalMapLogSink::retimestamped` samples the destination clock when it
appends each update, allowing a Mapper hosted by Park (or any other peer) to
consume a remote peer's aligned point-cloud/pose pair while publishing a
correctly-clocked local Map Log. Other peers, explicit time-transform sinks,
or materializers can implement `MapUpdateSink` without changing the Mapper.

Point-cloud and pose inputs must still share one exact clock so interpolation
is meaningful. The output Map Log clock is independent: the sink owns the
conversion or restamping boundary and the run report exposes both
`alignment_clock` and `map_clock`.

A restamped update's Map Log timestamp is its production/append time, not the
original sensor observation time. Applications that need observation-time
provenance should provide a sink backed by an explicit SDK time transform.

The live runner separates stream alignment from voxel computation. Point-cloud
and pose subscriptions remain responsive on the async runtime; once a point
cloud has a bracketing pose, the aligned job enters a bounded blocking-worker
queue. The queue is latest-wins, so a newer aligned cloud replaces stale pending
work while one cloud is being voxelized. The run report distinguishes these
worker-overload drops from point clouds dropped while waiting for poses.

`MapUpdateSink::append_from` is part of the async boundary: implementations must
not perform expensive synchronous compaction, encoding, or I/O before returning
their future. Applications also control runner lifetime. A UI such as Park
should start the voxel Mapper only while an output consumer is active and stop
it when demand disappears.

## Pure voxelization

`Voxelizer` decodes the canonical XYZ fields from an SDK `Rangefinder`
payload, interpolates the SDK-supplied sensor-to-Map pose, traces free-space
rays, and emits additive occupied/free evidence grouped into sparse chunks.

## Stable occupancy

`run_sdk_voxel_mapper` enables `VoxelPersistenceConfig::default()` unless its
service configuration explicitly sets `persistence: None`. The filter first
normalizes a point-cloud frame to at most one occupied or free observation per
voxel, so point density does not count as repeated evidence. A candidate voxel
is published only after six observations spanning three seconds with no gap
over 500 ms. Once confirmed, it remains in the Map until continuous free-space
observations span one second. Missing or occluded observations do not clear it.

All four thresholds are configurable. The low-level `VoxelMapperRunner`
retains its immediate raw-evidence behavior unless the application calls
`with_persistence`, which keeps existing algorithm and test uses explicit.

## Camera calibration for metric Mappers

`effective_camera_calibration` applies the shared camera contract used by PnP
and future Portal Mappers. A `CameraFrame.dynamic_intrinsics` value replaces
the complete Camera Registry calibration for that frame. Otherwise the static,
content-addressed registry calibration is used. Resolution fails closed when
neither source exists or when the selected calibration is invalid.
