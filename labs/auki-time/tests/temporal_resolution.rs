#![cfg(feature = "components")]

use std::sync::Arc;

use auki_components::{
    BufferLimits, ClockReference, Envelope, Observation, ObservationAccess, OutputManifest,
    PayloadContract, ProductForm, ProductManifest, RetainedProduct, StructuredPayloadContract,
};
use auki_registry::{ClockBody, ClockMeta, ClockRegistryEntry, Scope};
use auki_time::SessionClock;
use auki_time::components::{SessionClockPublisher, SessionUtcSample, UtcUnavailable};
use auki_time::resolver::{
    OffsetContract, OffsetSample, ResolutionIssue, ResolverLimits, TemporalError, TemporalQuery,
    TemporalResolver, TimeInterval,
};

const SECOND: u64 = 1_000_000_000;
const MILLISECOND: u64 = 1_000_000;

fn monotonic(peer: &str, session: &str) -> ClockRegistryEntry {
    ClockRegistryEntry {
        peer_id: peer.into(),
        session_id: session.into(),
        clock_id: format!("{peer}/{session}/capture"),
        body: ClockBody::MonotonicClock(ClockMeta {
            unit: "ns".into(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        }),
    }
}

fn utc(peer: &str, session: &str) -> ClockRegistryEntry {
    ClockRegistryEntry {
        clock_id: format!("{peer}/{session}/utc"),
        body: ClockBody::UtcClock(ClockMeta {
            unit: "ns".into(),
            monotonic: false,
            epoch: Some("1970-01-01T00:00:00Z".into()),
            scope: Scope::DeviceLocal,
        }),
        ..monotonic(peer, session)
    }
}

fn reference(clock: &ClockRegistryEntry) -> ClockReference {
    ClockReference {
        peer_id: clock.peer_id.clone(),
        id: clock.clock_id.clone(),
        hash: clock.hash(),
    }
}

fn append<T>(product: &RetainedProduct<T>, sequence: u64, timestamp_ns: u64, payload: T) {
    let observation = Observation {
        output: product.producer.reference(),
        sequence,
        timestamp_ns,
        payload: Arc::new(payload),
    };
    product
        .buffer()
        .append_shared(Arc::new(Envelope::new(sequence, timestamp_ns, observation)))
        .unwrap();
}

fn resolver(clocks: &[&ClockRegistryEntry]) -> TemporalResolver {
    let mut resolver = TemporalResolver::new(ResolverLimits::default()).unwrap();
    for clock in clocks {
        resolver
            .register_clock(reference(clock), (*clock).clone())
            .unwrap();
    }
    resolver
}

fn interval(start_ns: u64, end_ns: u64) -> TimeInterval {
    TimeInterval { start_ns, end_ns }
}

fn contract(from: &ClockRegistryEntry, to: &ClockRegistryEntry) -> OffsetContract {
    OffsetContract {
        from_clock: reference(from),
        to_clock: reference(to),
        allow_inverse: true,
    }
}

fn offset_product(
    owner: &str,
    name: &str,
    contract: &OffsetContract,
) -> RetainedProduct<OffsetSample> {
    let producer = OutputManifest {
        schema: "auki.component-output-manifest/v3".into(),
        peer_id: owner.into(),
        component_id: name.into(),
        component_manifest_hash: "a".repeat(32),
        slot: "measurements".into(),
        output_id: contract.output_id(),
        clock: contract.from_clock.clone(),
        spatial_frame_id: None,
        spatial_frame: None,
        payload: contract.payload_contract(),
    };
    let manifest = ProductManifest {
        schema: "auki.product-manifest/v1".into(),
        peer_id: owner.into(),
        product_id: name.into(),
        form: ProductForm::Buffer,
        producer: producer.reference(),
        access: vec![
            ObservationAccess::LatestExisting,
            ObservationAccess::TimeRange,
        ],
    };
    RetainedProduct::imported_buffer(
        manifest.clone(),
        manifest.hash(),
        producer,
        BufferLimits::entries(32),
        |_| std::mem::size_of::<OffsetSample>(),
    )
    .unwrap()
}

fn sample(offset_ns: i64, uncertainty_ns: u64, validity: TimeInterval) -> OffsetSample {
    OffsetSample::Available {
        offset_ns,
        uncertainty_ns: Some(uncertainty_ns),
        max_drift_ppb: Some(0),
        validity,
    }
}

fn add(
    resolver: &mut TemporalResolver,
    owner: &str,
    name: &str,
    from: &ClockRegistryEntry,
    to: &ClockRegistryEntry,
    offset_ns: i64,
    uncertainty_ns: u64,
) {
    let contract = contract(from, to);
    let product = offset_product(owner, name, &contract);
    append(
        &product,
        0,
        100 * SECOND,
        sample(offset_ns, uncertainty_ns, interval(0, 1_000 * SECOND)),
    );
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
}

fn query(from: &ClockRegistryEntry, to: &ClockRegistryEntry, timestamp_ns: u64) -> TemporalQuery {
    TemporalQuery {
        from_clock: reference(from),
        to_clock: reference(to),
        timestamp_ns,
        input_uncertainty_ns: 0,
        max_uncertainty_ns: 100 * SECOND,
    }
}

fn assert_issue(
    result: Result<auki_time::resolver::TemporalResolution, TemporalError>,
    issue: ResolutionIssue,
) {
    match result {
        Err(TemporalError::Unresolved { issues }) => assert!(issues.contains(&issue), "{issues:?}"),
        other => panic!("expected unresolved {issue:?}, got {other:?}"),
    }
}

#[test]
fn steve_and_bob_require_an_explicit_bridge_between_their_local_utc_clocks() {
    let steve = monotonic("Steve", "boot-a");
    let bob = monotonic("Bob", "boot-b");
    let steve_utc = utc("Steve", "boot-a");
    let bob_utc = utc("Bob", "boot-b");
    let mut resolver = resolver(&[&steve, &bob, &steve_utc, &bob_utc]);
    add(
        &mut resolver,
        "Steve",
        "steve-utc",
        &steve,
        &steve_utc,
        100 * SECOND as i64,
        MILLISECOND,
    );
    add(
        &mut resolver,
        "Bob",
        "bob-utc",
        &bob,
        &bob_utc,
        90 * SECOND as i64,
        MILLISECOND,
    );

    assert!(matches!(
        resolver.resolve(&query(&steve, &bob, 2 * SECOND)),
        Err(TemporalError::Unresolved { .. })
    ));

    // The bridge can be published by a third peer. No endpoint is privileged as Home.
    add(
        &mut resolver,
        "independent-observer",
        "utc-bridge",
        &steve_utc,
        &bob_utc,
        20 * SECOND as i64,
        2 * MILLISECOND,
    );
    let result = resolver.resolve(&query(&steve, &bob, 2 * SECOND)).unwrap();
    assert_eq!(result.timestamp_ns, 32 * SECOND);
    assert_eq!(result.uncertainty_ns, 4 * MILLISECOND);
    assert_eq!(result.path.len(), 3);
    assert_eq!(
        result.path[1].evidence.output.peer_id,
        "independent-observer"
    );
    assert!(result.path[2].inverted);
    assert!(result.source_coverage.start_ns <= 2 * SECOND);
    assert!(result.source_coverage.end_ns >= 2 * SECOND);

    let reverse = resolver.resolve(&query(&bob, &steve, 32 * SECOND)).unwrap();
    assert_eq!(reverse.timestamp_ns, 2 * SECOND);
    assert_eq!(reverse.uncertainty_ns, 4 * MILLISECOND);
}

#[test]
fn an_explicitly_shared_registry_clock_connects_peers_without_a_synthetic_utc_alias() {
    let a = monotonic("first", "one");
    let b = monotonic("second", "two");
    let mut shared = utc("clock-owner", "epoch-one");
    if let ClockBody::UtcClock(meta) = &mut shared.body {
        meta.scope = Scope::Global;
    }
    let mut resolver = resolver(&[&a, &b, &shared]);
    add(
        &mut resolver,
        "first",
        "first-shared",
        &a,
        &shared,
        100 * SECOND as i64,
        MILLISECOND,
    );
    add(
        &mut resolver,
        "second",
        "second-shared",
        &b,
        &shared,
        70 * SECOND as i64,
        MILLISECOND,
    );
    let result = resolver.resolve(&query(&a, &b, 2 * SECOND)).unwrap();
    assert_eq!(result.timestamp_ns, 32 * SECOND);
    assert_eq!(result.uncertainty_ns, 2 * MILLISECOND);
    assert_eq!(result.path.len(), 2);
}

#[test]
fn registry_identity_includes_owner_session_and_definition_hash() {
    let a = monotonic("same-peer", "first-boot");
    let b = monotonic("same-peer", "second-boot");
    let mut resolver = resolver(&[&a]);
    assert!(matches!(
        resolver.resolve(&query(&a, &b, 0)),
        Err(TemporalError::MissingClock(_))
    ));
    let mut wrong_hash = reference(&a);
    wrong_hash.hash = "0".repeat(32);
    assert!(matches!(
        resolver.register_clock(wrong_hash, a.clone()),
        Err(TemporalError::Invalid(_))
    ));
    let mut wrong_owner = reference(&a);
    wrong_owner.peer_id = "other-peer".into();
    assert!(matches!(
        resolver.register_clock(wrong_owner, a.clone()),
        Err(TemporalError::Invalid(_))
    ));
    let mut changed_definition = a.clone();
    changed_definition.session_id = "replacement-boot".into();
    assert!(matches!(
        resolver.register_clock(reference(&a), changed_definition.clone()),
        Err(TemporalError::Invalid(_))
    ));
    // Even reusing the readable clock ID cannot carry an old transform across
    // a session change: the new definition has a different exact reference.
    assert!(matches!(
        resolver.resolve(&query(&a, &changed_definition, 0)),
        Err(TemporalError::MissingClock(_))
    ));
    resolver
        .register_clock(reference(&changed_definition), changed_definition.clone())
        .unwrap();
    assert!(matches!(
        resolver.resolve(&query(&a, &changed_definition, 0)),
        Err(TemporalError::Unresolved { .. })
    ));
    resolver.register_clock(reference(&b), b.clone()).unwrap();
    assert!(matches!(
        resolver.resolve(&query(&a, &b, 0)),
        Err(TemporalError::Unresolved { .. })
    ));
}

#[test]
fn host_admission_is_explicit_even_for_well_formed_third_peer_evidence() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let contract = contract(&a, &b);
    let product = offset_product("unapproved-publisher", "bridge", &contract);
    append(&product, 0, 10, sample(20, 1, interval(0, 100)));
    assert!(matches!(
        resolver.insert_offset_sample(&product, &contract, 0, |_| false),
        Err(TemporalError::Denied)
    ));
    assert!(matches!(
        resolver.resolve(&query(&a, &b, 10)),
        Err(TemporalError::Unresolved { .. })
    ));
    resolver
        .insert_offset_sample(&product, &contract, 0, |e| {
            e.output.peer_id == "unapproved-publisher"
        })
        .unwrap();
    assert_eq!(
        resolver.resolve(&query(&a, &b, 10)).unwrap().timestamp_ns,
        30
    );
}

