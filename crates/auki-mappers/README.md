# auki-mappers

SDK-native Map producers. The crate contains no robot, ROS, authentication, or
peer-runtime policy.

Its first Mapper aligns a point-cloud stream with a pose stream, voxelizes the
result, and writes sparse mergeable `MapUpdate`s.

## Application flow

The host composes networking through the portable protocol Clients:

1. Fetch Catalog v3 rows with `CatalogClient` and extract the v2-shaped Sensor
   and Pose Log rows.
2. Fetch the exact Map, Rangefinder, Frame, and Clock Registry entries with
   `RegistryClient`.
3. Use `VoxelMapperSources::select` to choose one unambiguous point-cloud/pose
   pair for the requested Map frame.
4. Open both typed subscriptions with `StreamClient`.
5. Convert them with `MapperInput::from_sdk_subscription` and run
   `run_sdk_voxel_mapper` into an application-owned `MapUpdateSink`.

Catalog, Registry, and Stream clients authenticate the expected peer. Mapper
selection then verifies the content-addressed data contracts; route knowledge
alone is never enough.

## Exact frame and clock binding

Point-cloud and pose inputs must share one exact clock so interpolation is
meaningful. Frame references must match exactly unless the application provides
a `ValidatedFrameAlias` whose handedness, axes, and units are identical. An
alias is an identity-preserving rebind, not an implicit coordinate conversion.

Accept-time resource/clock mismatch, timestamp regression, and sequence gaps
fail closed. Source peers for point cloud and pose may differ from the peer
writing the output Map Log.

## Output sinks

`LocalMapLogSink::new` preserves aligned input timestamps and therefore
requires the destination Map Log to use the same exact clock.

`LocalMapLogSink::retimestamped` samples the destination clock while appending.
This lets a viewer or compute peer consume remote aligned inputs and publish a
correctly clocked local Map Log. Its timestamp is production time, not original
observation time; applications needing observation-time lineage should provide
a sink backed by an explicit time transform.

`MapUpdateSink::append_from` is asynchronous. Implementations must not perform
expensive synchronous encoding, compaction, or I/O before returning their
future.

## Bounded live runner

Subscription polling remains on the async runtime. Once a point cloud has a
bracketing pose, the aligned job enters a bounded blocking-worker queue. The
queue is latest-wins: newer aligned work replaces stale pending work while one
cloud is voxelized.

The run report separates:

- point clouds dropped while waiting for a usable pose; and
- aligned jobs dropped because the worker was overloaded.

Applications own lifetime. Start a Mapper when an output is needed and stop it
when demand ends.

## Voxel behavior

`Voxelizer` decodes canonical XYZ fields from an SDK Rangefinder payload,
applies the interpolated sensor-to-Map pose, traces free-space rays, and emits
occupied/free evidence grouped into sparse chunks.

`run_sdk_voxel_mapper` enables stable occupancy filtering by default. Evidence
is normalized to one observation per voxel per frame. Candidate voxels require
repeated observations over time before publication, and confirmed voxels need
continuous free-space evidence before removal. Thresholds are configurable.

The lower-level `VoxelMapperRunner` retains immediate raw-evidence behavior
unless persistence is explicitly enabled.

## Camera calibration

`effective_camera_calibration` selects dynamic frame intrinsics when present;
otherwise it uses the exact content-addressed Camera Registry calibration. It
fails closed if neither source is valid.
