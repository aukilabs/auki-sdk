# Changelog — auki-geometry

Detailed changes for `auki-geometry`. Latest entry on top.

---

### Nils's codex · May 24, HKT, 2026

Swift package template now excludes the generated XCFramework directory from the source target while retaining it as a binary target, removing SwiftPM unhandled-file warnings from generated package builds.

### Nils's codex · May 24, HKT, 2026

**Multiplatform binding standard added.** The typed geometry implementation moved into binding-free `src/core.rs`; `src/lib.rs` now only wires feature-gated modules and re-exports the existing Rust API. Native `src/ffi.rs` exposes UniFFI adapters for Python and Swift; `src/wasm.rs` exposes wasm-bindgen adapters for JavaScript/WebAssembly. Generated languages use JSON-string helpers for frame entries, axis conventions, vectors, quaternions, and poses while Rust keeps the typed `FrameRegistryEntry` / `SpatialTransform` API.

**Binding assets.** The crate now declares `staticlib` / `cdylib` / `rlib`, default `uniffi`, `cli`, and `wasm` features, a crate-local `uniffi-bindgen` binary, `bindings.toml`, and crate-owned Python, Swift, and JavaScript package templates plus a JavaScript smoke vector.

**Tests.** Added JSON adapter locked vectors in the core tests and `tests/surface.rs` to pin crate-root source compatibility. Existing convention/quaternion tests remain the Rust behavior lock; generated package checks cover Python import/conversion smoke, JavaScript wasm smoke, and SwiftPM build output.

### Nils's codex · May 24, HKT, 2026

Opted the `auki-registry` dependency out of default features after `auki-registry` adopted the binding standard, keeping geometry conversion builds on the direct Rust registry types without inheriting generated binding dependencies.

### Nils's codex · May 22, HKT, 2026

Active geometry docs now point pose payload readers at generated `auki-proto`.

### Nils's codex · May 22, HKT, 2026

Switched pose payload imports from deprecated `auki-datatypes` to generated Rust `auki-proto`.

### Nils's codex · May 16, 11:34 HKT, 2026

Initial `auki-geometry` crate scaffolded as the pure spatial-math home for the SDK. Phase 1 ships convention conversion: `meters_per_unit`, `axis_convention_matrix`, `convention_matrix`, `convert_point_convention`, `convert_vector_convention`, `convert_direction_convention`, and `convert_pose_convention`. The crate depends on `auki-registry` for `FrameRegistryEntry` declarations and `auki-datatypes` for `SpatialTransform` / `Vec3` / `Quat`, but does no registry IO, log IO, or networking. Conversion is convention-agnostic at the public API: direct declared convention A -> declared convention B, with no exposed canonical Auki frame. `convert_pose_convention` re-expresses the same physical pose in a target convention; full pose-log graph/path `convert_pose` remains future work. Tests lock ROS optical/body, OpenGL/Three.js, and Unity signed permutations; unit scaling; handedness mismatch rejection; and quaternion basis-change behavior.