#[test]
fn full_uncertainty_must_fit_source_coverage_in_both_directions() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let contract = contract(&a, &b);
    let product = offset_product("a", "coverage", &contract);
    append(&product, 0, 15, sample(100, 2, interval(10, 20)));
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();

    let mut request = query(&a, &b, 10);
    request.input_uncertainty_ns = 1;
    assert_issue(resolver.resolve(&request), ResolutionIssue::OutsideCoverage);
    request.timestamp_ns = 12;
    let result = resolver.resolve(&request).unwrap();
    assert_eq!(result.timestamp_ns, 112);
    assert_eq!(result.uncertainty_ns, 3);
    assert_eq!(result.source_coverage, interval(11, 19));

    let mut reverse = query(&b, &a, 115);
    reverse.input_uncertainty_ns = 1;
    let result = resolver.resolve(&reverse).unwrap();
    assert_eq!(result.uncertainty_ns, 3);
    assert_eq!(result.source_coverage, interval(113, 117));
    reverse.timestamp_ns = 111;
    assert_issue(resolver.resolve(&reverse), ResolutionIssue::OutsideCoverage);
}

#[test]
fn inverse_is_not_assumed_when_contract_forbids_it() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let mut contract = contract(&a, &b);
    contract.allow_inverse = false;
    let product = offset_product("a", "one-way", &contract);
    append(&product, 0, 10, sample(20, 0, interval(0, 100)));
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    assert_eq!(
        resolver.resolve(&query(&a, &b, 10)).unwrap().timestamp_ns,
        30
    );
    assert!(matches!(
        resolver.resolve(&query(&b, &a, 30)),
        Err(TemporalError::Unresolved { .. })
    ));
}

