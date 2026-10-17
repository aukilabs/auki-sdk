#![cfg(feature = "components")]

use std::cell::Cell;
use std::sync::Arc;

use auki_components::{
    BufferLimits, ComponentRuntime, Envelope, Observation, PayloadContract, ProductAccessError,
    ProductState, RetainedProduct, TimeRangeRequest,
};
use auki_registry::{ClockBody, ClockMeta, Scope};
use auki_time::SessionClock;
use auki_time::components::{
    CONTRACT_SCHEMA, SAMPLE_SCHEMA, SessionClockError, SessionClockPublisher, SessionUtcContract,
    SessionUtcSample, UtcUnavailable, read_session_sample,
};

const UTC_NS: i64 = 1_750_000_000_000_000_000;

fn publisher(peer: &str, session: &str) -> (ComponentRuntime, SessionClockPublisher) {
    let runtime = ComponentRuntime::new(peer);
    let clock = SessionClock::new(peer, session, "capture");
    let publisher = SessionClockPublisher::new(&runtime, &clock).unwrap();
    (runtime, publisher)
}

fn meta_mut(body: &mut ClockBody) -> &mut ClockMeta {
    match body {
        ClockBody::MonotonicClock(meta) | ClockBody::UtcClock(meta) => meta,
    }
}

fn imported(source: &RetainedProduct<SessionUtcSample>) -> RetainedProduct<SessionUtcSample> {
    RetainedProduct::imported_buffer(
        source.manifest.clone(),
        source.manifest_hash.clone(),
        source.producer.clone(),
        BufferLimits::entries(4),
        |_| std::mem::size_of::<SessionUtcSample>(),
    )
    .unwrap()
}

fn append(product: &RetainedProduct<SessionUtcSample>, observation: Observation<SessionUtcSample>) {
    product
        .buffer()
        .append_shared(Arc::new(Envelope::new(
            observation.sequence,
            observation.timestamp_ns,
            observation,
        )))
        .unwrap();
}

fn assert_same_sample(
    actual: &Observation<SessionUtcSample>,
    expected: &Observation<SessionUtcSample>,
) {
    assert_eq!(actual.output, expected.output);
    assert_eq!(actual.sequence, expected.sequence);
    assert_eq!(actual.timestamp_ns, expected.timestamp_ns);
    assert_eq!(actual.payload, expected.payload);
}

fn sample_utc(sample: &Observation<SessionUtcSample>) -> i64 {
    let SessionUtcSample::Available { offset_ns, .. } = *sample.payload else {
        panic!("valid UTC read should be available");
    };
    i64::try_from(sample.timestamp_ns)
        .unwrap()
        .checked_add(offset_ns)
        .unwrap()
}

#[test]
fn catalog_exposes_exact_definitions_and_retained_sample_for_any_peer() {
    assert_eq!(SAMPLE_SCHEMA, "auki.session-utc-sample/v1");
    assert_eq!(CONTRACT_SCHEMA, "auki.session-utc-history/v1");
    for peer in ["Steve", "Bob"] {
        let runtime = ComponentRuntime::new(peer);
        let session = SessionClock::new(peer, "session-1", "capture");
        let original_definition = session.registry_entry();
        let mut publisher = SessionClockPublisher::new(&runtime, &session).unwrap();
        let before = session.now_ns();
        let observation = publisher
            .publish_measurement_with_utc(|| Ok(UTC_NS))
            .unwrap();
        let after = session.now_ns();
        let contract = publisher.contract();
        let product = publisher.product();

        assert_eq!(contract.from_clock, original_definition);
        assert_eq!(contract.from_reference().hash, session.clock_hash());
        assert_eq!(contract.to_reference().hash, contract.to_clock.hash());
        assert_eq!(contract.from_reference().peer_id, peer);
        assert_eq!(contract.to_reference().peer_id, peer);
        assert_eq!(contract.to_clock.session_id, "session-1");
        assert_ne!(contract.from_reference(), contract.to_reference());
        assert_eq!(contract.utc_accuracy_ns, None);
        assert_eq!(product.producer.clock, contract.from_reference());
        assert_eq!(product.manifest.producer, product.producer.reference());
        assert_eq!(observation.output, product.producer.reference());
        assert_eq!(observation.sequence, 0);
        assert!((before..=after).contains(&observation.timestamp_ns));
        assert_eq!(sample_utc(&observation), UTC_NS);
        assert!(contract.output_id().starts_with("session-utc-history-"));
        assert_eq!(product.buffer().limits().max_entries, Some(4096));
        assert_eq!(product.buffer().limits().target_duration, None);

        let snapshot = runtime.catalog().snapshot();
        assert_eq!(snapshot.components.len(), 1);
        assert_eq!(snapshot.products.len(), 1);
        let entry = &snapshot.products[0];
        assert_eq!(entry.manifest, product.manifest);
        assert_eq!(entry.manifest_hash, product.manifest_hash);
        assert!(matches!(
            entry.state,
            ProductState::Buffer { entries: 1, .. }
        ));
        let metadata = entry.metadata.as_ref().unwrap();
        assert_eq!(metadata.schema, CONTRACT_SCHEMA);
        assert_eq!(metadata.source_sequence, observation.sequence);
        let discovered: SessionUtcContract =
            serde_json::from_value(metadata.value.clone()).unwrap();
        assert_eq!(&discovered, contract);
        discovered.validate_output(&product.producer).unwrap();
        assert!(
            matches!(&product.producer.payload, PayloadContract::Structured(payload)
            if payload.datatype == SAMPLE_SCHEMA && payload.schema == SAMPLE_SCHEMA)
        );

        let received = imported(&product);
        append(&received, observation.clone());
        let retained = read_session_sample(&received, &discovered, 0).unwrap();
        assert_same_sample(&retained, &observation);
    }
}

