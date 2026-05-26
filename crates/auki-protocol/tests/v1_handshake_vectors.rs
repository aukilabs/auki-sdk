use auki_protocol::v1::{
    authority::{PeerAuthorization, PeerAuthorizationPolicy, ServedDomainAuthority},
    error,
    handshake::{CLUSTER_LIFECYCLE_V1, HandshakeError, PEER_HANDSHAKE_TYPE, PeerHandshake},
    offer::{OFFER_CATALOG_PATH_TYPE, OFFER_CATALOG_PROTOCOL_ID, OFFER_CATALOG_VERSION},
};
use libp2p_identity::PeerId;
use serde_json::{Value, json};
use std::str::FromStr;

const V1_HANDSHAKE_VECTORS: &str = include_str!("../vectors/v1_handshakes.json");

fn fixture() -> Value {
    serde_json::from_str(V1_HANDSHAKE_VECTORS).expect("valid handshake fixture")
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
fn positive_handshake_vectors_match_authority_validation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.handshake.vectors".to_owned())
    );

    let peer_id = PeerId::from_str(input(&fixture, "delegate_peer_id")).expect("delegate peer id");
    let now = input(&fixture, "verification_now");
    let object = positive_object(&fixture, "delegated_serving_peer");
    let expected = positive_expected(&fixture, "delegated_serving_peer");

    let handshake = PeerHandshake::from_value(object.clone()).expect("valid handshake");

    assert_eq!(handshake.value(), object);
    assert_eq!(handshake.value()["type"], PEER_HANDSHAKE_TYPE);
    assert_eq!(
        handshake.supported_lifecycle_versions,
        vec![CLUSTER_LIFECYCLE_V1.to_owned()]
    );
    assert_eq!(handshake.authorization_material.as_ref().unwrap().len(), 1);
    assert_eq!(
        handshake.offer_catalog.as_ref().map(|path| path.value()),
        Some(&json!({
            "catalog_version": OFFER_CATALOG_VERSION,
            "protocol_id": OFFER_CATALOG_PROTOCOL_ID,
            "type": OFFER_CATALOG_PATH_TYPE
        }))
    );

    let authority = handshake
        .validate_authority(&peer_id, PeerAuthorization::Authorized, now)
        .expect("valid authority chain");
    let policy_authority = handshake
        .validate_authority_with_authorization_policy(&peer_id, PeerAuthorizationPolicy::all(), now)
        .expect("valid authority chain from policy");

    assert_eq!(authority.peer.peer_id, peer_id);
    assert_eq!(policy_authority, authority);
    assert_eq!(authority.accepted_served_domains.len(), 1);
    assert_eq!(authority.rejected_declared_domains, vec![]);

    let accepted = &authority.accepted_served_domains[0];
    assert_eq!(
        expected["accepted_served_domains"],
        Value::Array(vec![Value::String(accepted.domain_id.clone())])
    );
    assert_eq!(accepted.domain_id, input(&fixture, "domain_id"));
    assert_eq!(accepted.authority, ServedDomainAuthority::Delegated);
    assert_eq!(expected["authority"], "delegated");
    assert!(accepted.delegation.is_some());
}

#[test]
fn negative_handshake_parse_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "missing_required_lifecycle_version"),
        error::PROTOCOL_UNSUPPORTED_VERSION
    );
    let missing_version = PeerHandshake::from_value(
        negative_object(&fixture, "missing_required_lifecycle_version").clone(),
    )
    .unwrap_err();
    assert_eq!(
        missing_version,
        HandshakeError::MissingRequiredLifecycleVersion
    );
    assert_eq!(
        missing_version.failure_code(),
        error::PROTOCOL_UNSUPPORTED_VERSION
    );

    assert_eq!(
        expected_negative(&fixture, "duplicate_lifecycle_version"),
        error::HANDSHAKE_INVALID_MESSAGE
    );
    let duplicate_version =
        PeerHandshake::from_value(negative_object(&fixture, "duplicate_lifecycle_version").clone())
            .unwrap_err();
    assert_eq!(
        duplicate_version,
        HandshakeError::DuplicateLifecycleVersion {
            version: CLUSTER_LIFECYCLE_V1.to_owned()
        }
    );
    assert_eq!(
        duplicate_version.failure_code(),
        error::HANDSHAKE_INVALID_MESSAGE
    );

    assert_eq!(
        expected_negative(&fixture, "invalid_offer_catalog_path"),
        error::HANDSHAKE_INVALID_MESSAGE
    );
    let invalid_offer_catalog =
        PeerHandshake::from_value(negative_object(&fixture, "invalid_offer_catalog_path").clone())
            .unwrap_err();
    assert!(matches!(
        invalid_offer_catalog,
        HandshakeError::InvalidOfferCatalog(_)
    ));
    assert_eq!(
        invalid_offer_catalog.failure_code(),
        error::HANDSHAKE_INVALID_MESSAGE
    );
}

#[test]
fn negative_handshake_authority_vectors_fail_as_locked() {
    let fixture = fixture();
    let peer_id = PeerId::from_str(input(&fixture, "delegate_peer_id")).expect("delegate peer id");
    let now = input(&fixture, "verification_now");

    assert_eq!(
        expected_negative(&fixture, "wrong_authenticated_peer"),
        error::IDENTITY_PEER_ID_MISMATCH
    );
    let wrong_peer_case = &fixture["negative"]["wrong_authenticated_peer"];
    let wrong_peer = PeerId::from_str(
        wrong_peer_case["authenticated_peer_id"]
            .as_str()
            .expect("authenticated peer id"),
    )
    .expect("wrong authenticated peer id");
    let wrong_peer_handshake =
        PeerHandshake::from_value(negative_object(&fixture, "wrong_authenticated_peer").clone())
            .expect("parse wrong-peer handshake");
    let wrong_peer_error = wrong_peer_handshake
        .validate_authority(&wrong_peer, PeerAuthorization::Authorized, now)
        .unwrap_err();
    assert_eq!(
        wrong_peer_error.failure_code(),
        error::IDENTITY_PEER_ID_MISMATCH
    );

    assert_eq!(
        expected_negative(&fixture, "missing_delegation"),
        error::DOMAIN_MISSING_DELEGATION
    );
    let missing_delegation_handshake =
        PeerHandshake::from_value(negative_object(&fixture, "missing_delegation").clone())
            .expect("parse missing-delegation handshake");
    let authority = missing_delegation_handshake
        .validate_authority(&peer_id, PeerAuthorization::Authorized, now)
        .expect("peer binding remains valid");
    assert_eq!(authority.accepted_served_domains, vec![]);
    assert_eq!(authority.rejected_declared_domains.len(), 1);
    assert_eq!(
        authority.rejected_declared_domains[0].failure_code,
        error::DOMAIN_MISSING_DELEGATION
    );
}