#[test]
fn unknown_uncertainty_or_drift_never_becomes_a_zero_error_bound() {
    for (uncertainty_ns, max_drift_ppb, validity) in [
        (None, Some(0), interval(10, 10)),
        (Some(0), None, interval(0, 20)),
    ] {
        let a = monotonic("a", "one");
        let b = monotonic("b", "one");
        let mut resolver = resolver(&[&a, &b]);
        let contract = contract(&a, &b);
        let product = offset_product("a", "unknown", &contract);
        append(
            &product,
            0,
            10,
            OffsetSample::Available {
                offset_ns: 20,
                uncertainty_ns,
                max_drift_ppb,
                validity,
            },
        );
        resolver
            .insert_offset_sample(&product, &contract, 0, |_| true)
            .unwrap();
        assert_issue(
            resolver.resolve(&query(&a, &b, 10)),
            ResolutionIssue::UnknownUncertainty,
        );
    }
}

#[test]
fn declared_drift_increases_uncertainty_and_query_tolerance_is_enforced() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let contract = contract(&a, &b);
    let product = offset_product("a", "drift", &contract);
    append(
        &product,
        0,
        10 * SECOND,
        OffsetSample::Available {
            offset_ns: 50,
            uncertainty_ns: Some(3),
            max_drift_ppb: Some(100),
            validity: interval(0, 20 * SECOND),
        },
    );
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    let mut request = query(&a, &b, 10 * SECOND);
    assert_eq!(resolver.resolve(&request).unwrap().uncertainty_ns, 1_003);
    request.max_uncertainty_ns = 1_002;
    assert_issue(
        resolver.resolve(&request),
        ResolutionIssue::UncertaintyExceeded,
    );
}

