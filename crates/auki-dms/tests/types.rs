use auki_dms::{LeaseEnvelope, TaskSpec};
use serde_json::json;

#[test]
fn lease_round_trip_and_runner_view_preserve_canonical_fields() {
    let lease: LeaseEnvelope = serde_json::from_value(json!({
        "access_token": "storage",
        "p2p_access_token": "peer",
        "p2p_access_token_expires_at": "2026-09-09T12:00:00Z",
        "task": { "id": uuid::Uuid::nil(), "capability": "/example/task/v1", "priority": -3 }
    }))
    .unwrap();
    let decoded: LeaseEnvelope =
        serde_json::from_slice(&serde_json::to_vec(&lease).unwrap()).unwrap();
    assert_eq!(decoded, lease);
    let visible = lease.without_p2p_credentials();
    assert!(visible.p2p_access_token.is_none());
    assert!(visible.p2p_access_token_expires_at.is_none());
    assert_eq!(visible.access_token, lease.access_token);
    assert_eq!(visible.task, lease.task);
    assert!(lease.p2p_access_token.is_some());
}

#[test]
fn task_spec_priority_allows_negative_values() {
    use uuid::Uuid;

    let spec: TaskSpec = serde_json::from_value(json!({
        "id": Uuid::nil(),
        "capability": "/dummy/v1",
        "capability_filters": {},
        "inputs_cids": [],
        "meta": {},
        "priority": -3
    }))
    .unwrap();

    assert_eq!(spec.priority, Some(-3));
}

#[test]
fn lease_json_without_p2p_fields_remains_compatible() {
    use uuid::Uuid;

    let lease: LeaseEnvelope = serde_json::from_value(json!({
        "access_token": "domain-http-token",
        "task": {
            "id": Uuid::nil(),
            "capability": "/dummy/v1"
        }
    }))
    .unwrap();

    assert_eq!(lease.access_token.as_deref(), Some("domain-http-token"));
    assert!(lease.p2p_access_token.is_none());
    assert!(lease.p2p_access_token_expires_at.is_none());
}
