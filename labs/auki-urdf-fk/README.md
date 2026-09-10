# auki-urdf-fk

Experimental URDF parsing and forward kinematics for articulated robots. The
crate runs in native Rust and compiles for WebAssembly. It has no SDK networking
dependencies and does not bundle robot descriptions or meshes.

Parse an application-supplied URDF once, then resolve link transforms for each
joint-angle frame:

```rust
use auki_urdf_fk::{FkError, Model};

fn resolve_pose(xml: &str, angles_rad: &[f32]) -> Result<(), FkError> {
    let model = Model::from_str(xml)?;
    // Angles follow model.joint_names(): active joints in URDF declaration order.
    let links = model.resolve(angles_rad)?;
    for link in links {
        // Column-major 4×4 transform in the model's root frame.
        println!("{}: {:?}", link.link_name, link.transform);
    }
    Ok(())
}
```

Only revolute and continuous joints consume angle slots. Fixed joints retain
their origins; prismatic, floating, planar, and spherical joints are traversed
at zero displacement. Use `Model::load` for a local file on native platforms,
and `Model::from_str` when the application supplies XML, including on the web.

The [Swift binding](../../core/bindings/swift/auki-sdk-swift/README.md) exposes
this crate through its optional `urdf-fk` feature, also included in
`standard-protocols`.

## Source and compatibility

Imported from `aukilabs/auki-libs`, `crates/auki-urdf-fk`, at commit
`46f1d852640b511b54b44d4a286ec5d3ca4a47ed` under the MIT license. The parsing/FK
API and behavior used by the SDK are preserved. This crate contains the upstream
default-feature implementation and its self-contained tests.

The deprecated upstream `pack` feature and bundled-model helpers are not part of
this SDK crate. They require the separate `auki-robot-assets` package. Consumers
of those helpers must supply their own URDF/assets before migrating here. The
SDK's Swift and Expo APIs and backend contracts are unchanged.

## Validation

Run from the SDK repository root:

```sh
cargo test --locked -p auki-urdf-fk
cargo check --locked -p auki-urdf-fk --target wasm32-unknown-unknown
cargo clippy --locked -p auki-urdf-fk --all-targets -- -D warnings
```

The tests use inline URDF fixtures and do not contact shared services.
