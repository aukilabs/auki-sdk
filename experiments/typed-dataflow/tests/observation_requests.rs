use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    CameraBufferRoller, CameraComponent, EveryFullPolicy, InMemoryTransport, InvocationContext,
    ObservationAccess, ObservationDelivery, ObservationError, ObservationEvent, ObservationStatus,
    PeerRuntime, ProductAccessError, SerializedInMemoryTransport, SetResolution, TimeRangeRequest,
    VideoFrame, observation_input,
};

fn frame_bytes(width: u32, height: u32, value: u8) -> Arc<[u8]> {
    Arc::from(vec![value; width as usize * height as usize * 3])
}

fn camera() -> (PeerRuntime, CameraComponent) {
    let peer = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer.peer_id(),
        "front-camera",
        2,
        2,
        peer.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    (peer, camera)
}

fn resize(camera: &CameraComponent, timestamp_ns: u64) {
    InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            InvocationContext {
                invocation_id: format!("resize-{timestamp_ns}"),
                caller_peer_id: "peer-b".to_owned(),
                caller_component_id: "camera-controller".to_owned(),
            },
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: timestamp_ns,
            },
        )
        .unwrap();
}

#[test]
fn fresh_camera_truthfully_supports_follow_new_only() {
    let (peer, camera) = camera();
    let observable = camera.current_output();

    assert_eq!(
        observable.supported_access(),
        &[ObservationAccess::FollowNew]
    );
    assert_eq!(
        observable.latest_existing().unwrap_err(),
        ObservationError::UnsupportedRequest(ObservationAccess::LatestExisting)
    );
    assert_eq!(
        observable
            .time_range(TimeRangeRequest {
                clock_id: "peer-a.session-clock".to_owned(),
                start_ns: 0,
                end_ns: 10,
            })
            .unwrap_err(),
        ObservationError::UnsupportedRequest(ObservationAccess::TimeRange)
    );

    let catalog = peer.catalog().component("front-camera").unwrap();
    assert_eq!(
        catalog.manifest.observables[0].access,
        vec![ObservationAccess::FollowNew]
    );
    assert_eq!(
        catalog.current_outputs["frames"].manifest.payload.kind,
        "camera"
    );
    assert_eq!(
        catalog.current_outputs["frames"].manifest.clock_id,
        "peer-a.session-clock"
    );
}

#[test]
fn retained_product_answers_latest_and_time_range_without_becoming_a_component() {
    let (peer, camera) = camera();
    let buffers = CameraBufferRoller::attach(&camera, 8).unwrap();

    camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    camera.publish_rgb8(20, 2, 2, frame_bytes(2, 2, 2)).unwrap();
    camera.publish_rgb8(30, 2, 2, frame_bytes(2, 2, 3)).unwrap();

    let product = buffers.current();
    assert_eq!(
        product.manifest.access,
        vec![
            ObservationAccess::LatestExisting,
            ObservationAccess::TimeRange,
        ]
    );
    assert_eq!(product.latest_existing().unwrap().unwrap().sequence, 2);

    let selected = product
        .time_range(TimeRangeRequest {
            clock_id: "peer-a.session-clock".to_owned(),
            start_ns: 15,
            end_ns: 25,
        })
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected.observations[0].sequence, 1);

    assert!(
        peer.catalog()
            .product(&product.manifest.product_id)
            .is_some()
    );
    assert!(
        peer.catalog()
            .component(&product.manifest.product_id)
            .is_none()
    );
    assert_eq!(peer.catalog().products().len(), 1);
}

#[test]
fn retained_time_range_rejects_wrong_clock_and_invalid_bounds() {
    let (_peer, camera) = camera();
    let buffers = CameraBufferRoller::attach(&camera, 8).unwrap();
    camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    let product = buffers.current();

    assert!(matches!(
        product.time_range(TimeRangeRequest {
            clock_id: "some-other-clock".to_owned(),
            start_ns: 0,
            end_ns: 20,
        }),
        Err(ProductAccessError::ClockMismatch { .. })
    ));
    assert_eq!(
        product
            .time_range(TimeRangeRequest {
                clock_id: "peer-a.session-clock".to_owned(),
                start_ns: 20,
                end_ns: 10,
            })
            .unwrap_err(),
        ProductAccessError::InvalidTimeRange {
            start_ns: 20,
            end_ns: 10,
        }
    );
}

#[test]
fn pinned_handle_becomes_reconfigured_and_disconnects() {
    let (_peer, camera) = camera();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let input = observation_input("pinned", move |event| {
        sink.lock().unwrap().push(event.clone());
    });
    let handle = camera
        .current_output()
        .follow_new(&input, ObservationDelivery::inline_every_selected())
        .unwrap();

    camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    resize(&camera, 20);

    let status = handle.status();
    let ObservationStatus::Reconfigured(transition) = status else {
        panic!("expected reconfigured handle, got {status:?}");
    };
    assert_eq!(transition.previous.output_id, "frames-1");
    assert_eq!(transition.replacement.output_id, "frames-2");
    assert!(handle.stats().closed);

    camera.publish_rgb8(21, 1, 1, frame_bytes(1, 1, 2)).unwrap();
    assert_eq!(events.lock().unwrap().len(), 2);
}

#[test]
fn follow_current_stays_active_until_explicitly_cancelled() {
    let (_peer, camera) = camera();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let input = observation_input("follow-current", move |event| {
        sink.lock().unwrap().push(event.clone());
    });
    let handle = camera
        .follow_current_output()
        .follow_new(&input, ObservationDelivery::inline_every_selected())
        .unwrap();

    camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    resize(&camera, 20);
    camera.publish_rgb8(21, 1, 1, frame_bytes(1, 1, 2)).unwrap();

    assert_eq!(handle.status(), ObservationStatus::Active);
    assert_eq!(events.lock().unwrap().len(), 3);
    handle.cancel();
    assert_eq!(handle.status(), ObservationStatus::Cancelled);
    camera.publish_rgb8(22, 1, 1, frame_bytes(1, 1, 3)).unwrap();
    assert_eq!(events.lock().unwrap().len(), 3);
}