#[test]
fn conflicting_direct_and_multihop_paths_are_reported_not_ranked_away() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let c = monotonic("c", "one");
    let mut resolver = resolver(&[&a, &b, &c]);
    add(&mut resolver, "a", "direct", &a, &c, 10, 1);
    add(&mut resolver, "a", "first", &a, &b, 0, 1);
    add(&mut resolver, "b", "second", &b, &c, 20, 1);
    match resolver.resolve(&query(&a, &c, SECOND)) {
        Err(TemporalError::Conflict { candidates }) => {
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate.timestamp_ns == SECOND + 10)
            );
            assert!(
                candidates
                    .iter()
                    .any(|candidate| candidate.timestamp_ns == SECOND + 20)
            );
        }
        other => panic!("expected conflicting path evidence, got {other:?}"),
    }
}

#[test]
fn conflicting_evidence_is_not_hidden_by_the_requested_uncertainty_limit() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    add(&mut resolver, "a", "precise", &a, &b, 0, 1);
    add(&mut resolver, "b", "less-precise", &a, &b, 100, 10);
    let mut request = query(&a, &b, SECOND);
    request.max_uncertainty_ns = 1;
    assert!(matches!(
        resolver.resolve(&request),
        Err(TemporalError::Conflict { .. })
    ));
}

#[test]
fn query_overflow_and_underflow_are_errors_not_saturated_timestamps() {
    for (timestamp, offset) in [(u64::MAX - 5, 10), (5, -10)] {
        let a = monotonic("a", "one");
        let b = monotonic("b", "one");
        let mut resolver = resolver(&[&a, &b]);
        let contract = contract(&a, &b);
        let product = offset_product("a", "overflow", &contract);
        append(
            &product,
            0,
            timestamp,
            sample(offset, 0, interval(timestamp, timestamp)),
        );
        resolver
            .insert_offset_sample(&product, &contract, 0, |_| true)
            .unwrap();
        assert_issue(
            resolver.resolve(&query(&a, &b, timestamp)),
            ResolutionIssue::ArithmeticOverflow,
        );
    }
}

