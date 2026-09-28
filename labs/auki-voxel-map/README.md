# Portal-aligned voxel maps

A single-writer voxel Map Component aligned explicitly to a Portal map. It reuses
`auki-mappers::Voxelizer` and `auki-maps::VoxelMapAccumulator`; it does not introduce
a second occupancy model. No detector, device driver, tracking service, database
lookup or background network task is started.

## Flow

1. Obtain a validated, non-empty Portal `MapSnapshot` and its exact Product/sequence.
2. Declare full SDK frame registry entries for the map and depth sensor. The map
   frame ID, handedness, units and up axis must agree with the Portal map. This
   first version uses the Portal frame directly, right-handed meters only.
   `grid_origin_m` explicitly equals `[0,0,0]`; voxel size is in meters and chunks
   have 16 cells per edge. The host supplies the full axis convention; it is not
   guessed from the map ID or a Domain.
3. Create `VoxelMapDefinition::from_portal_map` with a distinct voxel map identity,
   source depth Product, sensor frame, observation clock, and grid resolution.
   The constructor creates one `SensorBinding` in `sources`; add bindings for other
   admitted Products before constructing the Component. Each binds an exact Product,
   sensor frame and capture clock. The list is fixed for this map instance.
4. Create `VoxelMapComponent` on the peer's `ComponentRuntime` with fresh publication
   identity and a monotonic publication clock.
5. Use QR localization (or tracking tied to it) to obtain a sensor-to-map transform
   valid at each depth observation's capture timestamp. If the depth sensor differs
   from the localization camera, the host must compose calibrated extrinsics first.
6. Submit a `DepthObservation`. It includes its exact source Product and sequence,
   capture timestamp/clock, sensor-frame ID, measured hit endpoints in meters, and
   a `TimedSensorPose` with explicit from/to frames and Portal snapshot provenance.
7. Read `component.product()` locally, or explicitly export it through
   `ComponentProtocolEndpoint::export_product` using the existing protocol.

The pure `PortalVoxelMapper::map_observation` checks these bindings and produces
an SDK `MapUpdate`. `VoxelMapComponent::integrate` applies it to a candidate
accumulator and publishes only validated, bounded state. Invalid frames, clocks,
timestamps, source identities, geometry or provenance leave accepted state intact.
Publication failure closes ingestion rather than acknowledging unretained state.

## Occupancy and publication

Rays mark crossed cells as free evidence and their measured endpoints as occupied
evidence. The endpoint cell is never also cleared by its own ray. Unknown space
remains absent. These are additive scores (`-1` free, `+1` occupied), not calibrated
probabilities; clearing repeatedly observed occupancy requires sufficient contrary
evidence. Per-point evidence is not normalized by scan density or uncertainty.
Only actual hit endpoints are accepted: no-return or max-range samples need a
separate sensor-model adapter and must not be treated as occupied surfaces.

The `auki.voxel-map.snapshot/v2` Product retains one complete checkpoint. It contains
the existing SDK `MapUpdate` protobuf bytes, map/frame contracts, Portal-map
snapshot reference, observation count, and latest observation/pose provenance.
The Observable explicitly declares its spatial frame. Catalog metadata uses
`auki.voxel-map.catalog/v2` and advertises the grid and Portal alignment reference.
It does not claim that the voxel map contains a copied list of Portal anchors.
`VoxelSnapshot::accumulator()` validates and decodes received snapshots.

Viewer snapshots contain only occupied voxels. Inspect the full checkpoint when
examining negative free-space evidence. Geometry is currently occupancy-only;
the underlying SDK map format already supports optional semantic/color evidence,
but this adapter does not add those attributes.

## Ordering, corrections and bounds

Up to 64 source Products are explicitly pinned. Capture times and sequences must
strictly increase **within each source**; clocks and sequence numbers are never
compared across peers. An exact repeat of each source's latest accepted observation
is a no-op, even after another source contributes. Older/revised observations are
rejected and cannot count twice. Only the latest full observation per source is
kept for replay checks, so memory is bounded. A fresh camera publication needs a
new binding/map build. The host performs time interpolation and clock conversion;
this adapter accepts only an exactly time-matched pose.

All input poses must already target the declared map frame and reference the
pinned Portal snapshot. The host must establish alignment and compose transforms
before ingestion; shared Domain membership is never enough. Source bindings are
an ingestion allowlist, not proof of remote identity. The host must authenticate
transport, verify that the sender may supply the bound Product, and authorize
contributions. No remote write Operable or automatic subscription is added.

The additive evidence model does not determine which peer's observation is newer.
Delayed but unseen evidence can still affect the map. It does not implement decay,
last-observation-wins, or reconciliation between independently maintained maps.

The v2 snapshot/catalog contract replaces the singular sensor fields with an
explicit `sources` list. Consumers must support v2 before receiving these Products;
v1 definitions are not silently migrated. `from_portal_map` keeps its existing
single-source constructor signature. No backend protocol is changed.

Portal-map provenance is pinned. A changed Portal snapshot or corrected historic
pose requires rebuilding from retained source observations; this implementation
does not silently reposition already integrated evidence. The checkpoint retains
latest provenance only, not a per-observation history. Persist source observations
and poses separately when replay/correction is required. Crash recovery and
resuming ingestion from a checkpoint are not implemented.

Limits: 1,024 hit points per observation, 100m maximum ray range, a conservative
65,536-cell traversal work budget per observation, 65,536 stored voxels, and a
4MiB encoded snapshot. Voxel sizes range from 1cm to 10m. A capacity failure rejects
the entire observation; it never partially inserts it or silently evicts evidence.
These are prototype bounds, not supermarket-scale capacity. Chunk streaming remains future work.

## Validation

`tests/portal_pipeline.rs` computes a real PnP pose from synthetic Portal corners,
integrates depth in that frame, checks occupied/free/unknown cells, verifies
free-space clearing, and checks catalog/snapshot publication. It also tests replay,
invalid inputs, time/frame mismatches, changed Portal provenance, and publication
failures. The two-peer test creates separate Component runtimes and retained depth
Products, gives the sensors different poses and unrelated clocks, and feeds their
serialized observations into one mapper. It checks that two free-space observations
outweigh an earlier hit, that the new location is occupied, and that replay and
invalid contributions cannot mutate accepted state. It uses no physical camera,
network transport or live backend.

```sh
cargo test --locked -p auki-voxel-map -p auki-mappers -p auki-maps
cargo clippy --locked -p auki-voxel-map --all-targets --no-deps -- -D warnings
cargo check --locked -p auki-voxel-map --target wasm32-unknown-unknown
```
