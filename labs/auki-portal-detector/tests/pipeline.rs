use auki_components::*;
use auki_domain_client::PortalId;
use auki_portal_detector::*;
use auki_qr_mapper::{PortalSizeError, ResolvedPortal, local::PortalMetadataSource};
use auki_scenegraph::{component::*, resolution::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct Metadata {
    calls: Arc<AtomicUsize>,
    gate: Arc<tokio::sync::Semaphore>,
    active: Arc<AtomicUsize>,
    mismatch: bool,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl PortalMetadataSource for Metadata {
    async fn resolve(
        &self,
        _: Uuid,
        _: &PortalId,
        _: &CancellationToken,
    ) -> Result<ResolvedPortal, PortalSizeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(self.active.clone());
        self.gate.acquire().await.unwrap().forget();
        let short = if self.mismatch {
            "ZZZ12345678"
        } else {
            "ABC12345678"
        };
        ResolvedPortal::try_from(serde_json::from_value::<auki_domain_client::Portal>(serde_json::json!({
            "id":"00000000-0000-0000-0000-000000000001", "short_id":short,
            "name":"fixture", "size":10.0, "organization_id":null, "default_domain_id":null,
            "redirect_url":null, "created_at":"2026-09-01T00:00:00Z", "updated_at":"2026-09-02T00:00:00Z"
        })).unwrap())
    }
}
fn metadata() -> Metadata {
    Metadata {
        calls: Arc::new(AtomicUsize::new(0)),
        gate: Arc::new(tokio::sync::Semaphore::new(0)),
        active: Arc::new(AtomicUsize::new(0)),
        mismatch: false,
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: "fixture".into(),
        caller_peer_id: "local".into(),
        caller_component_id: "host".into(),
    }
}
fn portal_map(runtime: &ComponentRuntime, id: &str, size: f64) -> Arc<MapComponent> {
    let ticks = AtomicU64::new(1);
    let map = Arc::new(
        MapComponent::new(
            runtime,
            MapComponentConfig {
                component_id: id.into(),
                publication_id: id.into(),
                clock: fixture_clock("map-clock"),
                map: MapDefinition {
                    map_id: id.into(),
                    name: None,
                    domain_reference: None,
                    frame: MapFrame::z_up_meters(format!("{id}-frame"), "explicit fixture frame"),
                },
            },
            move || ticks.fetch_add(1, Ordering::SeqCst),
            |_| true,
            |_| true,
        )
        .unwrap(),
    );
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            context(),
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor: QrAnchor {
                    anchor_id: "portal".into(),
                    payload: "ABC12345678".into(),
                    side_length_m: size,
                    pose_in_map: RigidTransform::identity("portal-frame", format!("{id}-frame")),
                },
            },
        )
        .unwrap();
    map
}
struct Fixture {
    runtime: ComponentRuntime,
    _camera: Component,
    _capture: BufferProductCapture<VideoFrame>,
    frames: ConfiguredObservable<VideoFrame>,
    detector: PortalDetector,
    results: BufferProductCapture<PortalDetection>,
    image: VideoFrame,
    maps: PortalMaps,
}
fn fixture(source: Metadata) -> Fixture {
    fixture_payload(source, "ABC12345678")
}
fn fixture_payload(source: Metadata, payload: &str) -> Fixture {
    let runtime = ComponentRuntime::new("local");
    let qr = qrcode::QrCode::new(payload).unwrap();
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
    let component = runtime
        .component(ComponentSpec::new("camera").observable(ObservableContract {
            name: "frames".into(),
            datatype: VideoFrame::DATATYPE.into(),
            schema: "auki.video-frame/v1".into(),
            access: vec![ObservationAccess::FollowNew],
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let frames = component
        .configured_observable(
            ConfiguredObservableSpec::new(
                "frames",
                "camera-output",
                fixture_clock("camera-clock"),
                PayloadContract::Camera(CameraPayloadContract {
                    datatype: VideoFrame::DATATYPE.into(),
                    schema: "auki.video-frame/v1".into(),
                    encoding: "rgb8".into(),
                    width: size as u32,
                    height: size as u32,
                    nominal_frame_rate_hz: Some(10),
                    observes: "fixture".into(),
                }),
            )
            .in_registered_frame(auki_components::FrameRegistryEntry::ros_optical(
                "camera-peer",
                "camera-optical",
            )),
        )
        .unwrap();
    component.expose().unwrap();
    let capture = runtime
        .capture_buffer(
            "camera-product",
            &frames,
            BufferLimits::entries(8),
            |f: &VideoFrame| f.bytes.len(),
        )
        .unwrap();
    let maps = PortalMaps::new(context());
    let ticks = AtomicU64::new(1);
    let detector = PortalDetector::bind(
        &runtime,
        "portal-detector",
        "run1",
        &capture.product(),
        maps.clone(),
        source,
        Uuid::nil(),
        fixture_clock("processing-clock"),
        move || ticks.fetch_add(1, Ordering::SeqCst),
    )
    .unwrap();
    let results = runtime
        .capture_buffer(
            "enriched",
            detector.portals(),
            BufferLimits::entries(256),
            |_| 4096,
        )
        .unwrap();
    Fixture {
        runtime,
        _camera: component,
        _capture: capture,
        frames,
        detector,
        results,
        maps,
        image: VideoFrame {
            width: size as u32,
            height: size as u32,
            encoding: "rgb8".into(),
            bytes: bytes.into(),
        },
    }
}
fn records(f: &Fixture) -> Vec<PortalDetection> {
    f.results
        .product()
        .buffer()
        .snapshot(0, u64::MAX)
        .iter()
        .map(|e| (*e.payload.payload).clone())
        .collect()
}
fn emit(f: &Fixture, time: u64) {
    f.frames.publish(time, Arc::new(f.image.clone())).unwrap();
}
#[test]
fn real_qr_scanning_continues_during_coalesced_lookup_and_preserves_identity() {
    let source = metadata();
    let mut f = fixture(source.clone());
    emit(&f, 100);
    emit(&f, 200);
    emit(&f, 300);
    wait(|| records(&f).len() == 3 && source.calls.load(Ordering::SeqCst) == 1);
    assert!(
        records(&f)
            .iter()
            .all(|r| matches!(r.size, PortalSize::Pending))
    );
    assert_eq!(
        f.detector
            .detection_product()
            .latest_existing()
            .unwrap()
            .unwrap()
            .sequence,
        2
    );
    source.gate.add_permits(1);
    wait(|| records(&f).len() == 6);
    let all = records(&f);
    for sequence in 0..3 {
        let matching: Vec<_> = all
            .iter()
            .filter(|r| r.detection_sequence == sequence)
            .collect();
        assert_eq!(matching.len(), 2);
        assert_eq!(matching[0].detection, matching[1].detection);
        assert_eq!(matching[1].source_frame.timestamp_ns, (sequence + 1) * 100);
        assert_eq!(matching[1].capture_clock, fixture_clock("camera-clock"));
        assert_eq!(
            matching[1].detection_product,
            f.detector.detection_product().reference()
        );
        assert!(
            matches!(matching[1].size, PortalSize::Database { side_length_m, .. } if side_length_m == 0.1)
        );
    }
    emit(&f, 400);
    wait(|| records(&f).len() == 7);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        records(&f).last().unwrap().size,
        PortalSize::Database { .. }
    ));
    assert_eq!(f.runtime.catalog().snapshot().components.len(), 2); // Camera + one Portal Detector.
    assert!(f.detector.last_error().is_none());
    assert!(f.results.errors().is_empty());
    f.detector.close();
    let count = records(&f).len();
    emit(&f, 500);
    assert_eq!(records(&f).len(), count);
}
#[test]
fn local_maps_override_database_and_conflicts_are_explicit() {
    let source = metadata();
    let f = fixture(source.clone());
    let a = portal_map(&f.runtime, "a", 0.2);
    f.maps.register(a.clone()).unwrap();
    emit(&f, 100);
    wait(|| records(&f).len() == 1);
    assert!(
        matches!(&records(&f)[0].size, PortalSize::LocalMaps {side_length_m, candidates} if *side_length_m==0.2 && candidates.len()==1)
    );
    // The existing one-shot localizer consumes this same component's retained raw
    // Product; enrichment supplies the exact candidate for explicit selection.
    let calibration = auki_qr_localizer::Calibration::for_product(
        "fixture-calibration",
        &f._capture.product(),
        auki_qr_localizer::Intrinsics {
            fx: 500.,
            fy: 500.,
            cx: f.image.width as f64 / 2.,
            cy: f.image.height as f64 / 2.,
            distortion: auki_qr_localizer::Distortion::None,
        },
    )
    .unwrap();
    let localizer = auki_qr_localizer::component::QrLocalizerComponent::bind(
        &f.runtime,
        "localizer",
        "localizer-output",
        &f.detector.detection_product(),
        Arc::new(f.maps.clone()),
        calibration,
        auki_qr_localizer::QualityGate::default(),
    )
    .unwrap();
    let record = &records(&f)[0];
    let localized = InMemoryTransport
        .invoke(
            localizer.localize_once(),
            context(),
            auki_qr_localizer::component::LocalizeOnce {
                detection_product: record.detection_product.clone(),
                detection_sequence: record.detection_sequence,
                detection_index: record.detection_index,
                target_map: a.snapshot_reference(),
            },
        )
        .unwrap()
        .result;
    assert_eq!(localized.estimates.len(), 1);
    assert_eq!(
        localized.estimates[0].pose.camera_pose_in_map.to_frame_id,
        "a-frame"
    );
    let b = portal_map(&f.runtime, "b", 0.3);
    f.maps.register(b).unwrap();
    emit(&f, 200);
    wait(|| records(&f).len() == 2);
    assert!(
        matches!(&records(&f)[1].size,PortalSize::Conflict {candidates} if candidates.len()==2)
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn lookup_completion_rechecks_maps_and_shutdown_cancels_pending_io() {
    let source = metadata();
    let mut f = fixture(source.clone());
    emit(&f, 100);
    wait(|| source.active.load(Ordering::SeqCst) == 1);
    let a = portal_map(&f.runtime, "a", 0.25);
    f.maps.register(a.clone()).unwrap();
    source.gate.add_permits(1);
    wait(|| records(&f).len() == 2);
    assert!(
        matches!(records(&f)[1].size,PortalSize::LocalMaps {side_length_m,..} if side_length_m==0.25)
    );
    f.maps.unregister(&a);
    f.detector.close();
    let source = metadata();
    let mut f = fixture(source.clone());
    emit(&f, 100);
    wait(|| source.active.load(Ordering::SeqCst) == 1);
    f.detector.close();
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
}
#[test]
fn mismatched_database_identity_is_unresolved_and_failure_is_cached() {
    let mut source = metadata();
    source.mismatch = true;
    source.gate.add_permits(1);
    let f = fixture(source.clone());
    emit(&f, 100);
    wait(|| records(&f).len() == 2);
    assert!(
        matches!(&records(&f)[1].size,PortalSize::Unresolved {reason} if reason.contains("identity mismatch"))
    );
    emit(&f, 200);
    wait(|| records(&f).len() == 3);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn arbitrary_qr_urls_do_not_trigger_database_requests() {
    let source = metadata();
    let f = fixture_payload(source.clone(), "https://example.com/not-a-portal");
    emit(&f, 100);
    wait(|| records(&f).len() == 1);
    assert!(matches!(records(&f)[0].size, PortalSize::Unresolved { .. }));
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn pending_observations_are_bounded_and_closed_maps_do_not_trigger_fallback() {
    let source = metadata();
    let mut f = fixture(source.clone());
    for index in 0..66 {
        emit(&f, (index + 1) * 100);
        wait(|| records(&f).len() > index as usize);
    }
    assert!(
        matches!(&records(&f)[65].size, PortalSize::Unresolved { reason } if reason.contains("capacity"))
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    f.detector.close();
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    let source = metadata();
    let f = fixture(source.clone());
    let map = portal_map(&f.runtime, "closed-map", 0.1);
    f.maps.register(map.clone()).unwrap();
    map.close();
    emit(&f, 100);
    wait(|| records(&f).len() == 1);
    assert!(matches!(records(&f)[0].size, PortalSize::Unresolved { .. }));
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
