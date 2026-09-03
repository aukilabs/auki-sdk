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
