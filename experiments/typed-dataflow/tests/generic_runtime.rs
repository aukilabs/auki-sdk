use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use auki_typed_dataflow_experiment::{
    AudioLayout, AudioPayloadContract, AudioSampleFormat, BufferError, BufferLimits,
    ComponentBuildError, ComponentSpec, ConfiguredObservableSpec, ContractType, EveryFullPolicy,
    Exposure, GaugePayloadContract, InMemoryTransport, InvocationContext, InvocationError,
    InvocationOptions, InvocationOrdering, InvocationStatus, ObservableContract, ObservationAccess,
    ObservationDelivery, ObservationEndReason, ObservationEvent, OperableContract, PayloadContract,
    PeerRuntime, ProductCaptureError, ProductState, PublishError, SerializedInMemoryTransport,
    SharedScheduler, observation_input,
};

fn gauge_payload(observes: &str) -> PayloadContract {
    PayloadContract::Gauge(GaugePayloadContract {
        datatype: "float64".to_owned(),
        schema: "demo.gauge/v1".to_owned(),
        observes: observes.to_owned(),
        unit: "percent".to_owned(),
    })
}

fn gauge_contract(name: &str) -> ObservableContract {
    ObservableContract {
        name: name.to_owned(),
        datatype: "float64".to_owned(),
        schema: "demo.gauge/v1".to_owned(),
        access: vec![ObservationAccess::FollowNew],
        exposure: Exposure::Cluster,
    }
}

fn command_contract(name: &str) -> OperableContract {
    OperableContract {
        name: name.to_owned(),
        instruction: "uint64".to_owned(),
        result: "uint64".to_owned(),
        exposure: Exposure::Cluster,
    }
}

fn context(peer: &str, component: &str, invocation: &str) -> InvocationContext {
    InvocationContext {
        invocation_id: invocation.to_owned(),
        caller_peer_id: peer.to_owned(),
        caller_component_id: component.to_owned(),
    }
}

struct CleanupInstruction {
    payload: Arc<()>,
    block: bool,
}

impl ContractType for CleanupInstruction {
    const DATATYPE: &'static str = "cleanup_instruction";
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + timeout;
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "condition timed out");
        thread::yield_now();
    }
}

#[test]
fn components_can_expose_only_observables_only_operables_or_both() {
    let peer = PeerRuntime::new("peer-a");

    let sensor = peer
        .component(ComponentSpec::new("sensor").observable(gauge_contract("level")))
        .unwrap();
    let _level = sensor
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("battery_state_of_charge"),
        ))
        .unwrap();
    sensor.expose().unwrap();

    let actuator = peer
        .component(ComponentSpec::new("actuator").operable(command_contract("set")))
        .unwrap();
    let _set = actuator
        .operable("set", |_| true, |_, value: u64| Ok(value))
        .unwrap();
    actuator.expose().unwrap();

    let both = peer
        .component(
            ComponentSpec::new("both")
                .observable(gauge_contract("level"))
                .operable(command_contract("set")),
        )
        .unwrap();
    let _level = both
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    let _set = both
        .operable("set", |_| true, |_, value: u64| Ok(value))
        .unwrap();
    both.expose().unwrap();

    assert_eq!(peer.catalog().components().len(), 3);
    assert!(
        peer.catalog()
            .component("sensor")
            .unwrap()
            .manifest
            .operables
            .is_empty()
    );
    assert!(
        peer.catalog()
            .component("actuator")
            .unwrap()
            .manifest
            .observables
            .is_empty()
    );
}

#[test]
fn modality_contract_serialization_has_one_kind_and_only_relevant_fields() {
    let audio = PayloadContract::Audio(AudioPayloadContract {
        datatype: "audio_block_f32".to_owned(),
        schema: "demo.audio/v1".to_owned(),
        sample_format: AudioSampleFormat::F32,
        layout: AudioLayout::Interleaved,
        sample_rate_hz: 48_000,
        channels: 2,
        frames_per_block: 480,
        observes: "acoustic_pressure_waveform".to_owned(),
        unit: Some("full_scale_amplitude".to_owned()),
    });
    let json = serde_json::to_value(audio).unwrap();
    assert_eq!(json["kind"], "audio");
    assert_eq!(json["sample_format"], "f32");
    assert_eq!(json["layout"], "interleaved");
    assert_eq!(json["sample_rate_hz"], 48_000);
    assert!(json.get("width").is_none());
    assert!(json.get("height").is_none());
}

