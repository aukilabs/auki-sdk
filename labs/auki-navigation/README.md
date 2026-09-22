# auki-navigation

Recast bake + landmass path / snap / optimize / restrict. Intended to make `domain-spatial-computer` unnecessary for those ops.

**Mesh ingest / raycast:** [`auki-geometry`](../auki-geometry). **Y-slice occupancy:** [`auki-raster`](../auki-raster). Domain assets (`navmesh_v1`, `occlusionmesh_v1`) are consumer loaders (e.g. robot-runner-kit `MeshStore`).

**Status:** v0. Engine is `rerecast` (bake) + `landmass` (query). Bindings: `labs/bindings/python/auki-navigation-py` (`auki_navigation`), `labs/bindings/web/auki-navigation-web`.

```bash
cargo test -p auki-navigation
```

## Compute (this crate)

```text
BakeProfile { gotu(), dsc(radius), robot_r40(), restrict_bake() }
NavMesh::bake(mesh, profile)           // TriangleMesh only
  find_closest_point / find_path / find_optimized_path
  find_optimized_segment_path / restrict / agent_radius
```

`BakeProfile::dsc(radius)` is the cell-size preset name (legacy knobs, not this crate's name). `radius` is **metres** → voxels via `ceil(m / cs)`. Legacy DSC HTTP truncated float→int (`int(0.05)==0`); use `dsc(0.0)` only to bit-match that bug.

Coordinates: metres, **Y-up** (OpenGL). Landmass is Z-up; conversion is only at the bake/query boundary.

`restrict` min-separation is **not** bake `walkable_radius`.

Extents default `(100,10,100)`. Off-mesh stub prepend in DSC `full` is not chased.

Downward-facing triangles are flipped before bake. Polygon winding is reversed at the landmass boundary after the Y↔Z remap.

## Tests

- Synthetic open floor + L-shape (`tests/floor.rs`, `fixtures/`).
- Mesh parse / raycast: `auki-geometry`. Slice: `auki-raster`.
- Golden vs DSC Deno: `tests/golden.rs` is `#[ignore]` until fixtures are committed.

## Out of this crate

HTTP / Domain fetch, STCM, Map Registry, Unity, mesh parse/raycast (`auki-geometry`), occupancy PNG (`auki-raster`).