#[test]
fn later_samples_preserve_historical_queries_and_do_not_fill_gaps() {
    let runtime = auki_components::ComponentRuntime::new("Steve");
    let session = SessionClock::new("Steve", "session-one", "capture");
    let publisher = SessionClockPublisher::new(&runtime, &session).unwrap();
    let contract = publisher.contract();
    let source = publisher.product();
    // A deterministic imported history, without mutating the publisher's buffer
    // or pretending a queued capture timestamp is a live clock reading.
    let product = RetainedProduct::imported_buffer(
        source.manifest.clone(),
        source.manifest_hash.clone(),
        source.producer.clone(),
        BufferLimits::entries(8),
        |_| std::mem::size_of::<SessionUtcSample>(),
    )
    .unwrap();
    append(
        &product,
        0,
        10 * SECOND,
        SessionUtcSample::Available {
            offset_ns: 100 * SECOND as i64,
            uncertainty_ns: 3,
        },
    );
    let mut resolver = resolver(&[&contract.from_clock, &contract.to_clock]);
    resolver
        .insert_session_sample(&product, contract, 0, |_| true)
        .unwrap();
    let historical = query(&contract.from_clock, &contract.to_clock, 10 * SECOND);
    let before = resolver.resolve(&historical).unwrap();
    assert_eq!(before.timestamp_ns, 110 * SECOND);

    append(
        &product,
        1,
        20 * SECOND,
        SessionUtcSample::Available {
            offset_ns: 105 * SECOND as i64,
            uncertainty_ns: 4,
        },
    );
    resolver
        .insert_session_sample(&product, contract, 1, |_| true)
        .unwrap();
    let after = resolver.resolve(&historical).unwrap();
    assert_eq!(after.timestamp_ns, before.timestamp_ns);
    assert_eq!(after.path[0].evidence.sequence, 0);
    assert_eq!(
        after.path[0].evidence.source_payload_hash,
        before.path[0].evidence.source_payload_hash
    );
    assert_eq!(
        resolver
            .resolve(&query(
                &contract.from_clock,
                &contract.to_clock,
                20 * SECOND
            ))
            .unwrap()
            .timestamp_ns,
        125 * SECOND
    );
    assert_issue(
        resolver.resolve(&query(
            &contract.from_clock,
            &contract.to_clock,
            15 * SECOND,
        )),
        ResolutionIssue::OutsideCoverage,
    );

    append(
        &product,
        2,
        30 * SECOND,
        SessionUtcSample::Unavailable {
            reason: UtcUnavailable::ReadFailed,
        },
    );
    resolver
        .insert_session_sample(&product, contract, 2, |_| true)
        .unwrap();
    assert_issue(
        resolver.resolve(&query(
            &contract.from_clock,
            &contract.to_clock,
            30 * SECOND,
        )),
        ResolutionIssue::Unavailable,
    );
    assert_eq!(
        resolver.resolve(&historical).unwrap().timestamp_ns,
        110 * SECOND
    );
    assert!(matches!(
        resolver.insert_session_sample(&product, contract, 99, |_| true),
        Err(TemporalError::MissingSample { sequence: 99 })
    ));
}

#[test]
fn retained_evidence_is_an_immutable_snapshot_not_a_live_buffer_alias() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let contract = contract(&a, &b);
    let product = offset_product("a", "snapshot", &contract);
    append(&product, 0, 10, sample(20, 1, interval(0, 100)));
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    product
        .buffer()
        .set_limits(BufferLimits::entries(1))
        .unwrap();
    append(&product, 1, 200, sample(50, 1, interval(200, 300)));
    assert!(product.buffer().snapshot(0, 0).is_empty());
    assert_eq!(
        resolver.resolve(&query(&a, &b, 10)).unwrap().timestamp_ns,
        30
    );
}

#[test]
fn bounded_search_fails_explicitly_instead_of_returning_a_partial_answer() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let c = monotonic("c", "one");
    for limits in [
        ResolverLimits {
            max_hops: 1,
            ..ResolverLimits::default()
        },
        ResolverLimits {
            max_expansions: 1,
            ..ResolverLimits::default()
        },
        ResolverLimits {
            max_candidates: 1,
            ..ResolverLimits::default()
        },
    ] {
        let mut resolver = TemporalResolver::new(limits).unwrap();
        for clock in [&a, &b, &c] {
            resolver
                .register_clock(reference(clock), clock.clone())
                .unwrap();
        }
        add(&mut resolver, "a", "direct", &a, &c, 0, 0);
        add(&mut resolver, "a", "first", &a, &b, 0, 0);
        add(&mut resolver, "b", "second", &b, &c, 0, 0);
        assert!(matches!(
            resolver.resolve(&query(&a, &c, SECOND)),
            Err(TemporalError::LimitExceeded { .. })
        ));
    }
}