#[test]
fn exposure_requires_live_handles_and_exact_contracts() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(
            ComponentSpec::new("incomplete")
                .observable(gauge_contract("level"))
                .operable(command_contract("set")),
        )
        .unwrap();
    assert_eq!(
        component.expose().unwrap_err(),
        ComponentBuildError::MissingObservable("level".to_owned())
    );
    assert!(peer.catalog().component("incomplete").is_none());

    let mismatch = PayloadContract::Gauge(GaugePayloadContract {
        datatype: "vec3".to_owned(),
        schema: "demo.vec3/v1".to_owned(),
        observes: "level".to_owned(),
        unit: "percent".to_owned(),
    });
    assert!(matches!(
        component.configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            mismatch,
        )),
        Err(ComponentBuildError::ContractMismatch { .. })
    ));
    assert_eq!(
        component
            .configured_observable::<String>(ConfiguredObservableSpec::new(
                "level",
                "level-1",
                "peer-a.session-clock",
                gauge_payload("load"),
            ))
            .unwrap_err(),
        ComponentBuildError::RustDatatypeMismatch {
            interface: "level".to_owned(),
            rust_datatype: "string".to_owned(),
            contract_datatype: "float64".to_owned(),
        }
    );

    let _level = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    assert_eq!(
        component.expose().unwrap_err(),
        ComponentBuildError::MissingOperable("set".to_owned())
    );
    let _set = component
        .operable("set", |_| true, |_, value: u64| Ok(value))
        .unwrap();
    component.expose().unwrap();
    assert!(peer.catalog().component("incomplete").is_some());
}

#[test]
fn configured_output_replacement_updates_catalog_ends_old_output_and_requires_new_product() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(ComponentSpec::new("sensor").observable(gauge_contract("level")))
        .unwrap();
    let first = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    component.expose().unwrap();
    let first_product = peer
        .capture_buffer(
            "level-history-1",
            &first,
            BufferLimits {
                max_entries: Some(10),
                max_bytes: None,
                target_duration: None,
            },
            |_| size_of::<f64>(),
        )
        .unwrap();
    first.publish(10, Arc::new(1.0)).unwrap();

    let transition = component
        .replace_configured_observable::<f64>(
            &first,
            ConfiguredObservableSpec::new(
                "level",
                "level-2",
                "peer-a.session-clock",
                gauge_payload("load after reconfiguration"),
            ),
            20,
        )
        .unwrap();
    let second = transition.replacement;

    assert_eq!(transition.previous_end.output, *first.reference());
    assert_eq!(transition.previous_end.last_sequence, Some(0));
    assert_eq!(transition.previous_end.timestamp_ns, 20);
    assert_eq!(
        transition.previous_end.reason,
        ObservationEndReason::Reconfigured {
            replacement: Some(second.reference().clone()),
        }
    );
    assert!(matches!(
        first.publish(21, Arc::new(2.0)),
        Err(PublishError::Ended)
    ));
    assert_eq!(first_product.end_notice(), Some(transition.previous_end));
    assert_eq!(first_product.product().buffer().range().entries, 1);

    let catalog_component = peer.catalog().component("sensor").unwrap();
    assert_eq!(
        catalog_component.current_outputs["level"]
            .manifest
            .reference(),
        *second.reference()
    );
    assert_eq!(
        catalog_component.current_outputs["level"].manifest.payload,
        gauge_payload("load after reconfiguration")
    );

    let second_product = peer
        .capture_buffer(
            "level-history-2",
            &second,
            BufferLimits {
                max_entries: Some(10),
                max_bytes: None,
                target_duration: None,
            },
            |_| size_of::<f64>(),
        )
        .unwrap();
    second.publish(30, Arc::new(3.0)).unwrap();
    assert_eq!(second_product.product().buffer().range().entries, 1);
    assert_eq!(peer.catalog().products().len(), 2);
    assert_eq!(
        peer.catalog()
            .product("level-history-2")
            .unwrap()
            .manifest
            .producer,
        *second.reference()
    );

    assert_eq!(
        component
            .replace_configured_observable::<f64>(
                &first,
                ConfiguredObservableSpec::new(
                    "level",
                    "level-3",
                    "peer-a.session-clock",
                    gauge_payload("load"),
                ),
                40,
            )
            .unwrap_err(),
        ComponentBuildError::OutputAlreadyEnded("level-1".to_owned())
    );
}

