use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    Buffer, CameraBufferRoller, CameraComponent, CameraError, ConnectionOptions, CursorRead,
    CursorStart, EveryFullPolicy, InputPort, ObservationEvent, ObservationStatus, OutputPort,
    PeerRuntime, SerializedInMemoryTransport, SharedDelivery, SharedScheduler, VideoFrame, connect,
    connect_buffer, connect_shared, observation_input,
};

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::yield_now();
        thread::sleep(Duration::from_millis(1));
    }
}

fn rgb8(width: u32, height: u32, value: u8) -> Arc<[u8]> {
    Arc::from(vec![value; width as usize * height as usize * 3])
}

#[test]
fn fixed_worker_pool_serves_many_relationships_and_isolates_one_blocked_observer() {
    const RELATIONSHIPS: usize = 256;
    let scheduler = SharedScheduler::new(4).unwrap();
    let dispatcher = scheduler.dispatcher();
    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let blocker_entered = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicUsize::new(0));
    let worker_names = Arc::new(Mutex::new(BTreeSet::new()));
    let mut connections = Vec::with_capacity(RELATIONSHIPS);

    for index in 0..RELATIONSHIPS {
        let gate = Arc::clone(&gate);
        let blocker_entered = Arc::clone(&blocker_entered);
        let delivered = Arc::clone(&delivered);
        let worker_names = Arc::clone(&worker_names);
        let input = InputPort::new(format!("observer-{index}"), move |_: &_| {
            worker_names.lock().unwrap().insert(
                thread::current()
                    .name()
                    .unwrap_or("unnamed-worker")
                    .to_owned(),
            );
            if index == 0 {
                blocker_entered.store(true, Ordering::Release);
                let (lock, changed) = &*gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
            }
            delivered.fetch_add(1, Ordering::Relaxed);
        });
        connections.push(
            connect_shared(
                &output,
                &input,
                &dispatcher,
                SharedDelivery::coalesce_latest(),
            )
            .unwrap(),
        );
    }

    output.publish(0, 7u64);
    wait_until(Duration::from_secs(2), || {
        blocker_entered.load(Ordering::Acquire)
    });
    wait_until(Duration::from_secs(2), || {
        delivered.load(Ordering::Relaxed) == RELATIONSHIPS - 1
    });

    let stats = scheduler.stats();
    assert_eq!(stats.worker_count, 4);
    assert!(stats.max_active <= 4);
    assert!(worker_names.lock().unwrap().len() <= 4);

    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    wait_until(Duration::from_secs(2), || {
        delivered.load(Ordering::Relaxed) == RELATIONSHIPS
    });
    assert!(
        connections
            .iter()
            .all(|connection| !connection.stats().failed)
    );
}

