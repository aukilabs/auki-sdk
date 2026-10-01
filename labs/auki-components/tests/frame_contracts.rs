use auki_components::{
    CameraComponent, Catalog, FrameRegistryEntry, OutputManifest, RegisteredFrame,
};
#[path = "support/clock.rs"]
mod clock_fixture;
fn output() -> OutputManifest {
    CameraComponent::new(
        "camera-peer",
        "camera",
        2,
        2,
        Catalog::default(),
        clock_fixture::fixture_clock("capture"),
        [],
        FrameRegistryEntry::ros_optical("frame-owner", "optical"),
    )
    .unwrap()
    .current_output_manifest()
}
#[test]
fn camera_carries_exact_existing_registry_definition() {
    let manifest = output();
    let frame = manifest.spatial_frame.as_ref().unwrap();
    assert_eq!(frame.reference.peer_id, "frame-owner");
    assert_eq!(frame.reference.id, "optical");
    assert_eq!(frame.reference.hash, frame.definition.hash());
    manifest.validate().unwrap();
    let decoded: OutputManifest =
        serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(decoded, manifest);
}
#[test]
fn bare_camera_frame_id_is_rejected() {
    let mut manifest = output();
    manifest.spatial_frame = None;
    assert!(manifest.validate().is_err());
}
#[test]
fn mismatched_owner_id_hash_and_axes_are_rejected() {
    let original = output();
    for mutation in 0..5 {
        let mut manifest = original.clone();
        let frame = manifest.spatial_frame.as_mut().unwrap();
        match mutation {
            0 => frame.reference.peer_id = "other".into(),
            1 => frame.reference.id = "other".into(),
            2 => frame.reference.hash = "0".repeat(32),
            3 => frame.definition.axes.x = frame.definition.axes.y,
            _ => manifest.spatial_frame_id = Some("other".into()),
        }
        assert!(manifest.validate().is_err());
    }
}
#[test]
fn convention_change_changes_registry_and_output_identity() {
    let first = output();
    let mut second = first.clone();
    second.spatial_frame = Some(RegisteredFrame::new(FrameRegistryEntry::opengl(
        "frame-owner",
        "optical",
    )));
    second.validate().unwrap();
    assert_ne!(first.hash(), second.hash());
    assert_ne!(
        first.spatial_frame.unwrap().reference.hash,
        second.spatial_frame.unwrap().reference.hash
    );
}
