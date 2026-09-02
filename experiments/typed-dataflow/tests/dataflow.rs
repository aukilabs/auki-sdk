use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    Buffer, BufferLimits, ChunkBuilder, ChunkBuilderConfig, ConnectionOptions, CursorRead,
    CursorStart, Episode, EpisodeState, EveryFullPolicy, InputPort, OutputPort, PumpOptions,
    SinkFullPolicy, StaticConnection, StreamPump, connect, connect_buffer,
};

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::yield_now();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn static_inline_delivers_in_order_without_shared_ownership() {
    let mut seen = Vec::new();
    {
        let mut connection =
            StaticConnection::new(|value: &auki_typed_dataflow_experiment::Envelope<u64>| {
                seen.push((value.sequence, value.payload));
            });
        for value in 10..15 {
            connection.publish(value, value);
        }
    }
    assert_eq!(seen, vec![(0, 10), (1, 11), (2, 12), (3, 13), (4, 14)]);
}

#[test]
fn inline_every_delivers_once_in_registration_order() {
    let output = OutputPort::new("source.values");
    let seen = Arc::new(Mutex::new(Vec::new()));

    let first_seen = Arc::clone(&seen);
    let first = InputPort::new("first.values", move |value: &_| {
        first_seen.lock().unwrap().push(("first", value.sequence));
    });
    let second_seen = Arc::clone(&seen);
    let second = InputPort::new("second.values", move |value: &_| {
        second_seen.lock().unwrap().push(("second", value.sequence));
    });
    let _first_connection = connect(&output, &first, ConnectionOptions::InlineEvery).unwrap();
    let _second_connection = connect(&output, &second, ConnectionOptions::InlineEvery).unwrap();

    for value in 0..3u64 {
        output.publish(value, value);
    }

    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ("first", 0),
            ("second", 0),
            ("first", 1),
            ("second", 1),
            ("first", 2),
            ("second", 2),
        ]
    );
}

#[test]
fn queued_every_with_backpressure_delivers_every_accepted_value() {
    let output = OutputPort::new("source.values");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let input_seen = Arc::clone(&seen);
    let input = InputPort::new("sink.values", move |value: &_| {
        input_seen.lock().unwrap().push(value.sequence);
    });
    let connection = connect(
        &output,
        &input,
        ConnectionOptions::QueuedEvery {
            capacity: 2,
            when_full: EveryFullPolicy::Backpressure,
        },
    )
    .unwrap();

    for value in 0..100u64 {
        output.publish(value, value);
    }
    wait_until(Duration::from_secs(1), || {
        connection.stats().delivered == 100
    });

    assert_eq!(*seen.lock().unwrap(), (0..100).collect::<Vec<_>>());
    assert_eq!(connection.stats().accepted, 100);
    assert_eq!(connection.stats().overruns, 0);
}

#[test]
fn queued_every_disconnect_reports_full_queue_instead_of_dropping() {
    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let input_gate = Arc::clone(&gate);
    let input_entered = Arc::clone(&entered);
    let input = InputPort::new("blocked.values", move |_: &_| {
        input_entered.store(true, Ordering::Release);
        let (lock, changed) = &*input_gate;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = changed.wait(open).unwrap();
        }
    });
    let connection = connect(
        &output,
        &input,
        ConnectionOptions::QueuedEvery {
            capacity: 1,
            when_full: EveryFullPolicy::Disconnect,
        },
    )
    .unwrap();

    assert_eq!(output.publish(0, 0).accepted, 1);
    wait_until(Duration::from_secs(1), || entered.load(Ordering::Acquire));
    assert_eq!(output.publish(1, 1).accepted, 1);
    let report = output.publish(2, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(connection.stats().overruns, 1);
    assert!(connection.stats().closed);
    assert!(connection.stats().failed);
    assert!(connection.failure().is_some());

    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
}

