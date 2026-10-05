# auki-navigation

IO-free local Recast bake + landmass path / snap / optimize / restrict.

Mesh parse, triangulation, and raycast live in **`auki-geometry`**. Occupancy slice is **`auki-raster`**.

No HTTP, no Domain fetch, no asset-type names. Legacy `navmesh_v1` / `occlusionmesh_v1` are consumer loaders in front of `auki-geometry`.
