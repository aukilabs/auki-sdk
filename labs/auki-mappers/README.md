# SDK map producers

`Voxelizer` converts decoded points into SDK voxel evidence updates. Sensor rays
use grid traversal to visit crossed cells once per ray, including negative
coordinates, and exclude the hit cell from free-space evidence. This replaces
approximate metric sampling, which could skip crossed cells. Evidence remains
additive and existing MapUpdate wire formats are unchanged.

The default `runtime` feature preserves the existing Session/stream runner,
discovery, and persistence-filter APIs. Disable default features for pure
voxelization, camera calibration helpers and frame-alias types without the
native Session runtime. This allows the Portal-aligned Component adapter to
compile for WASM. Existing consumers retain their default behavior and APIs.

See [Portal-aligned voxel maps](../auki-voxel-map/README.md) for the explicit
frame/time binding and Component publication layer.

```sh
cargo test --locked -p auki-mappers
cargo test --locked -p auki-mappers --no-default-features
```
