use std::sync::Arc;

use auki_components::{
    BufferLimits, ComponentRuntime, ComponentSpec, ConfiguredObservableSpec, Exposure,
    GaugePayloadContract, ObservableContract, ObservationAccess, PayloadContract, manifest_hash,
};

fn level_contract() -> ObservableContract {
    ObservableContract {
        name: "level".to_owned(),
        datatype: "float64".to_owned(),
        schema: "test.level/v1".to_owned(),
        access: vec![ObservationAccess::FollowNew],
        exposure: Exposure::Cluster,
    }
}

fn level_payload() -> PayloadContract {
    PayloadContract::Gauge(GaugePayloadContract {
        datatype: "float64".to_owned(),
        schema: "test.level/v1".to_owned(),
        observes: "level".to_owned(),
        unit: "percent".to_owned(),
    })
}

#[test]
fn manifest_hash_uses_the_sdk_canonical_content_hash() {
    let value = serde_json::json!({"z": 2, "a": 1});
    let canonical = auki_jcs::canonicalize(&value);
    assert_eq!(manifest_hash(&value), auki_hash::hash_jcs_bytes(&canonical));
    assert_eq!(manifest_hash(&value).len(), 32);
}

#[test]
fn snapshots_are_consistent_and_revisions_track_visible_changes() {
    let runtime = ComponentRuntime::new("peer-a");
    assert_eq!(runtime.catalog().snapshot().revision, 0);

    let sensor = runtime
        .component(ComponentSpec::new("sensor").observable(level_contract()))
        .unwrap();
    let output = sensor
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            "peer-a.clock",
            level_payload(),
        ))
        .unwrap();
    sensor.expose().unwrap();

    let exposed = runtime.catalog().snapshot();
    assert_eq!(exposed.components.len(), 1);
    assert!(exposed.revision > 0);

    let capture = runtime
        .capture_buffer("level-history", &output, BufferLimits::entries(4), |_| 8)
        .unwrap();
    let registered = runtime.catalog().snapshot();
    assert_eq!(registered.products.len(), 1);
    assert!(registered.revision > exposed.revision);

    output.publish(1, Arc::new(42.0)).unwrap();
    let populated = runtime.catalog().snapshot();
    assert!(populated.revision > registered.revision);
    assert_eq!(
        serde_json::to_value(&populated).unwrap()["products"][0]["state"]["Buffer"]["entries"],
        1
    );

    drop(capture);
}

#[test]
fn capture_metadata_changes_revision_without_changing_product_identity() {
    use auki_components::{CatalogProductEntry, CatalogProductMetadata};
    let runtime = ComponentRuntime::new("peer-a");
    let sensor = runtime
        .component(ComponentSpec::new("sensor").observable(level_contract()))
        .unwrap();
    let output = sensor
        .configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "out",
            "clock",
            level_payload(),
        ))
        .unwrap();
    sensor.expose().unwrap();
    let capture = runtime
        .capture_buffer_with_metadata(
            "history",
            &output,
            BufferLimits::entries(1),
            |_| 8,
            |observation| {
                Ok(Some(CatalogProductMetadata {
                    schema: "test.discovery/v1".into(),
                    source_sequence: observation.sequence,
                    value: serde_json::json!({"value":*observation.payload}),
                }))
            },
        )
        .unwrap();
    let identity = capture.product().reference();
    output.publish(1, Arc::new(1.)).unwrap();
    let first = runtime.catalog().snapshot();
    output.publish(2, Arc::new(2.)).unwrap();
    let second = runtime.catalog().snapshot();
    assert_eq!(second.revision, first.revision + 1);
    assert_eq!(first.products[0].state, second.products[0].state);
    assert_eq!(second.products[0].manifest.reference(), identity);
    let metadata = second.products[0].metadata.as_ref().unwrap();
    assert_eq!(metadata.source_sequence, 1);
    assert_eq!(metadata.value, serde_json::json!({"value":2.}));
    let mut old_json = serde_json::to_value(&second.products[0]).unwrap();
    old_json.as_object_mut().unwrap().remove("metadata");
    let decoded: CatalogProductEntry = serde_json::from_value(old_json).unwrap();
    assert!(decoded.metadata.is_none());
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("metadata")
            .is_none()
    );
}

#[test]
fn invalid_metadata_is_not_retained_or_advertised() {
    use auki_components::{CatalogProductMetadata, MAX_CATALOG_METADATA_BYTES};
    for case in 0..3 {
        let runtime = ComponentRuntime::new("peer-a");
        let sensor = runtime
            .component(ComponentSpec::new("sensor").observable(level_contract()))
            .unwrap();
        let output = sensor
            .configured_observable::<f64>(ConfiguredObservableSpec::new(
                "level",
                "out",
                "clock",
                level_payload(),
            ))
            .unwrap();
        sensor.expose().unwrap();
        let capture = runtime
            .capture_buffer_with_metadata(
                "history",
                &output,
                BufferLimits::entries(1),
                |_| 8,
                move |observation| {
                    Ok(Some(CatalogProductMetadata {
                        schema: if case == 0 {
                            String::new()
                        } else {
                            "test/v1".into()
                        },
                        source_sequence: if case == 1 {
                            observation.sequence + 1
                        } else {
                            observation.sequence
                        },
                        value: if case == 2 {
                            serde_json::json!("x".repeat(MAX_CATALOG_METADATA_BYTES))
                        } else {
                            serde_json::json!({})
                        },
                    }))
                },
            )
            .unwrap();
        let before = runtime.catalog().snapshot();
        output.publish(1, Arc::new(1.)).unwrap();
        assert_eq!(before, runtime.catalog().snapshot());
        assert!(capture.product().latest_existing().unwrap().is_none());
        assert_eq!(capture.errors().len(), 1);
    }
}
