use auki_components::{
    BufferLimits, CameraBufferCapture, CameraComponent, Catalog, ClockReference, OutputManifest,
    ProductAccessError, ProductImportError, RetainedProduct, TimeRangeRequest, VideoFrame,
};
#[path = "support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;

fn camera(clock: ClockReference) -> CameraComponent {
    CameraComponent::new(
        "camera-peer",
        "camera",
        1,
        1,
        Catalog::default(),
        clock,
        [],
        auki_components::FrameRegistryEntry::ros_optical("camera-peer", "camera.optical"),
    )
    .unwrap()
}

#[test]
fn time_queries_compare_owner_and_definition_not_just_clock_name() {
    let clock = fixture_clock("capture");
    let camera = camera(clock.clone());
    let capture = CameraBufferCapture::attach(&camera, 2).unwrap();
    camera.publish_rgb8(7, 1, 1, vec![0; 3].into()).unwrap();
    let product = capture.product();
    for field in ["owner", "boot"] {
        let mut other = clock.clone();
        if field == "owner" {
            other.peer_id = "another-peer".into();
        } else {
            other.hash = "00000000000000000000000000000000".into();
        }
        assert!(matches!(
            product.time_range(TimeRangeRequest {
                clock: other,
                start_ns: 0,
                end_ns: 10
            }),
            Err(ProductAccessError::ClockMismatch { .. })
        ));
    }
    assert_eq!(
        product
            .time_range(TimeRangeRequest {
                clock,
                start_ns: 0,
                end_ns: 10
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn missing_or_malformed_clock_cannot_be_imported_even_with_matching_manifest_hash() {
    let camera = camera(fixture_clock("capture"));
    let capture = CameraBufferCapture::attach(&camera, 2).unwrap();
    let product = capture.product();
    let mut json = serde_json::to_value(&product.producer).unwrap();
    json.as_object_mut().unwrap().remove("clock");
    assert!(serde_json::from_value::<OutputManifest>(json).is_err());
    for field in ["peer", "id", "hash", "schema"] {
        let mut producer = product.producer.clone();
        match field {
            "peer" => producer.clock.peer_id.clear(),
            "id" => producer.clock.id.clear(),
            "hash" => producer.clock.hash = "unregistered".into(),
            _ => producer.schema = "auki.component-output-manifest/v1".into(),
        }
        let mut manifest = product.manifest.clone();
        manifest.producer = producer.reference();
        assert!(matches!(
            RetainedProduct::<VideoFrame>::imported_buffer(
                manifest.clone(),
                manifest.hash(),
                producer,
                BufferLimits::entries(2),
                |_| 3
            ),
            Err(ProductImportError::InvalidProducer(_))
        ));
    }
}

#[test]
fn imported_product_preserves_source_clock_without_relabelling_to_receiver() {
    let camera = camera(fixture_clock("capture"));
    let capture = CameraBufferCapture::attach(&camera, 2).unwrap();
    let source = capture.product();
    let imported = RetainedProduct::<VideoFrame>::imported_buffer(
        source.manifest.clone(),
        source.manifest_hash.clone(),
        source.producer.clone(),
        BufferLimits::entries(2),
        |_| 3,
    )
    .unwrap();
    assert_eq!(imported.producer.clock, source.producer.clock);
    assert_ne!(imported.producer.clock.peer_id, imported.producer.peer_id);
}

#[test]
fn camera_refuses_invalid_clock_before_registering_component() {
    let catalog = Catalog::default();
    let mut clock = fixture_clock("capture");
    clock.peer_id.clear();
    assert!(
        CameraComponent::new(
            "peer",
            "camera",
            1,
            1,
            catalog.clone(),
            clock,
            [],
            auki_components::FrameRegistryEntry::ros_optical("camera-peer", "camera.optical")
        )
        .is_err()
    );
    assert!(catalog.snapshot().components.is_empty());
}

#[test]
fn configured_observables_reject_invalid_clock_identity() {
    use auki_components::{
        ComponentBuildError, ComponentRuntime, ComponentSpec, ConfiguredObservableSpec,
    };
    let camera = camera(fixture_clock("capture"));
    let runtime = ComponentRuntime::new("peer");
    let component = runtime
        .component(
            ComponentSpec::new("source")
                .observable(camera.component_manifest().observables[0].clone()),
        )
        .unwrap();
    let mut clock = fixture_clock("capture");
    clock.hash.clear();
    assert!(matches!(
        component.configured_observable::<VideoFrame>(ConfiguredObservableSpec::new(
            "frames",
            "output",
            clock,
            camera.current_output_manifest().payload
        )),
        Err(ComponentBuildError::InvalidClock(_))
    ));
}
