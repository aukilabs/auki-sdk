use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use auki_components::{
    BufferLimits, CameraPayloadContract, ComponentManifest, ComponentRuntime, ComponentSpec,
    ContractType, CursorStart, Envelope, Exposure, InputPort, Observation, ObservationAccess,
    OperableContract, OutputManifest, PayloadContract, ProductForm, ProductInputContract,
    ProductManifest, RetainedProduct,
};

struct Frame(Vec<u8>);

impl ContractType for Frame {
    const DATATYPE: &'static str = "video_frame";
}

#[test]
fn imported_remote_product_binds_through_the_normal_typed_input_api() {
    let remote_component = ComponentManifest {
        schema: "auki.component-manifest/v1".to_owned(),
        peer_id: "remote-peer".to_owned(),
        component_id: "camera".to_owned(),
        product_inputs: vec![],
        observables: vec![],
        operables: vec![OperableContract {
            name: "unused".to_owned(),
            instruction: "unit".to_owned(),
            result: "unit".to_owned(),
            exposure: Exposure::Cluster,
        }],
    };
    let producer = OutputManifest {
        schema: "auki.component-output-manifest/v1".to_owned(),
        peer_id: "remote-peer".to_owned(),
        component_id: "camera".to_owned(),
        component_manifest_hash: remote_component.hash(),
        slot: "frames".to_owned(),
        output_id: "frames-1".to_owned(),
        clock_id: "remote-peer.clock".to_owned(),
        spatial_frame_id: None,
        payload: PayloadContract::Camera(CameraPayloadContract {
            datatype: "video_frame".to_owned(),
            schema: "test.frame/v1".to_owned(),
            encoding: "bytes".to_owned(),
            width: 1,
            height: 1,
            nominal_frame_rate_hz: Some(1),
            observes: "visible_light".to_owned(),
        }),
    };
    let product_manifest = ProductManifest {
        schema: "auki.product-manifest/v1".to_owned(),
        peer_id: "remote-peer".to_owned(),
        product_id: "remote-frame-history".to_owned(),
        form: ProductForm::Buffer,
        producer: producer.reference(),
        access: vec![
            ObservationAccess::LatestExisting,
            ObservationAccess::TimeRange,
        ],
    };
    let imported = RetainedProduct::<Frame>::imported_buffer(
        product_manifest.clone(),
        product_manifest.hash(),
        producer.clone(),
        BufferLimits::entries(4),
        |frame| frame.0.len(),
    )
    .unwrap();

    let runtime = ComponentRuntime::new("local-peer");
    let detector = runtime
        .component(
            ComponentSpec::new("detector").product_input(ProductInputContract {
                name: "frames".to_owned(),
                form: ProductForm::Buffer,
                datatype: "video_frame".to_owned(),
                schema: "test.frame/v1".to_owned(),
                exposure: Exposure::Cluster,
            }),
        )
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let input = InputPort::<Observation<Frame>>::new("detector.frames", move |envelope| {
        sink.lock().unwrap().push(envelope.payload.sequence);
    });
    let _binding = detector
        .configured_buffer_input("frames", &imported, CursorStart::FromSequence(7), &input)
        .unwrap();
    detector.expose().unwrap();

    let observation = Observation {
        output: producer.reference(),
        sequence: 7,
        timestamp_ns: 99,
        payload: Arc::new(Frame(vec![1, 2, 3])),
    };
    imported
        .buffer()
        .append_shared(Arc::new(Envelope::new(7, 99, observation)))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    while received.lock().unwrap().as_slice() != [7] {
        assert!(Instant::now() < deadline, "remote input delivery timed out");
        std::thread::yield_now();
    }

    let snapshot = runtime.catalog().snapshot();
    assert!(snapshot.products.is_empty());
    assert_eq!(
        snapshot.components[0].current_product_inputs["frames"]
            .manifest
            .product
            .peer_id,
        "remote-peer"
    );

    let mut second_producer = producer;
    second_producer.peer_id = "second-remote-peer".to_owned();
    let mut second_manifest = product_manifest;
    second_manifest.peer_id = "second-remote-peer".to_owned();
    second_manifest.producer = second_producer.reference();
    let second_imported = RetainedProduct::<Frame>::imported_buffer(
        second_manifest.clone(),
        second_manifest.hash(),
        second_producer,
        BufferLimits::entries(4),
        |frame| frame.0.len(),
    )
    .unwrap();
    let second_binding = detector
        .replace_configured_buffer_input(&_binding, &second_imported, CursorStart::Latest, &input)
        .expect("the same Product ID on a different Peer is a distinct Product");
    assert_eq!(
        second_binding.manifest().product.peer_id,
        "second-remote-peer"
    );
}
