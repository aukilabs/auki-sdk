# auki-mappers

`auki-mappers` contains SDK-native Map producers. Its first Mapper is a voxel
Mapper which consumes SDK point-cloud and pose logs and writes sparse,
mergeable `MapUpdate`s. It has no robot or ROS API surface.

## Live voxel Mapper flow

1. Merge Resource Catalog rows obtained through the SDK.
2. Fetch the selected Map Registry body with
   `ClusterManager::fetch_map_entry`.
3. Use `VoxelMapperSources::select` to choose the unique live point-cloud and
   pose pair connecting the sensor frame to the selected Map frame.
4. Open `point_cloud_request` and `pose_request` against each row's writer
   peer through `Domain`/`ClusterManager`.
5. Pass the accepted subscriptions, the Rangefinder Registry body, the Voxel
   Map Registry body, and a `MapUpdateSink` to `run_sdk_voxel_mapper`.

Selection and stream binding require exact content-addressed frames and one
shared SDK clock. Accept-time resource/clock mismatches, timestamp regressions,
and sequence gaps fail closed. Point-cloud and pose source peers and the Map
Log destination peer are independent.

`LocalMapLogSink` is the initial Galbot deployment sink. Other peers or
materializers can implement `MapUpdateSink` without changing the Mapper.

## Pure voxelization

`Voxelizer` decodes the canonical XYZ fields from an SDK `Rangefinder`
payload, interpolates the SDK-supplied sensor-to-Map pose, traces free-space
rays, and emits additive occupied/free evidence grouped into sparse chunks.
