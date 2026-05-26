use auki_geometry::{convert_point_convention, convert_point_convention_json, meters_per_unit};
use auki_proto::pose::Vec3;
use auki_registry::{AxisConvention, AxisDirection, FrameRegistryEntry, Handedness, LengthUnit};

#[test]
fn rust_root_api_remains_source_compatible() {
    assert_eq!(meters_per_unit(LengthUnit::Meters), 1.0);

    let from = FrameRegistryEntry {
        frame_id: "camera".into(),
        handedness: Handedness::Right,
        axes: AxisConvention {
            x: AxisDirection::Right,
            y: AxisDirection::Down,
            z: AxisDirection::Forward,
        },
        units: LengthUnit::Centimeters,
    };
    let to = FrameRegistryEntry::opengl("world");
    let converted = convert_point_convention(
        Vec3 {
            x: 100.0,
            y: 200.0,
            z: 300.0,
        },
        &from,
        &to,
    )
    .unwrap();
    assert_eq!(converted.x, 1.0);
    assert_eq!(converted.y, -2.0);
    assert_eq!(converted.z, -3.0);

    let converted_json = convert_point_convention_json(
        r#"{"x":100.0,"y":200.0,"z":300.0}"#,
        r#"{"axes":{"x":"right","y":"down","z":"forward"},"frame_id":"camera","handedness":"right","units":"centimeters"}"#,
        r#"{"axes":{"x":"right","y":"up","z":"backward"},"frame_id":"world","handedness":"right","units":"meters"}"#,
    )
    .unwrap();
    assert_eq!(converted_json, r#"{"x":1.0,"y":-2.0,"z":-3.0}"#);
}
