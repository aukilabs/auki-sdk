use std::time::Duration;

use auki_manifests::{
    PoseSource, PoseWriterMode, TimeTransformSource, build_detection_log_manifest,
    build_pose_log_manifest, build_sensor_log_manifest, build_time_transform_log_manifest,
};

#[test]
fn rust_root_api_remains_source_compatible() {
    let sensor = build_sensor_log_manifest(
        "boosterapp",
        "session-1",
        "K1/head",
        "sensorhash",
        "K1/utc",
        "clockhash",
        Some("K1/head_optical"),
        Some("framehash"),
        Duration::from_secs(1),
        Duration::from_secs(30),
    );
    assert_eq!(sensor["sensor_id"], "K1/head");
    assert_eq!(sensor["segment_duration_ns"], 1_000_000_000i64);

    let pose_source = PoseSource::Ros2Tf {
        publishers: vec!["amcl".into(), "robot_state_publisher".into()],
    };
    let pose = build_pose_log_manifest(
        "boosterapp",
        "session-1",
        "map",
        "fromhash",
        "base_link",
        "tohash",
        "K1/utc",
        "clockhash",
        &pose_source,
        PoseWriterMode::Movable,
        100,
        Duration::from_secs(1),
        Duration::from_secs(30),
    );
    assert_eq!(pose["source"]["kind"], "ros2_tf");
    assert_eq!(pose["writer_mode"], "movable");

    let tt = build_time_transform_log_manifest(
        "boosterapp",
        "session-1",
        "K1/monotonic",
        "fromclockhash",
        "K1/utc",
        "toclockhash",
        &TimeTransformSource::LocalClockRead,
        Duration::from_secs(1),
        Duration::from_secs(60),
    );
    assert_eq!(tt["source"]["kind"], "local_clock_read");

    let detection = build_detection_log_manifest(
        "boosterapp",
        "session-1",
        "aukilabs/qr/v1",
        "detectorhash",
        "input-log",
        "K1/head",
        "sensorhash",
        "K1/utc",
        "clockhash",
        Duration::from_secs(1),
        Duration::from_secs(30),
    );
    assert_eq!(detection["detector_id"], "aukilabs/qr/v1");
    assert_eq!(detection["input_log_id"], "input-log");
}
