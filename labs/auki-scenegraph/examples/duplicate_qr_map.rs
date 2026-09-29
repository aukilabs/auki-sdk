//! Offline: duplicate an OpenGL Portal scene (or just B) into ROS2 body axes.
use auki_scenegraph::{
    CoordinateConvention, MapDefinition, MapDuplicateTarget, MapFrame, MapSnapshot, QrAnchor,
    RigidTransform, Scenegraph,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut scene = Scenegraph::new(MapDefinition {
        map_id: "opengl-map".into(),
        name: Some("OpenGL fixture".into()),
        domain_reference: None,
        frame: MapFrame::in_convention(
            "gl-frame",
            "fixture origin",
            CoordinateConvention::OpenGl,
            1.,
        )?,
    })?;
    for (id, translation) in [("A", [1., 2., 3.]), ("B", [-4., 5., -6.])] {
        scene.anchors.insert(
            id.into(),
            QrAnchor {
                anchor_id: id.into(),
                payload: format!("fixture-{id}"),
                side_length_m: 0.4,
                pose_in_map: RigidTransform {
                    from_frame_id: format!("portal-{id}-frame"),
                    to_frame_id: "gl-frame".into(),
                    translation,
                    rotation_wxyz: [1., 0., 0., 0.],
                },
            },
        );
    }
    let source = MapSnapshot::new(scene)?;
    let target = MapDuplicateTarget::new("ros-map", "ros-frame", CoordinateConvention::Ros2Body);
    let copy = if std::env::args().any(|a| a == "--only-b") {
        source.duplicate_part_in_convention(&["B"], &target)?
    } else {
        source.duplicate_in_convention(&target)?
    };
    print!("{}", copy.scenegraph.to_usda_with_portal_geometry()?);
    Ok(())
}
