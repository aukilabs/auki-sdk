//! Build script — invokes `prost-build` to compile every `.proto` file
//! under `proto/` into Rust code under `OUT_DIR`. Generated files are
//! re-included from `src/lib.rs`.
//!
//! `protoc` binary is supplied by `protoc-bin-vendored` so the build is
//! self-contained — no system `protoc` install needed on dev machines
//! or CI.
//!
//! Schemas land here per [`src/sprint.md`](src/sprint.md). Sawslin Lane 0
//! pulls `auki.pose` (originally step 5) and a new `auki.joint_state`
//! forward — see [`src/sprint.md`](src/sprint.md) for the rationale.

fn main() -> std::io::Result<()> {
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("vendored protoc binary not available for this platform");

    prost_build::Config::new()
        .protoc_executable(protoc)
        .compile_protos(
            &[
                "proto/joint_state.proto",
                "proto/pose.proto",
                "proto/pose_stream.proto",
            ],
            &["proto/"],
        )?;

    println!("cargo:rerun-if-changed=proto/");
    Ok(())
}
