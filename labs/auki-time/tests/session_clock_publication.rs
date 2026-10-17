#![cfg(feature = "components")]

use std::cell::Cell;
use std::sync::Arc;

use auki_components::{
    BufferLimits, ComponentRuntime, Envelope, Observation, PayloadContract, ProductState,
    RetainedProduct,
};
use auki_registry::{ClockBody, ClockMeta, Scope};
use auki_time::SessionClock;
use auki_time::components::{
    ANCHOR_SCHEMA, CONTRACT_SCHEMA, SessionClockError, SessionClockPublisher, SessionUtcAnchor,
    SessionUtcContract, UtcUnavailable, read_session_anchor,
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

fn imported(source: &RetainedProduct<SessionUtcAnchor>) -> RetainedProduct<SessionUtcAnchor> {
    RetainedProduct::imported_buffer(
        source.manifest.clone(),
        source.manifest_hash.clone(),
        source.producer.clone(),
        BufferLimits::entries(1),
        |_| std::mem::size_of::<SessionUtcAnchor>(),
    )
    .unwrap()
}

fn append(product: &RetainedProduct<SessionUtcAnchor>, observation: Observation<SessionUtcAnchor>) {
    product
        .buffer()
        .append_shared(Arc::new(Envelope::new(
            observation.sequence,
            observation.timestamp_ns,
            observation,
        )))
        .unwrap();
}

#[test]
fn catalog_exposes_exact_definitions_and_retained_anchor_for_any_peer() {
    for peer in ["Steve", "Bob"] {
        let runtime = ComponentRuntime::new(peer);
        let session = SessionClock::new(peer, "session-1", "capture");
        let original_definition = session.registry_entry();
        let mut publisher = SessionClockPublisher::new(&runtime, &session).unwrap();
        let before = session.now_ns();
        let observation = publisher
            .publish_startup_anchor_with_utc(|| Ok(UTC_NS))
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
        let SessionUtcAnchor::Available { offset_ns, .. } = *observation.payload else {
            panic!("valid UTC read should be available");
        };
        assert_eq!(observation.timestamp_ns as i64 + offset_ns, UTC_NS);

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
            if payload.datatype == ANCHOR_SCHEMA && payload.schema == ANCHOR_SCHEMA)
        );

        let received = imported(&product);
        append(&received, observation.clone());
        let retained = read_session_anchor(&received, &discovered).unwrap();
        assert_eq!(retained.output, observation.output);
        assert_eq!(retained.timestamp_ns, observation.timestamp_ns);
        assert_eq!(retained.sequence, observation.sequence);
        assert_eq!(retained.payload, observation.payload);
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
        assert!(read_session_anchor(&a.product(), other.contract()).is_err());
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
fn publication_is_one_shot_and_wall_clock_correction_cannot_replace_anchor() {
    let (runtime, mut publisher) = publisher("Steve", "session-1");
    assert!(matches!(
        read_session_anchor(&publisher.product(), publisher.contract()),
        Err(SessionClockError::NotPublished)
    ));
    let anchor = publisher
        .publish_startup_anchor_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let catalog = runtime.catalog().snapshot();
    let corrected_reader_called = Cell::new(false);
    let result = publisher.publish_startup_anchor_with_utc(|| {
        corrected_reader_called.set(true);
        Ok(UTC_NS + 30_000_000_000)
    });
    assert!(matches!(result, Err(SessionClockError::AlreadyPublished)));
    assert!(!corrected_reader_called.get());
    let retained = read_session_anchor(&publisher.product(), publisher.contract()).unwrap();
    assert_eq!(retained.sequence, 0);
    assert_eq!(retained.timestamp_ns, anchor.timestamp_ns);
    assert_eq!(retained.payload, anchor.payload);
    assert_eq!(runtime.catalog().snapshot(), catalog);
    // A second helper for the same session cannot replace the Catalog Product.
    let duplicate = SessionClock::new("Steve", "session-1", "capture");
    assert!(SessionClockPublisher::new(&runtime, &duplicate).is_err());
    assert_eq!(runtime.catalog().snapshot(), catalog);
}

#[test]
fn anchor_profile_round_trips_without_accepting_legacy_or_unknown_fields() {
    for anchor in [
        SessionUtcAnchor::Available {
            offset_ns: 123,
            uncertainty_ns: 4,
        },
        SessionUtcAnchor::Unavailable {
            reason: UtcUnavailable::ReadFailed,
        },
    ] {
        let value = serde_json::to_value(&anchor).unwrap();
        assert_eq!(
            serde_json::from_value::<SessionUtcAnchor>(value.clone()).unwrap(),
            anchor
        );
        let mut unknown = value;
        unknown["future_field"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SessionUtcAnchor>(unknown).is_err());
    }
    assert!(
        serde_json::from_value::<SessionUtcAnchor>(serde_json::json!({
            "offset_ns": 123, "uncertainty_ns": 4
        }))
        .is_err()
    );
}

#[test]
fn unavailable_utc_is_retained_explicitly_and_cannot_be_retried() {
    for reason in [
        UtcUnavailable::ReadFailed,
        UtcUnavailable::OutOfRange,
        UtcUnavailable::InvalidSample,
    ] {
        let (runtime, mut publisher) = publisher("Bob", "session-1");
        let anchor = publisher
            .publish_startup_anchor_with_utc(|| Err(reason))
            .unwrap();
        assert_eq!(*anchor.payload, SessionUtcAnchor::Unavailable { reason });
        assert_eq!(anchor.sequence, 0);
        assert!(runtime.catalog().snapshot().products[0].metadata.is_some());
        assert!(matches!(
            publisher.publish_startup_anchor_with_utc(|| panic!("must not retry UTC read")),
            Err(SessionClockError::AlreadyPublished)
        ));
        let retained = read_session_anchor(&publisher.product(), publisher.contract()).unwrap();
        assert_eq!(retained.payload, anchor.payload);
    }
}

#[test]
fn imported_replacement_or_mismatched_observation_provenance_is_rejected() {
    let (_runtime, mut publisher) = publisher("Steve", "session-1");
    let anchor = publisher
        .publish_startup_anchor_with_utc(|| Ok(UTC_NS))
        .unwrap();
    let replacement = imported(&publisher.product());
    let mut changed = anchor.clone();
    changed.sequence = 1;
    append(&replacement, changed);
    assert!(matches!(
        read_session_anchor(&replacement, publisher.contract()),
        Err(SessionClockError::AlreadyPublished)
    ));

    let mismatched = imported(&publisher.product());
    let mut changed = anchor.clone();
    changed.output.peer_id = "Bob".into();
    append(&mismatched, changed);
    assert!(matches!(
        read_session_anchor(&mismatched, publisher.contract()),
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
                read_session_anchor(&changed, publisher.contract()),
                Err(SessionClockError::Invalid(_))
            ),
            "accepted {mutation}"
        );
    }
}