#[test]
fn clocks_with_same_readable_name_remain_distinct_across_peers_and_sessions() {
    let (_runtime_a, a) = publisher("Steve", "session-1");
    let (_runtime_b, b) = publisher("Bob", "session-1");
    let (_runtime_next, next) = publisher("Steve", "session-2");
    for other in [b, next] {
        assert_ne!(
            a.contract().from_reference(),
            other.contract().from_reference()
        );
        assert_ne!(a.contract().to_reference(), other.contract().to_reference());
        assert_ne!(a.product().reference(), other.product().reference());
        assert!(read_session_sample(&a.product(), other.contract(), 0).is_err());
    }
}

#[test]
fn wrong_runtime_owner_and_invalid_session_are_rejected_before_catalog_registration() {
    let runtime = ComponentRuntime::new("Bob");
    let session = SessionClock::new("Steve", "session-1", "capture");
    assert!(matches!(
        SessionClockPublisher::new(&runtime, &session),
        Err(SessionClockError::Invalid(_))
    ));
    assert!(runtime.catalog().snapshot().components.is_empty());
    assert!(runtime.catalog().snapshot().products.is_empty());

    for session in ["", "../invalid"] {
        let clock = SessionClock::new("Bob", session, "capture");
        assert!(SessionClockPublisher::new(&runtime, &clock).is_err());
    }
    assert!(runtime.catalog().snapshot().components.is_empty());
}

