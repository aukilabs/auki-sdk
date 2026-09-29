//! Offline example: Z-up meters -> Y-up centimeters, preserving a 40cm Portal.
use auki_registry::{AxisConvention, AxisDirection, FrameRegistryEntry, LengthUnit};
use auki_scenegraph::{
    MapConventionConversion, MapDefinition, MapFrame, MapSnapshot, QrAnchor, RigidTransform,
    Scenegraph, UpAxis,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source_frame = MapFrame::z_up_meters("map-z", "Explicit origin and heading");
    let mut source = Scenegraph::new(MapDefinition {
        map_id: "portal-map-z".into(),
        name: Some("Convention conversion example".into()),
        domain_reference: None,
        frame: source_frame.clone(),
    })?;
    source.anchors.insert(
        "A".into(),
        QrAnchor {
            anchor_id: "A".into(),
            payload: "fixture-A".into(),
            side_length_m: 0.4,
            pose_in_map: RigidTransform {
                from_frame_id: "portal-A".into(),
                to_frame_id: "map-z".into(),
                translation: [1., 2., 3.],
                rotation_wxyz: [1., 0., 0., 0.],
            },
        },
    );
    let source_axes = FrameRegistryEntry {
        peer_id: "example-peer".into(),
        frame_id: "map-z".into(),
        handedness: auki_registry::Handedness::Right,
        axes: AxisConvention {
            x: AxisDirection::Right,
            y: AxisDirection::Forward,
            z: AxisDirection::Up,
        },
        units: LengthUnit::Meters,
    };
    let mut target_axes = FrameRegistryEntry::opengl("example-peer", "map-y");
    target_axes.units = LengthUnit::Centimeters;
    let target_frame = MapFrame {
        id: "map-y".into(),
        handedness: auki_scenegraph::Handedness::Right,
        up_axis: UpAxis::Y,
        meters_per_unit: 0.01,
        origin_description: "Same physical origin; X right, Y up, Z backward".into(),
    };
    let conversion = MapConventionConversion::from_registry_frames(
        source_frame,
        target_frame,
        &source_axes,
        &target_axes,
    )?;
    let converted = conversion.convert_snapshot(&MapSnapshot::new(source)?, "portal-map-y")?;
    print!("{}", converted.scenegraph.to_usda_with_portal_geometry()?);
    Ok(())
}