#[test]
fn deleting_a_buffer_capture_unregisters_its_product_and_stops_retention() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(ComponentSpec::new("sensor").observable(gauge_contract("level")))
        .unwrap();
    let output = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    component.expose().unwrap();
    let capture = peer
        .capture_buffer(
            "level-history",
            &output,
            BufferLimits {
                max_entries: Some(10),
                max_bytes: None,
                target_duration: None,
            },
            |_| size_of::<f64>(),
        )
        .unwrap();
    output.publish(10, Arc::new(1.0)).unwrap();
    let retained_lease = capture.product();

    let deleted = capture.delete().unwrap();

    assert_eq!(deleted.product_id, "level-history");
    assert!(peer.catalog().product("level-history").is_none());
    output.publish(20, Arc::new(2.0)).unwrap();
    assert_eq!(retained_lease.buffer().range().entries, 1);
}

#[test]
fn live_buffer_product_limits_can_be_reconfigured() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(ComponentSpec::new("sensor").observable(gauge_contract("level")))
        .unwrap();
    let output = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    component.expose().unwrap();
    let capture = peer
        .capture_buffer(
            "level-history",
            &output,
            BufferLimits {
                max_entries: Some(4),
                max_bytes: None,
                target_duration: None,
            },
            |_| size_of::<f64>(),
        )
        .unwrap();
    output.publish(10, Arc::new(1.0)).unwrap();
    output.publish(20, Arc::new(2.0)).unwrap();
    output.publish(30, Arc::new(3.0)).unwrap();

    let replacement_limits = BufferLimits {
        max_entries: Some(4),
        max_bytes: None,
        target_duration: Some(Duration::from_nanos(10)),
    };
    let range = capture.set_limits(replacement_limits).unwrap();

    assert_eq!(capture.product().buffer().limits(), replacement_limits);
    assert_eq!(range.first_sequence, Some(1));
    assert_eq!(range.entries, 2);
    assert_eq!(
        peer.catalog().product("level-history").unwrap().state,
        ProductState::Buffer {
            entries: 2,
            at_entry_capacity: false,
        }
    );
    output.publish(40, Arc::new(4.0)).unwrap();
    assert_eq!(capture.product().buffer().range().first_sequence, Some(2));

    capture.cancel();
    assert!(matches!(
        capture.set_limits(replacement_limits),
        Err(ProductCaptureError::Buffer(BufferError::Closed))
    ));
}

#[test]
fn fresh_observables_cannot_claim_retained_access_or_outlive_their_handle_at_exposure() {
    let peer = PeerRuntime::new("peer-a");
    let false_access = peer
        .component(
            ComponentSpec::new("false-access").observable(ObservableContract {
                name: "level".to_owned(),
                datatype: "float64".to_owned(),
                schema: "demo.gauge/v1".to_owned(),
                access: vec![
                    ObservationAccess::LatestExisting,
                    ObservationAccess::FollowNew,
                ],
                exposure: Exposure::Cluster,
            }),
        )
        .unwrap();
    assert_eq!(
        false_access
            .configured_observable::<f64>(ConfiguredObservableSpec::new(
                "level",
                "level-1",
                "peer-a.session-clock",
                gauge_payload("load"),
            ))
            .unwrap_err(),
        ComponentBuildError::UnsupportedLiveAccess {
            interface: "level".to_owned(),
            access: ObservationAccess::LatestExisting,
        }
    );

    let dropped = peer
        .component(ComponentSpec::new("dropped").observable(gauge_contract("level")))
        .unwrap();
    let output = dropped
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    drop(output);
    assert_eq!(
        dropped.expose().unwrap_err(),
        ComponentBuildError::DroppedObservable("level".to_owned())
    );
    assert!(peer.catalog().component("dropped").is_none());
}

