# Auki geometry

Pure, IO-free spatial math: coordinate conventions, unit conversion, pose
composition, inversion, and matrix conversion.

## Pose estimation

Enable the optional `pnp` feature to access `auki_geometry::pnp`. It exposes the
complete public API of Auki's `pnp-core`, pinned to revision
`c56b1d4d5518eac997f020168a4e508bb27d599a` from
[aukilabs/pnp-lab](https://github.com/aukilabs/pnp-lab).
The upstream implementation remains the source of the algorithms; it is not
copied into this crate.

Capabilities include general monocular PnP, square-marker poses from pixels or
rays, stereo and multi-view pose estimation, triangulation, camera calibration,
pinhole/Brown–Conrady/fisheye camera models, and pose utilities.

```rust
# #[cfg(feature = "pnp")] {
use auki_geometry::pnp::{Camera, Vector2, estimate_square_pose_from_pixels};
let camera = Camera::pinhole(800.0, 800.0, 320.0, 240.0).unwrap();
let corners = [[280.0, 200.0], [360.0, 200.0], [360.0, 280.0], [280.0, 280.0]]
    .map(|[x, y]| Vector2::new(x, y));
let estimate = estimate_square_pose_from_pixels(corners, 0.1, &camera).unwrap();
# }
```

These APIs preserve upstream types and conventions. Pixels use top-left origin,
+X right and +Y down. Object-pose solver outputs use OpenGL camera coordinates;
convert with `pnp::pose_tools::from_opengl_to_opencv` for +X right, +Y down,
+Z forward. Square corners are TL, TR, BR, BL in printed-marker order. Lengths
and translations share the input geometry's units. Map frame selection, map
updates, observation provenance and acceptance thresholds belong to consumers.

The upstream PnP return types are numerical solver results and do not carry
SDK frame identities. They must be wrapped with explicit source and destination
frame references before reporting or publishing a pose; the QR localizer does
this for its camera-to-map result. The raw re-export is not a frame-checked SDK
pose API.

The existing geometry API and default dependencies are unchanged. The PnP types
are namespaced under `pnp`; they are not implicit replacements for
`auki-datatypes` poses. Python bindings currently expose the existing convention
and transform helpers only; PnP bindings are not added here.

```sh
cargo test --locked -p auki-geometry --all-features
cargo check --locked -p auki-geometry --all-features --target wasm32-unknown-unknown
```
