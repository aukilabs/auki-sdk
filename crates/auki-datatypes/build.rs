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

    let mut config = prost_build::Config::new();

    for ty in [
        ".auki.sensors.SensorsRequest",
        ".auki.sensors.SensorsResponse",
        ".auki.sensors.SensorEntry",
    ] {
        config.type_attribute(
            ty,
            "#[cfg_attr(feature = \"swift-bindings\", derive(uniffi::Record))]",
        );
    }

    config
        .protoc_executable(protoc)
        .compile_protos(
            &[
                "proto/audio.proto",
                "proto/camera.proto",
                "proto/detection.proto",
                "proto/info.proto",
                "proto/join.proto",
                "proto/joint_encoders.proto",
                "proto/point_cloud.proto",
                "proto/pose.proto",
                "proto/sensors.proto",
                "proto/stream.proto",
                "proto/time_transform.proto",
            ],
            &["proto/"],
        )?;

    println!("cargo:rerun-if-changed=proto/");
    Ok(())
}