#[test]
fn local_and_serialized_paths_preserve_semantics_and_caller_identity() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(
            ComponentSpec::new("gauge")
                .observable(gauge_contract("level"))
                .operable(command_contract("add-one")),
        )
        .unwrap();
    let level = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    let callers = Arc::new(Mutex::new(Vec::new()));
    let handler_callers = Arc::clone(&callers);
    let add_one = component
        .operable(
            "add-one",
            |context| context.caller_peer_id == "peer-a" || context.caller_peer_id == "peer-b",
            move |context, value: u64| {
                handler_callers.lock().unwrap().push((
                    context.caller_peer_id.clone(),
                    context.caller_component_id.clone(),
                ));
                Ok(value + 1)
            },
        )
        .unwrap();
    component.expose().unwrap();

    let local_values = Arc::new(Mutex::new(Vec::new()));
    let local_sink = Arc::clone(&local_values);
    let local_input = observation_input("local", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            local_sink.lock().unwrap().push(*observation.payload);
        }
    });
    let remote_values = Arc::new(Mutex::new(Vec::new()));
    let remote_sink = Arc::clone(&remote_values);
    let remote_input = observation_input("remote", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            remote_sink.lock().unwrap().push(*observation.payload);
        }
    });
    let local = InMemoryTransport
        .follow_new(
            &level.observable(),
            &local_input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();
    let serialized = SerializedInMemoryTransport::default();
    let remote = serialized
        .follow_new(
            &level.observable(),
            &remote_input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();
    level.publish(10, Arc::new(42.0)).unwrap();
    assert_eq!(*local_values.lock().unwrap(), vec![42.0]);
    assert_eq!(*remote_values.lock().unwrap(), vec![42.0]);
    assert_eq!(local.stats().transport.encoded_bytes, 0);
    assert!(remote.stats().transport.encoded_bytes > 0);

    let local_result = InMemoryTransport
        .invoke(
            &add_one,
            context("peer-a", "local-controller", "local-1"),
            4,
        )
        .unwrap();
    let remote_result = serialized
        .invoke(
            &add_one,
            context("peer-b", "remote-controller", "remote-1"),
            4,
        )
        .unwrap();
    assert_eq!(local_result.result, remote_result.result);
    assert_eq!(
        *callers.lock().unwrap(),
        vec![
            ("peer-a".to_owned(), "local-controller".to_owned()),
            ("peer-b".to_owned(), "remote-controller".to_owned()),
        ]
    );
}

#[test]
fn local_and_serialized_live_paths_match_queued_and_coalescing_delivery() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(ComponentSpec::new("gauge").observable(gauge_contract("level")))
        .unwrap();
    let level = component
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.session-clock",
            gauge_payload("load"),
        ))
        .unwrap();
    component.expose().unwrap();
    let transport = SerializedInMemoryTransport::default();

    let local_every = Arc::new(Mutex::new(Vec::new()));
    let local_sink = Arc::clone(&local_every);
    let local_input = observation_input("local-every", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            local_sink.lock().unwrap().push(*observation.payload as u64);
        }
    });
    let remote_every = Arc::new(Mutex::new(Vec::new()));
    let remote_sink = Arc::clone(&remote_every);
    let remote_input = observation_input("remote-every", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            remote_sink
                .lock()
                .unwrap()
                .push(*observation.payload as u64);
        }
    });
    let local_handle = level
        .observable()
        .follow_new(
            &local_input,
            ObservationDelivery::queued_every_selected(16, EveryFullPolicy::Backpressure),
        )
        .unwrap();
    let remote_handle = transport
        .follow_new(
            &level.observable(),
            &remote_input,
            ObservationDelivery::queued_every_selected(16, EveryFullPolicy::Backpressure),
        )
        .unwrap();
    for value in 0..10 {
        level.publish(value, Arc::new(value as f64)).unwrap();
    }
    wait_until(Duration::from_secs(1), || {
        local_handle.stats().delivered == 10 && remote_handle.stats().delivered == 10
    });
    assert_eq!(*local_every.lock().unwrap(), (0..10).collect::<Vec<_>>());
    assert_eq!(*remote_every.lock().unwrap(), (0..10).collect::<Vec<_>>());
    drop(local_handle);
    drop(remote_handle);

    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let local_latest = Arc::new(Mutex::new(Vec::new()));
    let local_sink = Arc::clone(&local_latest);
    let local_gate = Arc::clone(&gate);
    let local_entered = Arc::clone(&entered);
    let local_input = observation_input("local-latest", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            if observation.sequence == 10 {
                local_entered.fetch_add(1, Ordering::Release);
                let (open, changed) = &*local_gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
            }
            local_sink.lock().unwrap().push(observation.sequence);
        }
    });
    let remote_latest = Arc::new(Mutex::new(Vec::new()));
    let remote_sink = Arc::clone(&remote_latest);
    let remote_gate = Arc::clone(&gate);
    let remote_entered = Arc::clone(&entered);
    let remote_input = observation_input("remote-latest", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            if observation.sequence == 10 {
                remote_entered.fetch_add(1, Ordering::Release);
                let (open, changed) = &*remote_gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
            }
            remote_sink.lock().unwrap().push(observation.sequence);
        }
    });
    let local_handle = level
        .observable()
        .follow_new(&local_input, ObservationDelivery::coalesce_latest())
        .unwrap();
    let remote_handle = transport
        .follow_new(
            &level.observable(),
            &remote_input,
            ObservationDelivery::coalesce_latest(),
        )
        .unwrap();
    level.publish(10, Arc::new(10.0)).unwrap();
    wait_until(Duration::from_secs(1), || {
        entered.load(Ordering::Acquire) == 2
    });
    for value in 11..20 {
        level.publish(value, Arc::new(value as f64)).unwrap();
    }
    assert!(local_handle.stats().coalesced > 0);
    assert!(remote_handle.stats().coalesced > 0);
    let (open, changed) = &*gate;
    *open.lock().unwrap() = true;
    changed.notify_all();
    wait_until(Duration::from_secs(1), || {
        local_handle.stats().delivered == 2 && remote_handle.stats().delivered == 2
    });
    assert_eq!(*local_latest.lock().unwrap(), vec![10, 19]);
    assert_eq!(*remote_latest.lock().unwrap(), vec![10, 19]);
}

