use std::fs;
use std::path::PathBuf;

const PROTOS: &[&str] = &[
    "audio.proto",
    "audio_stream.proto",
    "camera.proto",
    "detection.proto",
    "joint_encoders.proto",
    "joint_encoders_stream.proto",
    "message.proto",
    "point_cloud.proto",
    "point_cloud_stream.proto",
    "pose.proto",
    "stream.proto",
    "time_transform.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::current_dir()?;
    let proto_root = repo.join("proto");
    let out_dir = repo.join("crates/auki-proto/src/generated");

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;

    let protoc = protoc_bin_vendored::protoc_bin_path()
        .expect("vendored protoc binary not available for this platform");
    let protos: Vec<PathBuf> = PROTOS
        .iter()
        .map(|proto| proto_root.join("auki").join(proto))
        .collect();

    prost_build::Config::new()
        .out_dir(&out_dir)
        .protoc_executable(protoc)
        .compile_protos(&protos, &[proto_root])?;

    Ok(())
}