#[test]
fn insertion_limits_are_bounded_and_missing_definitions_are_not_inferred() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = TemporalResolver::new(ResolverLimits {
        max_clocks: 1,
        ..ResolverLimits::default()
    })
    .unwrap();
    resolver.register_clock(reference(&a), a.clone()).unwrap();
    assert!(matches!(
        resolver.register_clock(reference(&b), b.clone()),
        Err(TemporalError::LimitExceeded { .. })
    ));
    let contract = contract(&a, &b);
    let product = offset_product("a", "missing-clock", &contract);
    append(&product, 0, 10, sample(20, 0, interval(0, 100)));
    assert!(matches!(
        resolver.insert_offset_sample(&product, &contract, 0, |_| true),
        Err(TemporalError::MissingClock(_))
    ));

    let mut resolver = TemporalResolver::new(ResolverLimits {
        max_samples: 1,
        ..ResolverLimits::default()
    })
    .unwrap();
    for clock in [&a, &b] {
        resolver
            .register_clock(reference(clock), clock.clone())
            .unwrap();
    }
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    // Exact repeated ingestion is harmless and does not exhaust the sample budget.
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    append(&product, 1, 200, sample(30, 0, interval(200, 300)));
    assert!(matches!(
        resolver.insert_offset_sample(&product, &contract, 1, |_| true),
        Err(TemporalError::LimitExceeded { .. })
    ));
    assert_eq!(
        resolver.resolve(&query(&a, &b, 10)).unwrap().timestamp_ns,
        30
    );
}

#[test]
fn malformed_schema_and_observation_provenance_are_rejected_before_admission() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let contract = contract(&a, &b);
    for variant in 0..4 {
        let mut resolver = resolver(&[&a, &b]);
        let mut product = offset_product("a", "invalid", &contract);
        let mut observation = Observation {
            output: product.producer.reference(),
            sequence: 0,
            timestamp_ns: 10,
            payload: Arc::new(sample(20, 0, interval(0, 100))),
        };
        match variant {
            0 => {
                product.producer.payload = PayloadContract::Structured(StructuredPayloadContract {
                    modality: "time_transform".into(),
                    datatype: "unsupported/v999".into(),
                    schema: "unsupported/v999".into(),
                    observes: "clock".into(),
                    unit: Some("ns".into()),
                });
                product.manifest.producer = product.producer.reference();
                product.manifest_hash = product.manifest.hash();
                observation.output = product.producer.reference();
            }
            1 => observation.output.peer_id = "forged-owner".into(),
            2 => observation.sequence = 1,
            3 => observation.timestamp_ns = 11,
            _ => unreachable!(),
        }
        product
            .buffer()
            .append_shared(Arc::new(Envelope::new(0, 10, observation)))
            .unwrap();
        let called = std::cell::Cell::new(false);
        assert!(matches!(
            resolver.insert_offset_sample(&product, &contract, 0, |_| {
                called.set(true);
                true
            }),
            Err(TemporalError::Invalid(_))
        ));
        assert!(!called.get(), "invalid provenance reached host admission");
    }
}

#[test]
fn one_output_observation_in_multiple_products_is_one_piece_of_evidence() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = TemporalResolver::new(ResolverLimits {
        max_samples: 1,
        ..ResolverLimits::default()
    })
    .unwrap();
    for clock in [&a, &b] {
        resolver
            .register_clock(reference(clock), clock.clone())
            .unwrap();
    }
    let contract = contract(&a, &b);
    let original = offset_product("a", "first-buffer", &contract);
    append(&original, 0, 10, sample(20, 1, interval(0, 100)));
    resolver
        .insert_offset_sample(&original, &contract, 0, |_| true)
        .unwrap();

    let mut other_manifest = original.manifest.clone();
    other_manifest.product_id = "other-retention-of-same-source".into();
    let another = RetainedProduct::imported_buffer(
        other_manifest.clone(),
        other_manifest.hash(),
        original.producer.clone(),
        BufferLimits::entries(4),
        |_| std::mem::size_of::<OffsetSample>(),
    )
    .unwrap();
    append(&another, 0, 10, sample(20, 1, interval(0, 100)));
    resolver
        .insert_offset_sample(&another, &contract, 0, |_| true)
        .unwrap();
    let result = resolver.resolve(&query(&a, &b, 10)).unwrap();
    assert_eq!(result.timestamp_ns, 30);
    assert_eq!(result.path[0].evidence.product, original.reference());

    let corrupted = RetainedProduct::imported_buffer(
        another.manifest.clone(),
        another.manifest_hash.clone(),
        another.producer.clone(),
        BufferLimits::entries(4),
        |_| std::mem::size_of::<OffsetSample>(),
    )
    .unwrap();
    append(&corrupted, 0, 10, sample(25, 1, interval(0, 100)));
    assert!(matches!(
        resolver.insert_offset_sample(&corrupted, &contract, 0, |_| true),
        Err(TemporalError::Invalid(_))
    ));
    assert_eq!(
        resolver.resolve(&query(&a, &b, 10)).unwrap().timestamp_ns,
        30
    );
}

