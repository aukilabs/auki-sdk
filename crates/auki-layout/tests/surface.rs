use std::path::{Path, PathBuf};

use auki_layout::{
    clock_entry_path, detection_log_path, detector_entry_path, frame_entry_path, id_to_segment,
    poselog_path, registries_root, sensor_entry_path, sensorlog_path, session_root,
    timetransform_log_path,
};

#[test]
fn rust_root_api_remains_source_compatible() {
    let app = Path::new("/app");

    assert_eq!(registries_root(app), PathBuf::from("/app/registries"));
    assert_eq!(
        sensor_entry_path(app, "K1/head", "abcd"),
        PathBuf::from("/app/registries/sensors/K1__head/abcd.json")
    );
    assert_eq!(
        clock_entry_path(app, "K1/utc", "1234"),
        PathBuf::from("/app/registries/clocks/K1__utc/1234.json")
    );
    assert_eq!(
        frame_entry_path(app, "K1/base", "5678"),
        PathBuf::from("/app/registries/frames/K1__base/5678.json")
    );
    assert_eq!(
        detector_entry_path(app, "aukilabs/qr/v1", "9abc"),
        PathBuf::from("/app/registries/detectors/aukilabs__qr__v1/9abc.json")
    );

    let session = session_root(app, "session-1");
    assert_eq!(session, PathBuf::from("/app/session-1"));
    assert_eq!(
        timetransform_log_path(&session, "K1/utc", "K1/monotonic"),
        PathBuf::from("/app/session-1/timetransform_logs/K1__utc__K1__monotonic")
    );
    assert_eq!(
        sensorlog_path(&session, "input-log"),
        PathBuf::from("/app/session-1/sensorlogs/input-log")
    );
    assert_eq!(
        poselog_path(&session, "K1/base", "K1/cam"),
        PathBuf::from("/app/session-1/poselogs/K1__base__K1__cam")
    );
    assert_eq!(
        detection_log_path(&session, "aukilabs/qr/v1", "input-log"),
        PathBuf::from("/app/session-1/detection_logs/aukilabs__qr__v1__input-log")
    );
    assert_eq!(id_to_segment("a/b/c"), "a__b__c");
}