#[test]
fn invocation_handles_report_ordering_deadlines_cancellation_and_panics() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(
            ComponentSpec::new("controller")
                .operable(command_contract("serial"))
                .operable(command_contract("slow"))
                .operable(command_contract("panic"))
                .operable(command_contract("quick"))
                .operable(command_contract("bounded")),
        )
        .unwrap();

    let order = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let serial_order = Arc::clone(&order);
    let serial_active = Arc::clone(&active);
    let serial_max = Arc::clone(&max_active);
    let serial = component
        .operable_ordered(
            "serial",
            InvocationOrdering::SerialInAcceptanceOrder,
            |_| true,
            move |_, value: u64| {
                let now = serial_active.fetch_add(1, Ordering::AcqRel) + 1;
                serial_max.fetch_max(now, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(2));
                serial_order.lock().unwrap().push(value);
                serial_active.fetch_sub(1, Ordering::AcqRel);
                Ok(value)
            },
        )
        .unwrap();
    let slow = component
        .operable(
            "slow",
            |_| true,
            |_, value: u64| {
                thread::sleep(Duration::from_millis(40));
                Ok(value)
            },
        )
        .unwrap();
    let panicking = component
        .operable(
            "panic",
            |_| true,
            |_, _value: u64| -> Result<u64, InvocationError> { panic!("component panic") },
        )
        .unwrap();
    let quick = component
        .operable("quick", |_| true, |_, value: u64| Ok(value + 1))
        .unwrap();
    let bounded = component
        .operable(
            "bounded",
            |_| true,
            |_, value: u64| {
                thread::sleep(Duration::from_millis(40));
                Ok(value)
            },
        )
        .unwrap()
        .with_max_pending(2);
    component.expose().unwrap();

    let scheduler = SharedScheduler::new(4).unwrap();
    let dispatcher = scheduler.dispatcher();
    let handles: Vec<_> = (0..6)
        .map(|value| {
            serial
                .invoke_async(
                    context("peer-a", "test", &format!("serial-{value}")),
                    value,
                    &dispatcher,
                    InvocationOptions::default(),
                )
                .unwrap()
        })
        .collect();
    for (expected, handle) in handles.iter().enumerate() {
        assert_eq!(
            handle
                .wait_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()
                .result,
            expected as u64
        );
    }
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(max_active.load(Ordering::Relaxed), 1);

    let deadline = slow
        .invoke_async(
            context("peer-a", "test", "deadline"),
            1,
            &dispatcher,
            InvocationOptions {
                deadline_after: Some(Duration::from_millis(2)),
            },
        )
        .unwrap();
    assert_eq!(
        deadline.wait_timeout(Duration::from_millis(20)).unwrap(),
        Err(InvocationError::DeadlineExceeded)
    );
    assert_eq!(deadline.status(), InvocationStatus::DeadlineExceeded);

    let cancelled = slow
        .invoke_async(
            context("peer-a", "test", "cancel"),
            2,
            &dispatcher,
            InvocationOptions::default(),
        )
        .unwrap();
    cancelled.cancel();
    assert_eq!(
        cancelled.wait_timeout(Duration::ZERO).unwrap(),
        Err(InvocationError::Cancelled)
    );

    let failed = panicking
        .invoke_async(
            context("peer-a", "test", "panic"),
            3,
            &dispatcher,
            InvocationOptions::default(),
        )
        .unwrap();
    assert!(matches!(
        failed.wait_timeout(Duration::from_secs(1)).unwrap(),
        Err(InvocationError::TargetPanicked(reason)) if reason == "component panic"
    ));
    assert_eq!(
        quick
            .invoke_async(
                context("peer-a", "test", "quick"),
                4,
                &dispatcher,
                InvocationOptions::default(),
            )
            .unwrap()
            .wait_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap()
            .result,
        5
    );

    let first = bounded
        .invoke_async(
            context("peer-a", "test", "bounded-1"),
            1,
            &dispatcher,
            InvocationOptions::default(),
        )
        .unwrap();
    let second = bounded
        .invoke_async(
            context("peer-a", "test", "bounded-2"),
            2,
            &dispatcher,
            InvocationOptions::default(),
        )
        .unwrap();
    assert!(matches!(
        bounded.invoke_async(
            context("peer-a", "test", "bounded-3"),
            3,
            &dispatcher,
            InvocationOptions::default(),
        ),
        Err(InvocationError::Overloaded { limit: 2 })
    ));
    first.cancel();
    second.cancel();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while bounded.pending_invocations() != 0 {
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
}