#[test]
fn consistent_alternate_paths_are_deterministic_without_fusing_their_bounds() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut results = Vec::new();
    for reverse_order in [false, true] {
        let mut resolver = resolver(&[&a, &b]);
        let edges = if reverse_order {
            [("z-second", 12), ("a-first", 10)]
        } else {
            [("a-first", 10), ("z-second", 12)]
        };
        for (name, offset) in edges {
            add(&mut resolver, "a", name, &a, &b, offset, 2);
        }
        let result = resolver.resolve(&query(&a, &b, SECOND)).unwrap();
        assert_eq!(result.uncertainty_ns, 2);
        assert_eq!(result.timestamp_ns, SECOND + 10);
        assert_eq!(result.path[0].evidence.product.product_id, "a-first");
        results.push(result);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn all_paths_need_a_common_interval_not_just_overlap_with_the_broadest_path() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    add(&mut resolver, "a", "broad", &a, &b, 0, 100);
    add(&mut resolver, "a", "narrow-before", &a, &b, -10, 1);
    add(&mut resolver, "a", "narrow-after", &a, &b, 10, 1);
    assert!(matches!(
        resolver.resolve(&query(&a, &b, SECOND)),
        Err(TemporalError::Conflict { candidates }) if candidates.len() == 3
    ));
}

#[test]
fn inverse_of_minimum_signed_offset_uses_widened_arithmetic() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let mut resolver = resolver(&[&a, &b]);
    let contract = contract(&a, &b);
    let product = offset_product("a", "minimum-offset", &contract);
    let source_time = (1_u64 << 63) + 1;
    append(
        &product,
        0,
        source_time,
        sample(i64::MIN, 0, interval(source_time, source_time)),
    );
    resolver
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    assert_eq!(
        resolver
            .resolve(&query(&a, &b, source_time))
            .unwrap()
            .timestamp_ns,
        1
    );
    assert_eq!(
        resolver.resolve(&query(&b, &a, 1)).unwrap().timestamp_ns,
        source_time
    );
}

#[test]
fn drift_rounds_up_and_all_error_bound_additions_are_checked() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let contract = contract(&a, &b);
    let product = offset_product("a", "subnanosecond-drift", &contract);
    append(
        &product,
        0,
        0,
        OffsetSample::Available {
            offset_ns: 100,
            uncertainty_ns: Some(0),
            max_drift_ppb: Some(1),
            validity: interval(0, 1),
        },
    );
    let mut temporal = resolver(&[&a, &b]);
    temporal
        .insert_offset_sample(&product, &contract, 0, |_| true)
        .unwrap();
    assert_eq!(
        temporal.resolve(&query(&a, &b, 1)).unwrap().uncertainty_ns,
        1
    );

    for (base, ppb, coverage, input_error) in [
        (0, u64::MAX, interval(0, u64::MAX), 0),
        (u64::MAX, 1, interval(0, 101), 0),
        (u64::MAX, 0, interval(0, 200), 1),
    ] {
        let mut resolver = resolver(&[&a, &b]);
        let product = offset_product("a", "overflowing-error", &contract);
        append(
            &product,
            0,
            100,
            OffsetSample::Available {
                offset_ns: 100,
                uncertainty_ns: Some(base),
                max_drift_ppb: Some(ppb),
                validity: coverage,
            },
        );
        resolver
            .insert_offset_sample(&product, &contract, 0, |_| true)
            .unwrap();
        let mut request = query(&a, &b, 100);
        request.input_uncertainty_ns = input_error;
        assert_issue(
            resolver.resolve(&request),
            ResolutionIssue::ArithmeticOverflow,
        );
    }
}

