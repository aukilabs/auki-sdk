use auki_registry::{
    Aruco, AxisConvention, AxisDirection, Camera, ClockBody, ClockMeta, ClockRegistryEntry,
    DetectorBody, DetectorRegistryEntry, FrameRegistryEntry, Handedness, LengthUnit, Scope,
    SensorBody, SensorRegistryEntry,
};

#[test]
fn rust_root_api_remains_source_compatible() {
    let frame = FrameRegistryEntry::ros_body("K1-AABBCCDDEEFF/base_link");
    assert_eq!(frame.hash(), "fd0dc3789e898b71b5e16ee122a81a44");

    let explicit_frame = FrameRegistryEntry {
        frame_id: "K1-AABBCCDDEEFF/base_link".into(),
        handedness: Handedness::Right,
        axes: AxisConvention {
            x: AxisDirection::Forward,
            y: AxisDirection::Left,
            z: AxisDirection::Up,
        },
        units: LengthUnit::Meters,
    };
    assert_eq!(frame, explicit_frame);
    frame.validate().unwrap();

    let sensor = SensorRegistryEntry {
        sensor_id: "K1-AABBCCDDEEFF/head_left_cam".into(),
        body: SensorBody::Camera(Camera {
            width: 544,
            height: 488,
            frame_rate_hz: 20,
            pixel_format: "YUV_NV12".into(),
            color_space: "BT.709".into(),
            intrinsics_model: "pinhole".into(),
            distortion_model: "plumb_bob".into(),
            frame_id: "K1-AABBCCDDEEFF/head_left_cam_optical".into(),
            frame_hash: "e0d40e7b526e04f15f83f75897f53825".into(),
        }),
    };
    assert_eq!(sensor.hash(), "5559c9648e31eee2410b692fef393489");

    let clock = ClockRegistryEntry {
        clock_id: "K1-AABBCCDDEEFF/utc".into(),
        body: ClockBody::UtcClock(ClockMeta {
            unit: "milliseconds".into(),
            monotonic: false,
            epoch: Some("1970-01-01T00:00:00Z".into()),
            scope: Scope::Global,
        }),
    };
    assert_eq!(clock.hash(), "89f84f4c2e09bef81d385b2af1d17e6c");

    let detector = DetectorRegistryEntry {
        detector_id: "aukilabs/aruco/v1".into(),
        body: DetectorBody::Aruco(Aruco {
            dictionary: "5x5_50".into(),
        }),
        output_types: vec!["aruco".into()],
    };
    assert_eq!(
        std::str::from_utf8(&detector.canonical_bytes()).unwrap(),
        r#"{"detector_id":"aukilabs/aruco/v1","dictionary":"5x5_50","output_types":["aruco"],"type":"aruco"}"#
    );
}
