# auki-maps

`auki-maps` materializes SDK Map Logs into deterministic in-memory state and
adapts changed voxel chunks into renderer-neutral instance buffers.

## Current surface

- `VoxelMapAccumulator::new(map_ref, contract)` validates and pins the exact
  content-addressed Map identity and its `VoxelMap` Registry body.
- `VoxelMapAccumulator::apply(update)` merges one sparse `MapUpdate`
  atomically and reports the changed chunks.
- `viewer_snapshot(threshold)` exposes the current sparse voxel state in Map
  frame coordinates.
- `VoxelViewerAdapter::changed_chunks(...)` emits `ChunkRenderUpdate::Replace`
  or `ChunkRenderUpdate::Remove` operations containing pickable cube
  `VoxelInstance`s.

Updates use additive occupancy evidence. Applying independent updates in any
order produces the same state. Invalid coordinates, non-finite evidence, and
unknown semantic classes are rejected without partially mutating the map.

This crate owns no renderer and performs no networking. A consumer opens an
SDK Map Log stream, fetches its exact Map Registry entry, applies replay and
live updates to the accumulator, and forwards the resulting chunk operations
to its renderer. Viewer snapshots retain the Map Registry reference so several
maps cannot be confused in one UI.