#[test]
fn returned_coverage_guarantees_the_same_multihop_path_at_both_endpoints() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    let c = monotonic("c", "one");
    let mut resolver = resolver(&[&a, &b, &c]);
    let first_contract = contract(&a, &b);
    let first = offset_product("a", "first", &first_contract);
    append(&first, 0, 15, sample(100, 2, interval(10, 20)));
    resolver
        .insert_offset_sample(&first, &first_contract, 0, |_| true)
        .unwrap();
    let second_contract = contract(&c, &b);
    let second = offset_product("c", "second", &second_contract);
    append(&second, 0, 65, sample(50, 3, interval(58, 72)));
    resolver
        .insert_offset_sample(&second, &second_contract, 0, |_| true)
        .unwrap();
    let result = resolver.resolve(&query(&a, &c, 15)).unwrap();
    assert_eq!(result.timestamp_ns, 65);
    assert_eq!(result.uncertainty_ns, 5);
    assert_eq!(result.source_coverage, interval(13, 17));
    assert!(result.path[1].inverted);
    for timestamp in [
        result.source_coverage.start_ns,
        result.source_coverage.end_ns,
    ] {
        let endpoint = resolver.resolve(&query(&a, &c, timestamp)).unwrap();
        assert_eq!(endpoint.path, result.path);
        assert_eq!(endpoint.uncertainty_ns, result.uncertainty_ns);
    }
    for timestamp in [
        result.source_coverage.start_ns - 1,
        result.source_coverage.end_ns + 1,
    ] {
        assert_issue(
            resolver.resolve(&query(&a, &c, timestamp)),
            ResolutionIssue::OutsideCoverage,
        );
    }
}

#[test]
fn malformed_coverage_unsupported_clock_semantics_and_unknown_fields_are_rejected() {
    let a = monotonic("a", "one");
    let b = monotonic("b", "one");
    for coverage in [interval(20, 10), interval(11, 20)] {
        let mut resolver = resolver(&[&a, &b]);
        let contract = contract(&a, &b);
        let product = offset_product("a", "invalid-coverage", &contract);
        append(&product, 0, 10, sample(0, 0, coverage));
        assert!(matches!(
            resolver.insert_offset_sample(&product, &contract, 0, |_| true),
            Err(TemporalError::Invalid(_))
        ));
    }
    for mutation in 0..3 {
        let mut unsupported = a.clone();
        if let ClockBody::MonotonicClock(meta) = &mut unsupported.body {
            match mutation {
                0 => meta.unit = "ticks".into(),
                1 => meta.epoch = Some("invented epoch".into()),
                2 => meta.monotonic = false,
                _ => unreachable!(),
            }
        }
        let mut resolver = resolver(&[]);
        assert!(matches!(
            resolver.register_clock(reference(&unsupported), unsupported),
            Err(TemporalError::Invalid(_))
        ));
    }
    let mut contract_json = serde_json::to_value(contract(&a, &b)).unwrap();
    contract_json["assume_synced"] = serde_json::json!(true);
    assert!(serde_json::from_value::<OffsetContract>(contract_json).is_err());
    let mut sample_json = serde_json::to_value(sample(0, 0, interval(0, 10))).unwrap();
    sample_json["hold_forever"] = serde_json::json!(true);
    assert!(serde_json::from_value::<OffsetSample>(sample_json).is_err());
}

#[test]
fn identity_query_still_requires_exact_definition_and_honors_input_uncertainty() {
    let a = monotonic("a", "one");
    let empty = resolver(&[]);
    assert!(matches!(
        empty.resolve(&query(&a, &a, 10)),
        Err(TemporalError::MissingClock(_))
    ));
    let resolver = resolver(&[&a]);
    let mut request = query(&a, &a, 10);
    request.input_uncertainty_ns = 3;
    request.max_uncertainty_ns = 3;
    let result = resolver.resolve(&request).unwrap();
    assert_eq!(result.timestamp_ns, 10);
    assert_eq!(result.uncertainty_ns, 3);
    assert!(result.path.is_empty());
    assert_eq!(result.source_coverage, interval(3, u64::MAX - 3));
    request.max_uncertainty_ns = 2;
    assert_issue(
        resolver.resolve(&request),
        ResolutionIssue::UncertaintyExceeded,
    );
}