#[test]
fn malformed_endpoint_contracts_are_rejected() {
    let (_runtime, publisher) = publisher("Steve", "session-1");
    let original = publisher.contract();
    type ContractMutation = (&'static str, fn(&mut SessionUtcContract));
    let mutations: &[ContractMutation] = &[
        ("source owner empty", |c| c.from_clock.peer_id.clear()),
        ("different owner", |c| c.to_clock.peer_id = "Bob".into()),
        ("different session", |c| {
            c.to_clock.session_id = "session-2".into()
        }),
        ("session empty", |c| c.from_clock.session_id.clear()),
        ("session too long", |c| {
            c.from_clock.session_id = "s".repeat(257)
        }),
        ("same ID", |c| {
            c.to_clock.clock_id = c.from_clock.clock_id.clone()
        }),
        ("invalid ID", |c| {
            c.from_clock.clock_id = "../capture".into()
        }),
        ("source wrong kind", |c| {
            c.from_clock.body = c.to_clock.body.clone()
        }),
        ("destination wrong kind", |c| {
            c.to_clock.body = c.from_clock.body.clone()
        }),
        ("source not monotonic", |c| {
            meta_mut(&mut c.from_clock.body).monotonic = false
        }),
        ("destination monotonic", |c| {
            meta_mut(&mut c.to_clock.body).monotonic = true
        }),
        ("source wrong unit", |c| {
            meta_mut(&mut c.from_clock.body).unit = "ms".into()
        }),
        ("destination wrong unit", |c| {
            meta_mut(&mut c.to_clock.body).unit = "seconds".into()
        }),
        ("source has epoch", |c| {
            meta_mut(&mut c.from_clock.body).epoch = Some("1970-01-01T00:00:00Z".into())
        }),
        ("destination missing epoch", |c| {
            meta_mut(&mut c.to_clock.body).epoch = None
        }),
        ("destination wrong epoch", |c| {
            meta_mut(&mut c.to_clock.body).epoch = Some("2000-01-01T00:00:00Z".into())
        }),
        ("source wrong scope", |c| {
            meta_mut(&mut c.from_clock.body).scope = Scope::Global
        }),
        ("destination wrong scope", |c| {
            meta_mut(&mut c.to_clock.body).scope = Scope::DomainLocal
        }),
    ];
    for (name, mutate) in mutations {
        let mut contract = original.clone();
        mutate(&mut contract);
        assert!(contract.validate().is_err(), "accepted {name}");
        assert!(
            contract
                .validate_output(&publisher.product().producer)
                .is_err(),
            "accepted Output with {name}"
        );
    }
}

#[test]
fn output_contract_identity_and_payload_mismatches_are_rejected() {
    let (_runtime, publisher) = publisher("Steve", "session-1");
    for mutation in [
        "owner",
        "clock-owner",
        "clock-id",
        "clock-hash",
        "output-id",
        "datatype",
        "schema",
        "spatial-frame",
    ] {
        let mut output = publisher.product().producer;
        match mutation {
            "owner" => output.peer_id = "Bob".into(),
            "clock-owner" => output.clock.peer_id = "Bob".into(),
            "clock-id" => output.clock.id = "Steve/session-2/capture".into(),
            "clock-hash" => output.clock.hash = "0".repeat(32),
            "output-id" => output.output_id = "different-contract".into(),
            "spatial-frame" => output.spatial_frame_id = Some("not-a-clock-frame".into()),
            "datatype" | "schema" => {
                let PayloadContract::Structured(payload) = &mut output.payload else {
                    unreachable!()
                };
                if mutation == "datatype" {
                    payload.datatype = "different/v1".into();
                } else {
                    payload.schema = "different/v1".into();
                }
            }
            _ => unreachable!(),
        }
        assert!(
            publisher.contract().validate_output(&output).is_err(),
            "accepted {mutation}"
        );
    }
    let mut changed_accuracy = publisher.contract().clone();
    changed_accuracy.utc_accuracy_ns = Some(1);
    assert_ne!(
        changed_accuracy.output_id(),
        publisher.contract().output_id()
    );
    assert!(
        changed_accuracy
            .validate_output(&publisher.product().producer)
            .is_err()
    );
}

#[test]
fn repeated_measurements_retain_history_across_forward_and_backward_utc_steps() {
    let runtime = ComponentRuntime::new("Steve");
    let session = SessionClock::new("Steve", "session-1", "capture");
    let original_definition = session.registry_entry();
    let before_construction = session.now_ns();
    let mut publisher = SessionClockPublisher::new(&runtime, &session).unwrap();
    let contract = publisher.contract().clone();
    let product = publisher.product();
    let product_reference = product.reference();
    let output_reference = product.producer.reference();
    let initial_catalog = runtime.catalog().snapshot();
    let descriptor = serde_json::to_value(&contract).unwrap();
    let mut samples = Vec::new();

    for (sequence, utc) in [UTC_NS, UTC_NS + 30_000_000_000, UTC_NS - 30_000_000_000]
        .into_iter()
        .enumerate()
    {
        let before = session.now_ns();
        let sample = publisher.publish_measurement_with_utc(|| Ok(utc)).unwrap();
        let after = session.now_ns();
        assert_eq!(sample.sequence, sequence as u64);
        assert_eq!(sample.output, output_reference);
        assert!((before..=after).contains(&sample.timestamp_ns));
        assert!(sample.timestamp_ns >= before_construction);
        assert_eq!(sample_utc(&sample), utc);
        if let Some(previous) = samples.last() {
            let previous: &Observation<SessionUtcSample> = previous;
            assert!(sample.timestamp_ns >= previous.timestamp_ns);
        }
        samples.push(sample);

        assert_eq!(publisher.contract(), &contract);
        assert_eq!(session.registry_entry(), original_definition);
        assert_eq!(publisher.product().reference(), product_reference);
        assert_eq!(publisher.product().producer.reference(), output_reference);
        let catalog = runtime.catalog().snapshot();
        assert_eq!(catalog.components, initial_catalog.components);
        assert_eq!(catalog.products.len(), 1);
        let entry = &catalog.products[0];
        assert_eq!(entry.manifest, product.manifest);
        assert_eq!(entry.manifest_hash, product.manifest_hash);
        assert!(
            matches!(entry.state, ProductState::Buffer { entries, .. } if entries == samples.len())
        );
        let metadata = entry.metadata.as_ref().unwrap();
        assert_eq!(metadata.schema, CONTRACT_SCHEMA);
        assert_eq!(metadata.source_sequence, sequence as u64);
        assert_eq!(metadata.value, descriptor);

        // The Product handle obtained before publication observes all retained samples.
        for earlier in &samples {
            let retained = read_session_sample(&product, &contract, earlier.sequence).unwrap();
            assert_same_sample(&retained, earlier);
        }
    }
    let range = product.buffer().range();
    assert_eq!(range.entries, 3);
    assert_eq!(range.first_sequence, Some(0));
    assert_eq!(range.last_sequence, Some(2));
}

#[test]
fn duplicate_publishers_cannot_replace_or_reset_existing_history() {
    let (runtime, mut publisher) = publisher("Steve", "session-1");
    let first = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let catalog = runtime.catalog().snapshot();
    let duplicate = SessionClock::new("Steve", "session-1", "capture");
    assert!(SessionClockPublisher::new(&runtime, &duplicate).is_err());
    assert!(SessionClockPublisher::with_capacity(&runtime, &duplicate, 2).is_err());
    assert_eq!(runtime.catalog().snapshot(), catalog);
    assert_same_sample(
        &read_session_sample(&publisher.product(), publisher.contract(), 0).unwrap(),
        &first,
    );
    let second = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    assert_eq!(second.sequence, 1);
    assert_eq!(second.output, first.output);
}

#[test]
fn second_session_publishes_a_distinct_history_without_replacing_the_first() {
    let (runtime, mut first) = publisher("Steve", "session-1");
    let first_sample = first.publish_measurement_with_utc(|| Ok(UTC_NS)).unwrap();
    let next_clock = SessionClock::new("Steve", "session-2", "capture");
    let mut next = SessionClockPublisher::new(&runtime, &next_clock).unwrap();
    let next_sample = next.publish_measurement_with_utc(|| Ok(UTC_NS)).unwrap();
    assert_eq!(first_sample.sequence, 0);
    assert_eq!(next_sample.sequence, 0);
    assert_ne!(first.product().reference(), next.product().reference());
    assert_ne!(first_sample.output, next_sample.output);
    assert_ne!(
        first.contract().from_reference(),
        next.contract().from_reference()
    );
    assert_ne!(
        first.contract().to_reference(),
        next.contract().to_reference()
    );
    assert_eq!(runtime.catalog().snapshot().products.len(), 2);
    assert_same_sample(
        &read_session_sample(&first.product(), first.contract(), 0).unwrap(),
        &first_sample,
    );
    assert!(read_session_sample(&first.product(), next.contract(), 0).is_err());
}

#[test]
fn sample_profile_round_trips_without_accepting_legacy_or_unknown_fields() {
    for sample in [
        SessionUtcSample::Available {
            offset_ns: 123,
            uncertainty_ns: 4,
        },
        SessionUtcSample::Unavailable {
            reason: UtcUnavailable::ReadFailed,
        },
    ] {
        let value = serde_json::to_value(&sample).unwrap();
        assert_eq!(
            serde_json::from_value::<SessionUtcSample>(value.clone()).unwrap(),
            sample
        );
        let mut unknown = value;
        unknown["future_field"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SessionUtcSample>(unknown).is_err());
    }
    assert!(
        serde_json::from_value::<SessionUtcSample>(serde_json::json!({
            "offset_ns": 123, "uncertainty_ns": 4
        }))
        .is_err()
    );
}

#[test]
fn unavailable_utc_is_retained_explicitly_and_later_measurements_recover() {
    for reason in [
        UtcUnavailable::ReadFailed,
        UtcUnavailable::OutOfRange,
        UtcUnavailable::InvalidSample,
    ] {
        let (runtime, mut publisher) = publisher("Bob", "session-1");
        let unavailable = publisher
            .publish_measurement_with_utc(|| Err(reason))
            .unwrap();
        assert_eq!(
            *unavailable.payload,
            SessionUtcSample::Unavailable { reason }
        );
        assert_eq!(unavailable.sequence, 0);
        let recovered = publisher
            .publish_measurement_with_utc(|| Ok(UTC_NS))
            .unwrap();
        assert_eq!(recovered.sequence, 1);
        assert_eq!(sample_utc(&recovered), UTC_NS);
        assert_eq!(recovered.output, unavailable.output);
        for sample in [&unavailable, &recovered] {
            let retained =
                read_session_sample(&publisher.product(), publisher.contract(), sample.sequence)
                    .unwrap();
            assert_same_sample(&retained, sample);
        }
        let catalog = runtime.catalog().snapshot();
        assert_eq!(
            catalog.products[0]
                .metadata
                .as_ref()
                .unwrap()
                .source_sequence,
            1
        );
    }
}

#[test]
fn imported_mismatched_observation_and_product_provenance_is_rejected() {
    let (_runtime, mut publisher) = publisher("Steve", "session-1");
    let sample = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let mismatched = imported(&publisher.product());
    let mut changed = sample.clone();
    changed.output.peer_id = "Bob".into();
    append(&mismatched, changed);
    assert!(matches!(
        read_session_sample(&mismatched, publisher.contract(), 0),
        Err(SessionClockError::Invalid(_))
    ));

    for mutation in ["manifest-hash", "manifest-owner", "producer"] {
        let mut changed = publisher.product();
        match mutation {
            "manifest-hash" => changed.manifest_hash = "0".repeat(32),
            "manifest-owner" => {
                changed.manifest.peer_id = "Bob".into();
                changed.manifest_hash = changed.manifest.hash();
            }
            "producer" => {
                changed.manifest.producer.manifest_hash = "0".repeat(32);
                changed.manifest_hash = changed.manifest.hash();
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                read_session_sample(&changed, publisher.contract(), 0),
                Err(SessionClockError::Invalid(_))
            ),
            "accepted {mutation}"
        );
    }
}

#[test]
fn zero_capacity_is_rejected_before_catalog_registration() {
    let runtime = ComponentRuntime::new("Steve");
    let session = SessionClock::new("Steve", "session-1", "capture");
    let catalog = runtime.catalog().snapshot();
    assert!(matches!(
        SessionClockPublisher::with_capacity(&runtime, &session, 0),
        Err(SessionClockError::Invalid(_))
    ));
    assert_eq!(runtime.catalog().snapshot(), catalog);
    // A rejected constructor must not reserve the session's Component/Product IDs.
    assert!(SessionClockPublisher::with_capacity(&runtime, &session, 1).is_ok());
}

#[test]
fn full_history_rejects_before_utc_read_without_eviction_or_catalog_changes() {
    let runtime = ComponentRuntime::new("Steve");
    let session = SessionClock::new("Steve", "session-1", "capture");
    let mut publisher = SessionClockPublisher::with_capacity(&runtime, &session, 2).unwrap();
    let product = publisher.product();
    assert_eq!(product.buffer().limits().max_entries, Some(2));
    assert_eq!(product.buffer().limits().target_duration, None);
    let first = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let second = publisher
        .publish_measurement_with_utc(|| Err(UtcUnavailable::ReadFailed))
        .unwrap();
    let range = product.buffer().range();
    let catalog = runtime.catalog().snapshot();
    assert!(matches!(
        catalog.products[0].state,
        ProductState::Buffer {
            entries: 2,
            at_entry_capacity: true,
            ..
        }
    ));

    for _ in 0..2 {
        let reader_called = Cell::new(false);
        let result = publisher.publish_measurement_with_utc(|| {
            reader_called.set(true);
            Ok(UTC_NS + 30_000_000_000)
        });
        assert!(matches!(
            result,
            Err(SessionClockError::HistoryFull { capacity: 2 })
        ));
        assert!(!reader_called.get());
        assert_eq!(product.buffer().range(), range);
        assert_eq!(runtime.catalog().snapshot(), catalog);
        for sample in [&first, &second] {
            assert_same_sample(
                &read_session_sample(&product, publisher.contract(), sample.sequence).unwrap(),
                sample,
            );
        }
        assert!(matches!(
            read_session_sample(&product, publisher.contract(), 2),
            Err(SessionClockError::MissingSample { sequence: 2 })
        ));
    }
}

#[test]
fn exact_reads_report_empty_future_and_missing_sequences_without_using_latest() {
    let (_runtime, mut publisher) = publisher("Steve", "session-1");
    let product = publisher.product();
    assert!(matches!(
        read_session_sample(&product, publisher.contract(), 0),
        Err(SessionClockError::MissingSample { sequence: 0 })
    ));
    let mut samples = Vec::new();
    for utc in [UTC_NS, UTC_NS + 1_000_000, UTC_NS + 2_000_000] {
        samples.push(publisher.publish_measurement_with_utc(|| Ok(utc)).unwrap());
    }
    assert_same_sample(
        &read_session_sample(&product, publisher.contract(), 0).unwrap(),
        &samples[0],
    );
    assert!(matches!(
        read_session_sample(&product, publisher.contract(), 3),
        Err(SessionClockError::MissingSample { sequence: 3 })
    ));

    let received = imported(&product);
    append(&received, samples[0].clone());
    append(&received, samples[2].clone());
    for sample in [&samples[0], &samples[2]] {
        assert_same_sample(
            &read_session_sample(&received, publisher.contract(), sample.sequence).unwrap(),
            sample,
        );
    }
    for sequence in [1, 3, u64::MAX] {
        assert!(matches!(
            read_session_sample(&received, publisher.contract(), sequence),
            Err(SessionClockError::MissingSample { sequence: missing }) if missing == sequence
        ));
    }
}

#[test]
fn imported_envelope_and_observation_must_agree_on_sequence_time_and_output() {
    let (_runtime, mut publisher) = publisher("Steve", "session-1");
    let sample = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    for mutation in [
        "envelope-sequence",
        "observation-sequence",
        "envelope-time",
        "output-owner",
        "output-id",
        "output-hash",
        "timestamp-overflow",
        "utc-overflow",
        "negative-utc",
    ] {
        let received = imported(&publisher.product());
        let mut changed = sample.clone();
        let mut envelope_sequence = sample.sequence;
        let mut envelope_timestamp = sample.timestamp_ns;
        match mutation {
            "envelope-sequence" => envelope_sequence += 1,
            "observation-sequence" => changed.sequence += 1,
            "envelope-time" => envelope_timestamp += 1,
            "output-owner" => changed.output.peer_id = "Bob".into(),
            "output-id" => changed.output.output_id = "unrelated-output".into(),
            "output-hash" => changed.output.manifest_hash = "0".repeat(32),
            "timestamp-overflow" => {
                changed.timestamp_ns = u64::MAX;
                envelope_timestamp = changed.timestamp_ns;
            }
            "utc-overflow" => {
                changed.timestamp_ns = 1;
                envelope_timestamp = changed.timestamp_ns;
                changed.payload = Arc::new(SessionUtcSample::Available {
                    offset_ns: i64::MAX,
                    uncertainty_ns: 0,
                });
            }
            "negative-utc" => {
                changed.payload = Arc::new(SessionUtcSample::Available {
                    offset_ns: i64::MIN,
                    uncertainty_ns: 0,
                });
            }
            _ => unreachable!(),
        }
        received
            .buffer()
            .append_shared(Arc::new(Envelope::new(
                envelope_sequence,
                envelope_timestamp,
                changed,
            )))
            .unwrap();
        assert!(
            matches!(
                read_session_sample(&received, publisher.contract(), envelope_sequence),
                Err(SessionClockError::Invalid(_))
            ),
            "accepted {mutation}"
        );
    }
}

#[test]
fn time_range_queries_require_the_exact_source_clock() {
    let (_runtime, mut publisher) = publisher("Steve", "session-1");
    let first = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let second = publisher
        .publish_measurement_with_utc(|| Ok(UTC_NS - 30_000_000_000))
        .unwrap();
    let product = publisher.product();
    let source = publisher.contract().from_reference();
    let samples = product
        .time_range(TimeRangeRequest {
            clock: source.clone(),
            start_ns: first.timestamp_ns,
            end_ns: second.timestamp_ns,
        })
        .unwrap();
    assert_eq!(samples.len(), 2);
    assert_same_sample(&samples.observations[0], &first);
    assert_same_sample(&samples.observations[1], &second);

    for mutation in ["utc", "owner", "id", "hash"] {
        let mut clock = source.clone();
        match mutation {
            "utc" => clock = publisher.contract().to_reference(),
            "owner" => clock.peer_id = "Bob".into(),
            "id" => clock.id = "Steve/session-2/capture".into(),
            "hash" => clock.hash = "0".repeat(32),
            _ => unreachable!(),
        }
        assert!(
            matches!(
                product.time_range(TimeRangeRequest {
                    clock,
                    start_ns: 0,
                    end_ns: u64::MAX,
                }),
                Err(ProductAccessError::ClockMismatch { .. })
            ),
            "accepted {mutation}"
        );
    }
}
