use auki_protocol::v1::{
    authority::PeerAuthorizationMode,
    error,
    status::{LocalDomainRole, PathType, StatusError, StatusSnapshot},
};
use serde_json::Value;

const V1_STATUS_VECTORS: &str = include_str!("../vectors/v1_status.json");

fn fixture() -> Value {
    serde_json::from_str(V1_STATUS_VECTORS).expect("valid status fixture")
}

fn input<'a>(fixture: &'a Value, key: &str) -> &'a str {
    fixture["inputs"][key].as_str().expect("input string")
}

fn positive_object<'a>(fixture: &'a Value, key: &str) -> &'a Value {
    &fixture["positive"][key]["object"]
}

fn positive_expected<'a>(fixture: &'a Value, key: &str) -> &'a Value {
    &fixture["positive"][key]["expected"]
}

fn negative_object<'a>(fixture: &'a Value, key: &str) -> &'a Value {
    &fixture["negative"][key]["object"]
}

fn expected_negative<'a>(fixture: &'a Value, key: &str) -> &'a str {
    fixture["negative"][key]["expected"]
        .as_str()
        .expect("negative expected string")
}

#[test]
fn positive_status_vectors_match_implementation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.status.vectors".to_owned())
    );

    let snapshot_object = positive_object(&fixture, "full_snapshot");
    let expected = positive_expected(&fixture, "full_snapshot");
    let snapshot = StatusSnapshot::from_value(snapshot_object.clone()).unwrap();

    assert_eq!(snapshot.value(), snapshot_object);
    assert_eq!(snapshot.generated_at, input(&fixture, "generated_at"));
    assert_eq!(
        snapshot.local_peer.peer_id.as_deref(),
        Some(input(&fixture, "peer_id"))
    );
    assert_eq!(
        snapshot.local_peer.wallet_public_key.as_deref(),
        Some(input(&fixture, "wallet_public_key"))
    );
    assert_eq!(
        snapshot.local_peer.authorization_mode,
        Some(PeerAuthorizationMode::WhitelistedOnly)
    );
    assert_eq!(
        expected["authorization_mode"],
        PeerAuthorizationMode::WhitelistedOnly.as_str()
    );
    assert_eq!(snapshot.local_domains[0].role, Some(LocalDomainRole::Owner));
    assert_eq!(
        expected["local_domain_role"],
        LocalDomainRole::Owner.as_str()
    );
    assert_eq!(
        snapshot.discovery.as_ref().unwrap().advertised_domains,
        vec![input(&fixture, "domain_id")]
    );
    assert_eq!(
        snapshot.remote_peers[0].loaded_offers[0].registry_refs[0].hash,
        expected["loaded_offer_registry_hash"]
    );
    assert_eq!(
        snapshot.active_paths[0].path_type,
        Some(PathType::Subscribe)
    );
    assert_eq!(expected["path_type"], PathType::Subscribe.as_str());
    assert_eq!(snapshot.active_paths[0].last_sequence, Some(7));
    assert_eq!(snapshot.last_failures[0].code, error::OFFER_UNKNOWN_OFFER);
    assert_eq!(expected["last_failure_code"], error::OFFER_UNKNOWN_OFFER);
}

#[test]
fn negative_status_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "unsupported_type"),
        "unsupported_type"
    );
    let unsupported =
        StatusSnapshot::from_value(negative_object(&fixture, "unsupported_type").clone())
            .unwrap_err();
    assert_eq!(
        unsupported,
        StatusError::UnsupportedType {
            actual: "auki.status.v1".to_owned(),
        }
    );

    assert_eq!(
        expected_negative(&fixture, "missing_local_domains"),
        "missing_field"
    );
    let missing =
        StatusSnapshot::from_value(negative_object(&fixture, "missing_local_domains").clone())
            .unwrap_err();
    assert_eq!(
        missing,
        StatusError::MissingField {
            object: "status snapshot",
            field: "local_domains",
        }
    );

    assert_eq!(
        expected_negative(&fixture, "invalid_path_type"),
        "invalid_array_item"
    );
    let invalid_path =
        StatusSnapshot::from_value(negative_object(&fixture, "invalid_path_type").clone())
            .unwrap_err();
    assert!(matches!(
        invalid_path,
        StatusError::InvalidArrayItem {
            field: "active_paths",
            ..
        }
    ));

    assert_eq!(
        expected_negative(&fixture, "invalid_registry_hash"),
        "invalid_array_item"
    );
    let invalid_registry =
        StatusSnapshot::from_value(negative_object(&fixture, "invalid_registry_hash").clone())
            .unwrap_err();
    assert!(matches!(
        invalid_registry,
        StatusError::InvalidArrayItem {
            field: "remote_peers",
            ..
        }
    ));
}