#[test]
fn shared_every_preserves_accepted_values_while_shared_latest_coalesces() {
    let scheduler = SharedScheduler::new(2).unwrap();
    let dispatcher = scheduler.dispatcher();
    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let every_seen = Arc::new(Mutex::new(Vec::new()));
    let latest_seen = Arc::new(Mutex::new(Vec::new()));

    let every_gate = Arc::clone(&gate);
    let every_entered = Arc::clone(&entered);
    let every_values = Arc::clone(&every_seen);
    let every = InputPort::new("every", move |value: &_| {
        every_values.lock().unwrap().push(value.sequence);
        if value.sequence == 0 {
            every_entered.fetch_add(1, Ordering::Release);
            let (lock, changed) = &*every_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });
    let latest_gate = Arc::clone(&gate);
    let latest_entered = Arc::clone(&entered);
    let latest_values = Arc::clone(&latest_seen);
    let latest = InputPort::new("latest", move |value: &_| {
        latest_values.lock().unwrap().push(value.sequence);
        if value.sequence == 0 {
            latest_entered.fetch_add(1, Ordering::Release);
            let (lock, changed) = &*latest_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });

    let every_connection = connect_shared(
        &output,
        &every,
        &dispatcher,
        SharedDelivery::every_selected(16, EveryFullPolicy::Backpressure),
    )
    .unwrap();
    let latest_connection = connect_shared(
        &output,
        &latest,
        &dispatcher,
        SharedDelivery::coalesce_latest(),
    )
    .unwrap();

    output.publish(0, 0u64);
    wait_until(Duration::from_secs(1), || {
        entered.load(Ordering::Acquire) == 2
    });
    for value in 1..10u64 {
        output.publish(value, value);
    }
    assert_eq!(latest_connection.stats().replaced, 8);

    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    wait_until(Duration::from_secs(2), || {
        every_connection.stats().delivered == 10 && latest_connection.stats().delivered == 2
    });

    assert_eq!(*every_seen.lock().unwrap(), (0..10).collect::<Vec<_>>());
    assert_eq!(*latest_seen.lock().unwrap(), vec![0, 9]);
}

#[test]
fn observer_errors_and_panics_become_failed_handles_without_harming_healthy_observers() {
    let scheduler = SharedScheduler::new(3).unwrap();
    let dispatcher = scheduler.dispatcher();
    let peer = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer.peer_id(),
        "camera",
        1,
        1,
        peer.catalog().clone(),
        Vec::<String>::new(),
    )
    .unwrap();
    let observable = camera.current_output();

    let rejected = InputPort::try_new("rejecting-detector", |_: &_| {
        Err("decoder rejected frame".to_owned())
    });
    let panicking = InputPort::new("panicking-detector", |_: &_| {
        panic!("detector invariant violated");
    });
    let healthy_count = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&healthy_count);
    let healthy = observation_input("healthy-detector", move |_| {
        count.fetch_add(1, Ordering::Relaxed);
    });

    let rejected_handle = observable
        .follow_new_shared(&rejected, &dispatcher, SharedDelivery::coalesce_latest())
        .unwrap();
    let panicking_handle = observable
        .follow_new_shared(&panicking, &dispatcher, SharedDelivery::coalesce_latest())
        .unwrap();
    let healthy_handle = observable
        .follow_new_shared(&healthy, &dispatcher, SharedDelivery::coalesce_latest())
        .unwrap();

    camera.publish_rgb8(1, 1, 1, rgb8(1, 1, 3)).unwrap();
    wait_until(Duration::from_secs(2), || {
        matches!(rejected_handle.status(), ObservationStatus::Failed(_))
            && matches!(panicking_handle.status(), ObservationStatus::Failed(_))
            && healthy_count.load(Ordering::Relaxed) == 1
    });

    let ObservationStatus::Failed(rejected_reason) = rejected_handle.status() else {
        panic!("rejecting observer must fail");
    };
    assert!(rejected_reason.contains("decoder rejected frame"));
    let ObservationStatus::Failed(panic_reason) = panicking_handle.status() else {
        panic!("panicking observer must fail");
    };
    assert!(panic_reason.contains("detector invariant violated"));
    assert_eq!(scheduler.stats().job_panics, 0);
    assert_eq!(healthy_handle.status(), ObservationStatus::Active);

    camera.publish_rgb8(2, 1, 1, rgb8(1, 1, 4)).unwrap();
    wait_until(Duration::from_secs(1), || {
        healthy_count.load(Ordering::Relaxed) == 2
    });
    assert_eq!(rejected_handle.stats().delivered, 0);
    assert_eq!(panicking_handle.stats().delivered, 0);
}

#[test]
fn producer_failure_is_terminal_and_closes_its_buffer_product() {
    let scheduler = SharedScheduler::new(2).unwrap();
    let dispatcher = scheduler.dispatcher();
    let peer = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer.peer_id(),
        "camera",
        1,
        1,
        peer.catalog().clone(),
        Vec::<String>::new(),
    )
    .unwrap();
    let products = CameraBufferRoller::attach(&camera, 4).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    let observer = observation_input("observer", move |event| {
        seen.lock().unwrap().push(event.clone());
    });
    let handle = camera
        .current_output()
        .follow_new_shared(
            &observer,
            &dispatcher,
            SharedDelivery::every_selected(4, EveryFullPolicy::Backpressure),
        )
        .unwrap();

    camera.publish_rgb8(1, 1, 1, rgb8(1, 1, 3)).unwrap();
    assert!(camera.fail_current_output(2, "camera driver disconnected"));
    assert!(!camera.fail_current_output(3, "duplicate failure"));
    wait_until(Duration::from_secs(1), || {
        matches!(handle.status(), ObservationStatus::Failed(_))
    });
    assert!(matches!(
        camera.publish_rgb8(4, 1, 1, rgb8(1, 1, 4)),
        Err(CameraError::OutputFailed(reason)) if reason == "camera driver disconnected"
    ));

    let product = products.current();
    assert!(product.latest_existing().unwrap().is_some());
    let mut cursor = product.buffer.subscribe(CursorStart::Latest);
    assert!(matches!(
        cursor.next_timeout(Duration::from_millis(10)),
        CursorRead::Closed
    ));
    assert!(matches!(
        events.lock().unwrap().last(),
        Some(ObservationEvent::Failed(_))
    ));
}

#[test]
fn buffer_eviction_releases_ownership_without_invalidating_an_existing_lease() {
    let buffer = Buffer::new("one-frame-buffer", 1).unwrap();
    let output = OutputPort::new("camera.frames");
    let _retention = connect_buffer(&output, &buffer);

    let first: Arc<[u8]> = Arc::from(vec![1u8; 1024]);
    let first_weak = Arc::downgrade(&first);
    output.publish(0, first);
    let leased = buffer.snapshot(0, 0).pop().unwrap();

    output.publish(1, Arc::<[u8]>::from(vec![2u8; 1024]));
    assert_eq!(buffer.range().first_sequence, Some(1));
    assert_eq!(leased.payload[0], 1);
    assert!(first_weak.upgrade().is_some());

    drop(leased);
    assert!(first_weak.upgrade().is_none());
}

