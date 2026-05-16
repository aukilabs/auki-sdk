# Sprint — auki-geometry

Current work and next steps to close the gap between [`src/readme.md`](readme.md) and [the outer README](../README.md).

## Now

- `convert_pose_convention` shipped as the convention-only layer under future `convert_pose`.
- Point/vector/direction convention helpers shipped.
- Preset conversion and quaternion basis-change tests shipped.

## Next

- Add pose composition and inverse helpers.
- Add interpolation helpers for time-local pose-log reads.
- Add the full `convert_pose` path operation once pose-log indexing/path-finding is ready.
- Add ray helpers after camera intrinsics consumers need them.