#[test]
fn cancelled_queued_invocation_releases_its_instruction_when_drained() {
    let peer = PeerRuntime::new("peer-a");
    let component = peer
        .component(ComponentSpec::new("cleanup").operable(OperableContract {
            name: "run".to_owned(),
            instruction: "cleanup_instruction".to_owned(),
            result: "uint64".to_owned(),
            exposure: Exposure::Cluster,
        }))
        .unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let handler_gate = Arc::clone(&gate);
    let handler_entered = Arc::clone(&entered);
    let operation = component
        .operable(
            "run",
            |_| true,
            move |_, instruction: CleanupInstruction| {
                handler_entered.fetch_add(1, Ordering::Release);
                if instruction.block {
                    let (open, changed) = &*handler_gate;
                    let mut open = open.lock().unwrap();
                    while !*open {
                        open = changed.wait(open).unwrap();
                    }
                }
                Ok(Arc::strong_count(&instruction.payload) as u64)
            },
        )
        .unwrap()
        .with_max_pending(2);
    component.expose().unwrap();

    let scheduler = SharedScheduler::new(1).unwrap();
    let first = operation
        .invoke_async(
            context("peer-a", "test", "blocker"),
            CleanupInstruction {
                payload: Arc::new(()),
                block: true,
            },
            &scheduler.dispatcher(),
            InvocationOptions::default(),
        )
        .unwrap();
    wait_until(Duration::from_secs(1), || {
        entered.load(Ordering::Acquire) == 1
    });

    let payload = Arc::new(());
    let weak = Arc::downgrade(&payload);
    let cancelled = operation
        .invoke_async(
            context("peer-a", "test", "cancelled"),
            CleanupInstruction {
                payload: Arc::clone(&payload),
                block: false,
            },
            &scheduler.dispatcher(),
            InvocationOptions::default(),
        )
        .unwrap();
    drop(payload);
    cancelled.cancel();
    drop(cancelled);
    assert!(
        weak.upgrade().is_some(),
        "queued job still owns its instruction"
    );

    let (open, changed) = &*gate;
    *open.lock().unwrap() = true;
    changed.notify_all();
    first.wait_timeout(Duration::from_secs(1)).unwrap().unwrap();
    wait_until(Duration::from_secs(1), || {
        operation.pending_invocations() == 0 && weak.upgrade().is_none()
    });
    assert_eq!(entered.load(Ordering::Acquire), 1);
}