#[test]
fn dropping_handle_releases_the_observation_relationship() {
    let (_peer, camera) = camera();
    let delivered = Arc::new(Mutex::new(0_u64));
    let sink = Arc::clone(&delivered);
    let input = observation_input("drop-test", move |_event: &ObservationEvent<VideoFrame>| {
        *sink.lock().unwrap() += 1;
    });
    let handle = camera
        .current_output()
        .follow_new(&input, ObservationDelivery::inline_every_selected())
        .unwrap();
    camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    assert_eq!(*delivered.lock().unwrap(), 1);
    drop(handle);
    camera.publish_rgb8(11, 2, 2, frame_bytes(2, 2, 2)).unwrap();
    assert_eq!(*delivered.lock().unwrap(), 1);
}

#[test]
fn serialized_follow_preserves_values_but_not_local_allocation_identity() {
    let (_peer, camera) = camera();
    let transport = SerializedInMemoryTransport::default();
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    let input = observation_input("remote-preview", move |event| {
        sink.lock().unwrap().push(event.clone());
    });
    let handle = transport
        .follow_new(
            &camera.current_output(),
            &input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();

    let source = camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 7)).unwrap();
    let events = received.lock().unwrap();
    let ObservationEvent::Observation(remote) = &events[0] else {
        panic!("expected transported observation");
    };
    assert_eq!(remote.output, source.output);
    assert_eq!(remote.sequence, source.sequence);
    assert_eq!(remote.payload.bytes.as_ref(), source.payload.bytes.as_ref());
    assert_ne!(
        Arc::as_ptr(&remote.payload.bytes),
        Arc::as_ptr(&source.payload.bytes)
    );
    drop(events);

    let stats = handle.stats();
    assert_eq!(stats.transport.encoded_messages, 1);
    assert_eq!(stats.transport.decoded_messages, 1);
    assert!(stats.transport.encoded_bytes > source.payload.bytes.len() as u64);
}

#[test]
fn serialized_product_queries_report_transport_work() {
    let (_peer, camera) = camera();
    let buffers = CameraBufferRoller::attach(&camera, 8).unwrap();
    let source = camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 5)).unwrap();
    let product = buffers.current();
    let transport = SerializedInMemoryTransport::default();

    let remote = transport.latest_existing(&product).unwrap().unwrap();
    assert_eq!(remote.output, source.output);
    assert_eq!(remote.payload.bytes.as_ref(), source.payload.bytes.as_ref());
    assert_ne!(
        Arc::as_ptr(&remote.payload.bytes),
        Arc::as_ptr(&source.payload.bytes)
    );

    let selected = transport
        .time_range(
            &product,
            TimeRangeRequest {
                clock_id: "peer-a.session-clock".to_owned(),
                start_ns: 0,
                end_ns: 10,
            },
        )
        .unwrap();
    assert_eq!(selected.len(), 1);
    let stats = transport.stats();
    assert_eq!(stats.encoded_messages, 4);
    assert_eq!(stats.decoded_messages, 4);
    assert!(stats.encoded_bytes > source.payload.bytes.len() as u64);
}

#[test]
fn coalesce_latest_reports_replaced_selected_observations() {
    let (_peer, camera) = camera();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let first = Arc::new(AtomicBool::new(true));
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let input = {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        let first = Arc::clone(&first);
        let delivered = Arc::clone(&delivered);
        observation_input("slow-preview", move |event| {
            if first.swap(false, Ordering::AcqRel) {
                entered.wait();
                release.wait();
            }
            if let ObservationEvent::Observation(observation) = event {
                delivered.lock().unwrap().push(observation.sequence);
            }
        })
    };
    let handle = camera
        .current_output()
        .follow_new(&input, ObservationDelivery::coalesce_latest())
        .unwrap();

    camera.publish_rgb8(0, 2, 2, frame_bytes(2, 2, 0)).unwrap();
    entered.wait();
    for sequence in 1..32_u64 {
        camera
            .publish_rgb8(sequence, 2, 2, frame_bytes(2, 2, sequence as u8))
            .unwrap();
    }
    release.wait();

    let deadline = Instant::now() + Duration::from_secs(2);
    while handle.stats().delivered < 2 && Instant::now() < deadline {
        thread::yield_now();
    }
    let stats = handle.stats();
    assert_eq!(stats.accepted, 32);
    assert!(stats.coalesced > 0);
    assert_eq!(delivered.lock().unwrap().last().copied(), Some(31));
}

#[test]
fn queued_every_selected_keeps_its_explicit_delivery_guarantee() {
    let (_peer, camera) = camera();
    let sequences = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&sequences);
    let input = observation_input("queued-preview", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            sink.lock().unwrap().push(observation.sequence);
        }
    });
    let handle = camera
        .current_output()
        .follow_new(
            &input,
            ObservationDelivery::queued_every_selected(4, EveryFullPolicy::Backpressure),
        )
        .unwrap();

    for sequence in 0..16_u64 {
        camera
            .publish_rgb8(sequence, 2, 2, frame_bytes(2, 2, sequence as u8))
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while handle.stats().delivered < 16 && Instant::now() < deadline {
        thread::yield_now();
    }
    assert_eq!(*sequences.lock().unwrap(), (0..16).collect::<Vec<_>>());
    assert_eq!(handle.stats().coalesced, 0);
}
