#![cfg(feature = "qr-detector")]
use auki_components::*;
use auki_qr_detector::*;
use auki_qr_localizer::{component::*, *};
use auki_scenegraph::{component::*, resolution::*, *};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn output<T: ContractType + Send + Sync + 'static>(
    runtime: &ComponentRuntime,
    id: &str,
    frame: &str,
    payload: PayloadContract,
) -> (Component, ConfiguredObservable<T>) {
    let c = runtime
        .component(ComponentSpec::new(id).observable(ObservableContract {
            name: "out".into(),
            datatype: T::DATATYPE.into(),
            schema: payload.schema().into(),
            access: vec![ObservationAccess::FollowNew],
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let o = c
        .configured_observable(
            ConfiguredObservableSpec::new(
                "out",
                format!("{id}-out"),
                fixture_clock("clock"),
                payload,
            )
            .in_registered_frame(auki_components::FrameRegistryEntry::ros_optical(
                "camera-peer",
                frame,
            )),
        )
        .unwrap();
    c.expose().unwrap();
    (c, o)
}
fn wait<T: Clone + Send + Sync + 'static>(
    product: &RetainedProduct<T>,
    sequence: u64,
) -> Observation<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(o) = product.latest_existing().unwrap()
            && o.sequence >= sequence
        {
            return o;
        }
        assert!(Instant::now() < deadline, "timed out waiting for output");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn real_detector_localizes_camera_and_empty_frame_has_no_stale_pose() {
    let runtime = ComponentRuntime::new("local");
    let qr = qrcode::QrCode::new("test-qr").unwrap();
    let modules = qr.width();
    let size = (modules + 8) * 10;
    let mut bytes = vec![255u8; size * size * 3];
    for y in 0..modules {
        for x in 0..modules {
            if qr[(x, y)] == qrcode::Color::Dark {
                for dy in 0..10 {
                    for dx in 0..10 {
                        let at = ((y * 10 + 40 + dy) * size + x * 10 + 40 + dx) * 3;
                        bytes[at..at + 3].fill(0);
                    }
                }
            }
        }
    }
    let (_camera, frames) = output::<VideoFrame>(
        &runtime,
        "camera",
        "optical",
        PayloadContract::Camera(CameraPayloadContract {
            datatype: VideoFrame::DATATYPE.into(),
            schema: "auki.video-frame/v1".into(),
            encoding: "rgb8".into(),
            width: size as u32,
            height: size as u32,
            nominal_frame_rate_hz: Some(10),
            observes: "fixture".into(),
        }),
    );
    let frames_capture = runtime
        .capture_buffer("frames", &frames, BufferLimits::entries(4), |f| {
            f.bytes.len()
        })
        .unwrap();
    let calibration = Calibration::for_product(
        "cal",
        &frames_capture.product(),
        Intrinsics {
            fx: 500.,
            fy: 500.,
            cx: size as f64 / 2.,
            cy: size as f64 / 2.,
            distortion: Distortion::None,
        },
    )
    .unwrap();
    let detector = QrDetector::default()
        .bind_product(
            &runtime,
            "detector",
            "detector-out",
            &frames_capture.product(),
        )
        .unwrap();
    let detections = runtime
        .capture_buffer(
            "detections",
            detector.detections(),
            BufferLimits::entries(4),
            |_| 1024,
        )
        .unwrap();
    let clock = Arc::new(AtomicU64::new(1));
    let map = Arc::new(
        MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: "map".into(),
                publication_id: "map-run".into(),
                clock: fixture_clock("clock"),
                map: MapDefinition {
                    map_id: "map".into(),
                    name: None,
                    domain_reference: Some("store".into()),
                    frame: MapFrame::z_up_meters("map-frame", "QR origin"),
                },
            },
            move || clock.fetch_add(1, Ordering::SeqCst),
            |_| true,
            |_| true,
        )
        .unwrap(),
    );
    let context = InvocationContext {
        invocation_id: "test".into(),
        caller_peer_id: "local".into(),
        caller_component_id: "mapper".into(),
    };
    let maps = PortalMaps::new(context.clone());
    maps.register(map.clone()).unwrap();
    let mut localizer = QrLocalizerComponent::bind_with_controls(
        &runtime,
        "localizer",
        "localizer-out",
        &detections.product(),
        Arc::new(maps.clone()),
        calibration,
        QualityGate::default(),
        false,
        |_| true,
    )
    .unwrap();
    assert!(localizer.poses().manifest().spatial_frame_id.is_none());
    let poses = runtime
        .capture_buffer("poses", localizer.poses(), BufferLimits::entries(4), |_| {
            4096
        })
        .unwrap();
    frames
        .publish(
            100,
            Arc::new(VideoFrame {
                width: size as u32,
                height: size as u32,
                encoding: "rgb8".into(),
                bytes: bytes.clone().into(),
            }),
        )
        .unwrap();
    let unknown = wait(&poses.product(), 0);
    assert_eq!(unknown.payload.needs_mapping, vec![0]);
    assert!(unknown.payload.estimates.is_empty());
    let expected_snapshot = InMemoryTransport
        .invoke(
            map.upsert_qr(),
            context,
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor: QrAnchor {
                    anchor_id: "qr".into(),
                    payload: "test-qr".into(),
                    side_length_m: 0.2,
                    pose_in_map: RigidTransform::identity("qr-frame", "map-frame"),
                },
            },
        )
        .unwrap()
        .result;
    // The same running localizer sees the accepted local anchor on the next observation.
    frames
        .publish(
            150,
            Arc::new(VideoFrame {
                width: size as u32,
                height: size as u32,
                encoding: "rgb8".into(),
                bytes: bytes.into(),
            }),
        )
        .unwrap();
    let first = wait(&poses.product(), 1);
    let batch = &first.payload;
    assert!(batch.rejection.is_none(), "{batch:?}");
    assert_eq!(batch.estimates.len(), 1, "{batch:?}");
    let expected_z = 500. * 0.2 / (modules * 10) as f64;
    assert!(
        (batch.estimates[0].pose.camera_pose_in_map.translation[2] - expected_z).abs() < 0.01,
        "{batch:?}"
    );
    assert_eq!(batch.estimates[0].qr.snapshot, expected_snapshot);
    assert_eq!(batch.timestamp_ns, 150);
    assert!(batch.needs_mapping.is_empty());
    assert_eq!(
        batch.source_frame.as_ref().unwrap().camera_product,
        frames_capture.product().reference()
    );
    frames
        .publish(
            200,
            Arc::new(VideoFrame {
                width: size as u32,
                height: size as u32,
                encoding: "rgb8".into(),
                bytes: vec![255; size * size * 3].into(),
            }),
        )
        .unwrap();
    let empty = wait(&poses.product(), 2);
    assert!(empty.payload.estimates.is_empty());
    assert!(empty.payload.rejection.is_none());
    assert_eq!(empty.timestamp_ns, 200);
    localizer.close(201).unwrap();
    assert!(localizer.input().is_none());
    assert!(localizer.poses().publish(202, empty.payload).is_err());
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
