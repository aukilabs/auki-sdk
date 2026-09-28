//! Export a self-contained sample; no network or device access.
use auki_scenegraph::{MapDefinition, MapFrame, QrAnchor, RigidTransform, Scenegraph};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut map = Scenegraph::new(MapDefinition {
        map_id: "example-qr-map".into(),
        name: Some("First QR map".into()),
        domain_reference: None,
        frame: MapFrame::z_up_meters(
            "example-map-frame",
            "Manually established origin and heading",
        ),
    })?;
    let anchor = QrAnchor {
        anchor_id: "qr-123".into(),
        payload: "auki:fixture-qr".into(),
        side_length_m: 0.2,
        pose_in_map: RigidTransform {
            from_frame_id: "qr-123-frame".into(),
            to_frame_id: "example-map-frame".into(),
            translation: [1.0, 2.0, 3.0],
            rotation_wxyz: [0.5, 0.5, 0.5, 0.5],
        },
    };
    map.anchors.insert(anchor.anchor_id.clone(), anchor);
    print!("{}", map.to_usda()?);
    Ok(())
}
