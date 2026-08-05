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

## Pure voxelization

`Voxelizer` decodes the canonical XYZ fields from an SDK `Rangefinder`
payload, interpolates the SDK-supplied sensor-to-Map pose, traces free-space
rays, and emits additive occupied/free evidence grouped into sparse chunks.

## Camera calibration for metric Mappers

`effective_camera_calibration` applies the shared camera contract used by PnP
and future Portal Mappers. A `CameraFrame.dynamic_intrinsics` value replaces
the complete Camera Registry calibration for that frame. Otherwise the static,
content-addressed registry calibration is used. Resolution fails closed when
neither source exists or when the selected calibration is invalid.

## Portal PnP

`estimate_portal_observation` turns four detector-provided image corners in
`TL, TR, BR, BL` order and a canonical square Portal size into a metric
`PortalObservation`. It resolves static or per-frame calibration through the
shared camera contract and supports pinhole Brown–Conrady / ROS `plumb_bob` as
well as OpenCV fisheye / ROS `equidistant` distortion.

The exact Camera Frame Registry entry is also required. The Mapper verifies
its content hash against the Camera reference and uses its declared axes and
units to express the result, failing closed on an incompatible convention.

The API is detector-agnostic: QR Lab is the first reference detector, but any
detector can provide the ordered corners. Portal payload recognition and
Portal Service lookup remain application concerns. The result is a
camera-frame observation with confidence and normalized corner error, not an
authoritative Map placement; a later materializer can fuse repeated and
multi-peer observations.

## Live Portal Mapper flow

`PortalMapperRunner` consumes three streams on one exact SDK clock:

1. normalized detector batches carrying a source Camera Sensor hash,
2. the original Camera Frames at the same timestamps, and
3. Camera→Map poses bracketing those timestamps.

For every candidate, an application-supplied `PortalResolver` decides whether
the detector payload is a Portal and returns its canonical physical size. The
runner resolves frame-specific calibration, performs PnP, interpolates the
Camera→Map pose, and writes a Portal→Map observation. A batch is rejected if
its Camera Sensor hash differs from the configured camera, and it is skipped
if no Camera Frame exists at the exact detector timestamp.

Observation identity is `(source Detection Log, source sequence, detection
index)`, not timestamp alone. This makes replay idempotent even when a log has
multiple samples with the same timestamp. `PortalMapAccumulator` rejects
conflicting content under one provenance key and conflicting canonical sizes
for one Portal, and supports ordered checkpoint barriers. It deliberately
retains observations rather than selecting an authoritative Portal pose;
fusion remains a separate materializer policy.
