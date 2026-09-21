# auki-dsc

Local, IO-free spatial compute: Recast bake + path/restrict + occlusion raycast/cross-section. Intended to make `domain-spatial-computer` unnecessary.

**Status:** v0 on `feature/dsc`. Engine is `rerecast` (bake) + `landmass` (query). Not Detour. Python/Wasm bindings are not in this crate yet.

```bash
cargo test -p auki-dsc
```

## Public surface

```text
BakeProfile { gotu(), dsc(radius), robot_r40(), restrict_bake() }
TriangleMesh / parse_obj
NavMesh::bake(mesh, profile)
  find_closest_point / find_path / find_optimized_path
  find_optimized_segment_path / restrict
OcclusionMesh::from_mesh
  raycast(origin, direction)
  cross_section(height, ppm)  // PNG bytes + map.yaml
```

Coordinates: metres, **Y-up** (domain OBJ / OpenGL). Landmass is Z-up; conversion is only at the bake/query boundary inside this crate. No silent frame convert on the public API.

`restrict` min-separation is **not** bake `walkable_radius`. DSC overloaded that name — use `BakeProfile::restrict_bake()` / `dsc(0.0)` for a zero-erosion bake, then pass min-separation to `restrict`.

Downward-facing triangles are flipped before bake (matches DSC Python). Polygon winding is reversed at the landmass boundary after the Y↔Z remap.

`landmass_rerecast` is Bevy-tied; v0 bridges poly-mesh → landmass by hand.

## Tests

- Synthetic open floor + L-shape path/restrict/optimized/occlusion (`tests/floor.rs`, `fixtures/`).
- OBJ parse (`tests/obj_parse.rs`).
- Golden vs DSC Deno: `tests/golden.rs` is `#[ignore]` until `fixtures/golden/{mesh.obj,findpath.json}` are committed. Soft Hausdorff gate 0.5 m (measurement, not bit-identical).

## Out of this crate / this PR

DS HTTP fetch, STCM encoder, Map Registry / Blob tip, PyO3, Wasm, Unity, consumer cutover.