#[test]
fn latest_replaces_pending_values_and_delivers_newest() {
    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let input_gate = Arc::clone(&gate);
    let input_entered = Arc::clone(&entered);
    let input_seen = Arc::clone(&seen);
    let input = InputPort::new("preview.values", move |value: &_| {
        input_seen.lock().unwrap().push(value.sequence);
        if value.sequence == 0 {
            input_entered.store(true, Ordering::Release);
            let (lock, changed) = &*input_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });
    let connection = connect(&output, &input, ConnectionOptions::Latest).unwrap();

    output.publish(0, 0);
    wait_until(Duration::from_secs(1), || entered.load(Ordering::Acquire));
    for value in 1..10u64 {
        output.publish(value, value);
    }
    assert_eq!(connection.stats().replaced, 8);
    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    wait_until(Duration::from_secs(1), || connection.stats().delivered == 2);

    assert_eq!(*seen.lock().unwrap(), vec![0, 9]);
}

#[test]
fn owning_fanout_shares_one_envelope_and_payload() {
    #[derive(Debug)]
    struct Frame(Arc<[u8]>);

    let output = OutputPort::new("camera.frames");
    let envelope_addresses = Arc::new(Mutex::new(Vec::new()));
    let payload_addresses = Arc::new(Mutex::new(Vec::new()));
    let mut connections = Vec::new();
    for subscriber in 0..8 {
        let envelope_addresses = Arc::clone(&envelope_addresses);
        let payload_addresses = Arc::clone(&payload_addresses);
        let input =
            InputPort::<Frame>::new(format!("consumer-{subscriber}.frames"), move |value| {
                envelope_addresses
                    .lock()
                    .unwrap()
                    .push(value as *const _ as usize);
                payload_addresses
                    .lock()
                    .unwrap()
                    .push(value.payload.0.as_ptr() as usize);
            });
        connections.push(connect(&output, &input, ConnectionOptions::Latest).unwrap());
    }

    output.publish(0, Frame(Arc::from(vec![7u8; 6 * 1024 * 1024])));
    wait_until(Duration::from_secs(2), || {
        connections
            .iter()
            .all(|connection| connection.stats().delivered == 1)
    });

    let envelopes = envelope_addresses.lock().unwrap();
    assert!(envelopes.iter().all(|address| *address == envelopes[0]));
    let payloads = payload_addresses.lock().unwrap();
    assert!(payloads.iter().all(|address| *address == payloads[0]));
}

#[test]
fn blocked_latest_consumer_does_not_delay_unrelated_queued_consumer() {
    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let slow_gate = Arc::clone(&gate);
    let slow_entered = Arc::clone(&entered);
    let slow = InputPort::new("slow.values", move |value: &_| {
        if value.sequence == 0 {
            slow_entered.store(true, Ordering::Release);
            let (lock, changed) = &*slow_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });
    let fast_delivered = Arc::new(AtomicUsize::new(0));
    let fast_counter = Arc::clone(&fast_delivered);
    let fast = InputPort::new("fast.values", move |_: &_| {
        fast_counter.fetch_add(1, Ordering::Relaxed);
    });
    let _slow_connection = connect(&output, &slow, ConnectionOptions::Latest).unwrap();
    let fast_connection = connect(
        &output,
        &fast,
        ConnectionOptions::QueuedEvery {
            capacity: 128,
            when_full: EveryFullPolicy::Backpressure,
        },
    )
    .unwrap();

    output.publish(0, 0u64);
    wait_until(Duration::from_secs(1), || entered.load(Ordering::Acquire));
    for value in 1..100u64 {
        output.publish(value, value);
    }
    wait_until(Duration::from_secs(1), || {
        fast_connection.stats().delivered == 100
    });
    assert_eq!(fast_delivered.load(Ordering::Relaxed), 100);

    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
}

#[test]
fn buffer_evicts_and_reports_cursor_gap_without_cursor_owned_history() {
    let buffer = Buffer::new("camera.buffer", 3).unwrap();
    let output = OutputPort::new("camera.frames");
    let _connection = connect_buffer(&output, &buffer);
    let mut stalled = buffer.subscribe(CursorStart::FromSequence(0));

    for value in 0..10u64 {
        output.publish(value, value);
    }

    assert_eq!(buffer.range().entries, 3);
    assert_eq!(buffer.range().first_sequence, Some(7));
    assert!(buffer.snapshot(0, 6).is_empty());
    match stalled.next_timeout(Duration::ZERO) {
        CursorRead::Gap(gap) => {
            assert_eq!(gap.requested_sequence, 0);
            assert_eq!(gap.available_from, 7);
        }
        other => panic!("expected gap, got {other:?}"),
    }
    match stalled.next_timeout(Duration::ZERO) {
        CursorRead::Item(value) => assert_eq!(value.sequence, 7),
        other => panic!("expected retained item, got {other:?}"),
    }
    assert_eq!(buffer.range().entries, 3);
}

#[test]
fn buffer_honors_combined_hard_limits() {
    #[derive(Debug)]
    struct Bytes(Vec<u8>);

    let buffer = Buffer::with_limits(
        "bytes.buffer",
        BufferLimits {
            max_entries: Some(4),
            max_bytes: Some(10),
            target_duration: None,
        },
        |payload: &Bytes| payload.0.len(),
    )
    .unwrap();

    for sequence in 0..8 {
        buffer
            .append_shared(Arc::new(auki_typed_dataflow_experiment::Envelope::new(
                sequence,
                sequence,
                Bytes(vec![0; 3]),
            )))
            .unwrap();
        let range = buffer.range();
        assert!(range.entries <= 4);
        assert!(range.retained_payload_bytes <= 10);
    }
    assert_eq!(buffer.range().entries, 3);
    assert_eq!(buffer.range().retained_payload_bytes, 9);
}

#[test]
fn buffer_target_duration_evicts_old_entries_within_hard_bound() {
    let buffer = Buffer::with_limits(
        "timed.buffer",
        BufferLimits {
            max_entries: Some(10),
            max_bytes: None,
            target_duration: Some(Duration::from_nanos(10)),
        },
        |_| 1,
    )
    .unwrap();
    for (sequence, timestamp_ns) in [(0, 0), (1, 5), (2, 11), (3, 20)] {
        buffer
            .append_shared(Arc::new(auki_typed_dataflow_experiment::Envelope::new(
                sequence,
                timestamp_ns,
                sequence,
            )))
            .unwrap();
    }

    assert_eq!(buffer.range().first_sequence, Some(2));
    assert_eq!(buffer.range().last_sequence, Some(3));
    assert_eq!(buffer.range().entries, 2);
}

#[test]
fn episode_promotion_shares_payload_storage_and_concludes_once() {
    let buffer = Buffer::new("camera.buffer", 5).unwrap();
    let output = OutputPort::new("camera.frames");
    let _connection = connect_buffer(&output, &buffer);
    for value in 0..5u64 {
        output.publish(value, value);
    }

    let episode = Episode::promote("collision", &buffer, 2, 4).unwrap();
    let retained = buffer.snapshot(2, 4);
    let promoted = episode.snapshot();
    assert_eq!(promoted.len(), 3);
    assert!(
        retained
            .iter()
            .zip(&promoted)
            .all(|(left, right)| Arc::ptr_eq(left, right))
    );
    episode.conclude(99).unwrap();
    assert_eq!(
        episode.state(),
        EpisodeState::Concluded {
            last_sequence: Some(4),
            end_timestamp_ns: 99,
        }
    );
    assert!(episode.conclude(100).is_err());
}

#[test]
fn pump_reuses_source_envelope_and_cancellation_is_per_recipient() {
    let source = Buffer::new("camera.buffer", 8).unwrap();
    let recipient_a = Buffer::new("peer-a.remote-camera", 8).unwrap();
    let recipient_b = Buffer::new("peer-b.remote-camera", 8).unwrap();
    let pump_a = StreamPump::start(
        &source,
        CursorStart::Latest,
        &recipient_a,
        PumpOptions::default(),
    )
    .unwrap();
    let pump_b = StreamPump::start(
        &source,
        CursorStart::Latest,
        &recipient_b,
        PumpOptions::default(),
    )
    .unwrap();
    let output = OutputPort::new("camera.frames");
    let _connection = connect_buffer(&output, &source);

    output.publish(0, 10u64);
    wait_until(Duration::from_secs(1), || {
        recipient_a.range().entries == 1 && recipient_b.range().entries == 1
    });
    let source_entry = source.snapshot(0, 0).pop().unwrap();
    let remote_entry = recipient_a.snapshot(0, 0).pop().unwrap();
    assert!(Arc::ptr_eq(&source_entry, &remote_entry));

    pump_a.cancel();
    output.publish(1, 11);
    wait_until(Duration::from_secs(1), || recipient_b.range().entries == 2);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(recipient_a.range().entries, 1);
    assert_eq!(pump_b.stats().delivered_sequence, Some(1));
}

#[test]
fn lossy_pump_preserves_source_sequences_so_remote_gap_is_visible() {
    let source = Buffer::new("camera.buffer", 64).unwrap();
    let remote = Buffer::new("remote.camera.buffer", 64).unwrap();
    let pump = StreamPump::start(
        &source,
        CursorStart::Latest,
        &remote,
        PumpOptions {
            sink_capacity: 1,
            when_full: SinkFullPolicy::DropNewest,
            receiver_delay: Duration::from_millis(5),
            cursor_poll_interval: Duration::from_millis(1),
        },
    )
    .unwrap();
    let output = OutputPort::new("camera.frames");
    let _connection = connect_buffer(&output, &source);

    for value in 0..30u64 {
        output.publish(value, value);
    }
    wait_until(Duration::from_secs(1), || pump.stats().sink_drops > 0);
    thread::sleep(Duration::from_millis(30));
    output.publish(30, 30);
    wait_until(Duration::from_secs(1), || {
        remote.range().last_sequence == Some(30)
    });

    let mut cursor = remote.subscribe(CursorStart::FromSequence(0));
    let mut saw_gap = false;
    for _ in 0..remote.range().entries + 2 {
        if matches!(cursor.next_timeout(Duration::ZERO), CursorRead::Gap(_)) {
            saw_gap = true;
            break;
        }
    }
    assert!(saw_gap);
}

#[test]
fn live_pump_does_not_wait_for_chunk_sealing() {
    let source = Buffer::new("camera.buffer", 8).unwrap();
    let remote = Buffer::new("remote.camera.buffer", 8).unwrap();
    let builder = ChunkBuilder::start(
        &source,
        CursorStart::Latest,
        ChunkBuilderConfig {
            max_entries: 100,
            max_bytes: usize::MAX,
            max_latency: Duration::from_secs(10),
            poll_interval: Duration::from_millis(1),
        },
        |_| 1,
    )
    .unwrap();
    let _pump = StreamPump::start(
        &source,
        CursorStart::Latest,
        &remote,
        PumpOptions::default(),
    )
    .unwrap();
    let output = OutputPort::new("camera.frames");
    let _connection = connect_buffer(&output, &source);

    let started = Instant::now();
    output.publish(0, 42u64);
    wait_until(Duration::from_secs(1), || remote.range().entries == 1);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(builder.chunks().is_empty());
    let episode = Episode::promote("before-chunk-seal", &source, 0, 0).unwrap();
    assert!(Arc::ptr_eq(
        &episode.snapshot()[0],
        &source.snapshot(0, 0)[0]
    ));
    builder.stop();
    assert_eq!(builder.chunks().len(), 1);
}

#[test]
fn dropping_connection_stops_future_delivery() {
    let output = OutputPort::new("source.values");
    let delivered = Arc::new(AtomicUsize::new(0));
    let input_delivered = Arc::clone(&delivered);
    let input = InputPort::new("sink.values", move |_: &_| {
        input_delivered.fetch_add(1, Ordering::Relaxed);
    });
    let connection = connect(&output, &input, ConnectionOptions::InlineEvery).unwrap();
    output.publish(0, 0u64);
    drop(connection);
    output.publish(1, 1u64);
    assert_eq!(delivered.load(Ordering::Relaxed), 1);
}

#[test]
fn disconnect_clears_queued_payload_ownership() {
    struct DropObserved(Arc<AtomicUsize>);

    impl Drop for DropObserved {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let output = OutputPort::new("source.values");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let input_gate = Arc::clone(&gate);
    let input_entered = Arc::clone(&entered);
    let input = InputPort::new("blocked.values", move |value: &_| {
        if value.sequence == 0 {
            input_entered.store(true, Ordering::Release);
            let (lock, changed) = &*input_gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }
    });
    let connection = connect(
        &output,
        &input,
        ConnectionOptions::QueuedEvery {
            capacity: 1,
            when_full: EveryFullPolicy::Backpressure,
        },
    )
    .unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    output.publish(0, DropObserved(Arc::clone(&drops)));
    wait_until(Duration::from_secs(1), || entered.load(Ordering::Acquire));
    output.publish(1, DropObserved(Arc::clone(&drops)));

    let disconnect = thread::spawn(move || drop(connection));
    wait_until(Duration::from_secs(1), || {
        drops.load(Ordering::Relaxed) >= 1
    });
    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    disconnect.join().unwrap();
    wait_until(Duration::from_secs(1), || {
        drops.load(Ordering::Relaxed) == 2
    });
}
