//! Build script — invokes `prost-build` to compile every `.proto` file
//! under `proto/` into Rust code under `OUT_DIR`. Generated files are
//! re-included from `src/lib.rs`.
//!
//! `protoc` binary is supplied by `protoc-bin-vendored` so the build is
//! self-contained — no system `protoc` install needed on dev machines
//! or CI.

fn main() -> std::io::Result<()> {
    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("vendored protoc binary not available for this platform");

    prost_build::Config::new()
        .protoc_executable(protoc)
        .compile_protos(
            &[
                "proto/placeholder.proto",
                "proto/camera.proto",
                "proto/frame_stream.proto",
                "proto/point_cloud.proto",
                "proto/point_cloud_stream.proto",
                "proto/stream.proto",
            ],
            &["proto/"],
        )?;

    println!("cargo:rerun-if-changed=proto/");
    Ok(())
}
