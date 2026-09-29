use auki_registry::{CoordinateConvention as C, FrameRegistryEntry, LengthUnit};

#[test]
fn named_conventions_preserve_existing_explicit_registry_presets() {
    for (c, old) in [
        (C::OpenGl, FrameRegistryEntry::opengl("peer", "frame")),
        (C::Ros2Body, FrameRegistryEntry::ros_body("peer", "frame")),
        (
            C::Ros2Optical,
            FrameRegistryEntry::ros_optical("peer", "frame"),
        ),
        (C::Unity, FrameRegistryEntry::unity("peer", "frame")),
    ] {
        let new = FrameRegistryEntry::in_convention("peer", "frame", c);
        assert_eq!(new, old);
        assert_eq!(new.units, LengthUnit::Meters);
        new.validate().unwrap();
    }
}

#[test]
fn enum_names_round_trip_and_expand_to_expected_axes() {
    use auki_registry::AxisDirection::*;
    for (c, xyz) in [
        (C::OpenGl, [Right, Up, Backward]),
        (C::Ros2Body, [Forward, Left, Up]),
        (C::Ros2Optical, [Right, Down, Forward]),
        (C::Unity, [Right, Up, Forward]),
        (C::ZUpRightForward, [Right, Forward, Up]),
    ] {
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, format!("\"{}\"", c.as_str()));
        assert_eq!(serde_json::from_str::<C>(&json).unwrap(), c);
        let axes = c.axes();
        assert_eq!([axes.x, axes.y, axes.z], xyz);
    }
    assert!(serde_json::from_str::<C>("\"ros2\"").is_err()); // Body/optical is explicit.
}