#[test]
fn cancelling_a_shared_connection_releases_pending_payloads() {
    let scheduler = SharedScheduler::new(1).unwrap();
    let dispatcher = scheduler.dispatcher();
    let output = OutputPort::new("source.payloads");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let input_gate = Arc::clone(&gate);
    let input_entered = Arc::clone(&entered);
    let input = InputPort::new("blocked", move |value: &_| {
        if value.sequence == 0 {
            input_entered.store(true, Ordering::Release);
            let (lock, changed) = &*input_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });
    let connection = connect_shared(
        &output,
        &input,
        &dispatcher,
        SharedDelivery::every_selected(4, EveryFullPolicy::Backpressure),
    )
    .unwrap();

    output.publish(0, Arc::<[u8]>::from(vec![0u8; 16]));
    wait_until(Duration::from_secs(1), || entered.load(Ordering::Acquire));
    let pending: Arc<[u8]> = Arc::from(vec![1u8; 16]);
    let pending_weak = Arc::downgrade(&pending);
    output.publish(1, pending);
    assert!(pending_weak.upgrade().is_some());

    connection.disconnect();
    assert!(pending_weak.upgrade().is_none());
    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
}

#[test]
fn camera_buffer_local_detector_and_serialized_remote_have_explicit_copy_boundaries() {
    let scheduler = SharedScheduler::new(3).unwrap();
    let dispatcher = scheduler.dispatcher();
    let peer = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer.peer_id(),
        "camera",
        2,
        2,
        peer.catalog().clone(),
        Vec::<String>::new(),
    )
    .unwrap();
    let products = CameraBufferRoller::attach(&camera, 8).unwrap();
    let local_storage = Arc::new(Mutex::new(None));
    let local = Arc::clone(&local_storage);
    let local_detector = observation_input(
        "local-detector",
        move |event: &ObservationEvent<VideoFrame>| {
            if let ObservationEvent::Observation(observation) = event {
                *local.lock().unwrap() = Some(observation.payload.bytes.as_ptr() as usize);
            }
        },
    );
    let remote_storage = Arc::new(Mutex::new(None));
    let remote = Arc::clone(&remote_storage);
    let remote_detector = observation_input(
        "remote-detector",
        move |event: &ObservationEvent<VideoFrame>| {
            if let ObservationEvent::Observation(observation) = event {
                *remote.lock().unwrap() = Some(observation.payload.bytes.as_ptr() as usize);
            }
        },
    );

    let local_handle = camera
        .current_output()
        .follow_new_shared(
            &local_detector,
            &dispatcher,
            SharedDelivery::coalesce_latest(),
        )
        .unwrap();
    let transport = SerializedInMemoryTransport::default();
    let remote_handle = transport
        .follow_new_shared(
            &camera.current_output(),
            &remote_detector,
            &dispatcher,
            SharedDelivery::coalesce_latest(),
        )
        .unwrap();

    let produced = camera.publish_rgb8(1, 2, 2, rgb8(2, 2, 9)).unwrap();
    let source_address = produced.payload.bytes.as_ptr() as usize;
    wait_until(Duration::from_secs(1), || {
        local_handle.stats().delivered == 1 && remote_handle.stats().delivered == 1
    });
    let retained = products.current().latest_existing().unwrap().unwrap();

    assert_eq!(*local_storage.lock().unwrap(), Some(source_address));
    assert_eq!(retained.payload.bytes.as_ptr() as usize, source_address);
    assert_ne!(*remote_storage.lock().unwrap(), Some(source_address));
    assert!(remote_handle.stats().transport.encoded_bytes > 0);
}

#[test]
fn legacy_threaded_failure_is_also_inspectable() {
    let output = OutputPort::new("source.values");
    let input = InputPort::try_new("fallible", |_: &_| Err("no value".to_owned()));
    let connection = connect(&output, &input, ConnectionOptions::Latest).unwrap();
    output.publish(0, 1u64);
    wait_until(Duration::from_secs(1), || connection.stats().failed);
    assert!(
        connection
            .failure()
            .unwrap()
            .to_string()
            .contains("no value")
    );
}

#[test]
fn inline_reported_error_is_contained_and_inspectable() {
    let output = OutputPort::new("source.values");
    let input = InputPort::try_new("fallible", |_: &_| Err("bad input".to_owned()));
    let connection = connect(&output, &input, ConnectionOptions::InlineEvery).unwrap();

    let report = output.publish(0, 1u64);
    assert_eq!(report.failed, 1);
    assert!(connection.stats().failed);
    assert!(
        connection
            .failure()
            .unwrap()
            .to_string()
            .contains("bad input")
    );
}

#[test]
fn inline_panic_is_contained_and_inspectable() {
    let output = OutputPort::new("source.values");
    let input = InputPort::new("panicking", |_: &_| panic!("broken component"));
    let connection = connect(&output, &input, ConnectionOptions::InlineEvery).unwrap();

    let report = output.publish(0, 1u64);
    assert_eq!(report.failed, 1);
    assert!(connection.stats().failed);
    assert!(
        connection
            .failure()
            .unwrap()
            .to_string()
            .contains("broken component")
    );
}
