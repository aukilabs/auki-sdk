use auki_identity::Wallet;
use auki_protocol::v1::{
    domain::{DelegationScope, DomainDeclaration, DomainDelegation, DomainError, derive_domain_id},
    error,
    identity::{PeerBinding, PeerBindingError},
};
use libp2p_identity::PeerId;
use serde_json::Value;
use std::str::FromStr;

const V1_SIGNED_OBJECT_VECTORS: &str = include_str!("../vectors/v1_signed_objects.json");

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn bytes_from_hex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "hex string must have an even length");
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex byte"))
        .collect()
}

fn fixture() -> Value {
    serde_json::from_str(V1_SIGNED_OBJECT_VECTORS).expect("valid signed-object fixture")
}

fn input<'a>(fixture: &'a Value, key: &str) -> &'a str {
    fixture["inputs"][key].as_str().expect("input string")
}

fn positive_object<'a>(fixture: &'a Value, key: &str) -> &'a Value {
    &fixture["positive"][key]["object"]
}

fn positive_signed_hex<'a>(fixture: &'a Value, key: &str) -> &'a str {
    fixture["positive"][key]["signed_canonical_hex"]
        .as_str()
        .expect("signed_canonical_hex string")
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
fn positive_signed_object_vectors_match_implementation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.signed_object.vectors".to_owned())
    );

    let owner_wallet = Wallet::from_seed(bytes_from_hex(input(
        &fixture,
        "domain_owner_wallet_seed_hex",
    )))
    .expect("owner wallet seed");
    let delegate_wallet =
        Wallet::from_seed(bytes_from_hex(input(&fixture, "delegate_wallet_seed_hex")))
            .expect("delegate wallet seed");
    let peer_id = PeerId::from_str(input(&fixture, "delegate_peer_id")).expect("delegate peer id");
    let issued_at = input(&fixture, "peer_binding_issued_at");
    let valid_from = input(&fixture, "delegation_valid_from");
    let expires_at = input(&fixture, "delegation_expires_at");
    let now = input(&fixture, "verification_now");
    let nonce: [u8; 16] = bytes_from_hex(input(&fixture, "domain_nonce_hex"))
        .try_into()
        .expect("16-byte nonce");
    let domain_id = derive_domain_id(&owner_wallet.public_key(), &nonce);

    let peer_binding =
        PeerBinding::create(&delegate_wallet, &peer_id, issued_at, Some("delegate-peer")).unwrap();
    assert_eq!(
        peer_binding.value(),
        positive_object(&fixture, "peer_binding")
    );
    assert_eq!(
        hex(&peer_binding.signed_bytes().unwrap()),
        positive_signed_hex(&fixture, "peer_binding")
    );
    let verified_peer = PeerBinding::from_value(positive_object(&fixture, "peer_binding").clone())
        .unwrap()
        .verify_for_peer_id(&peer_id)
        .unwrap();
    assert_eq!(
        verified_peer.wallet_public_key,
        delegate_wallet.public_key()
    );

    let domain_declaration =
        DomainDeclaration::create(&owner_wallet, &nonce, Some("warehouse-main")).unwrap();
    assert_eq!(
        domain_declaration.value(),
        positive_object(&fixture, "domain_declaration")
    );
    assert_eq!(
        hex(&domain_declaration.signed_bytes().unwrap()),
        positive_signed_hex(&fixture, "domain_declaration")
    );
    let verified_declaration =
        DomainDeclaration::from_value(positive_object(&fixture, "domain_declaration").clone())
            .unwrap()
            .verify()
            .unwrap();
    assert_eq!(verified_declaration.domain_id, domain_id);
    assert_eq!(
        verified_declaration.domain_owner_public_key,
        owner_wallet.public_key()
    );

    let domain_delegation = DomainDelegation::create(
        &owner_wallet,
        &domain_id,
        &delegate_wallet.public_key(),
        &peer_id,
        &[DelegationScope::Serve, DelegationScope::Advertise],
        valid_from,
        expires_at,
        Some("delegate-peer"),
    )
    .unwrap();
    assert_eq!(
        domain_delegation.value(),
        positive_object(&fixture, "domain_delegation")
    );
    assert_eq!(
        hex(&domain_delegation.signed_bytes().unwrap()),
        positive_signed_hex(&fixture, "domain_delegation")
    );
    DomainDelegation::from_value(positive_object(&fixture, "domain_delegation").clone())
        .unwrap()
        .verify_for_authority(
            &domain_id,
            &owner_wallet.public_key(),
            &delegate_wallet.public_key(),
            &peer_id,
            DelegationScope::Serve,
            now,
        )
        .unwrap();
}

#[test]
fn negative_signed_object_vectors_fail_as_locked() {
    let fixture = fixture();
    let owner_wallet = Wallet::from_seed(bytes_from_hex(input(
        &fixture,
        "domain_owner_wallet_seed_hex",
    )))
    .expect("owner wallet seed");
    let delegate_wallet =
        Wallet::from_seed(bytes_from_hex(input(&fixture, "delegate_wallet_seed_hex")))
            .expect("delegate wallet seed");
    let peer_id = PeerId::from_str(input(&fixture, "delegate_peer_id")).expect("delegate peer id");
    let other_peer_id = PeerId::from_str(input(&fixture, "other_peer_id")).expect("other peer id");
    let nonce: [u8; 16] = bytes_from_hex(input(&fixture, "domain_nonce_hex"))
        .try_into()
        .expect("16-byte nonce");
    let domain_id = derive_domain_id(&owner_wallet.public_key(), &nonce);

    assert_eq!(
        expected_negative(&fixture, "peer_binding_bad_signature"),
        error::IDENTITY_INVALID_SIGNATURE
    );
    let bad_signature =
        PeerBinding::from_value(negative_object(&fixture, "peer_binding_bad_signature").clone())
            .unwrap();
    assert_eq!(
        bad_signature.verify_for_peer_id(&peer_id),
        Err(PeerBindingError::InvalidSignature)
    );

    assert_eq!(
        expected_negative(&fixture, "peer_binding_wrong_authenticated_peer"),
        error::IDENTITY_PEER_ID_MISMATCH
    );
    let wrong_authenticated_peer = PeerBinding::from_value(
        negative_object(&fixture, "peer_binding_wrong_authenticated_peer").clone(),
    )
    .unwrap();
    assert!(matches!(
        wrong_authenticated_peer.verify_for_peer_id(&other_peer_id),
        Err(PeerBindingError::PeerIdMismatch { .. })
    ));

    assert_eq!(
        expected_negative(&fixture, "domain_declaration_domain_id_mismatch"),
        error::DOMAIN_ID_MISMATCH
    );
    assert!(matches!(
        DomainDeclaration::from_value(
            negative_object(&fixture, "domain_declaration_domain_id_mismatch").clone()
        ),
        Err(DomainError::DomainIdMismatch { .. })
    ));

    assert_eq!(
        expected_negative(&fixture, "domain_delegation_unsorted_scopes"),
        error::DOMAIN_INVALID_DELEGATION
    );
    assert!(matches!(
        DomainDelegation::from_value(
            negative_object(&fixture, "domain_delegation_unsorted_scopes").clone()
        ),
        Err(DomainError::ScopesNotSorted { .. })
    ));

    assert_eq!(
        expected_negative(&fixture, "domain_delegation_expired"),
        error::DOMAIN_EXPIRED_DELEGATION
    );
    let expired = DomainDelegation::from_value(
        negative_object(&fixture, "domain_delegation_expired").clone(),
    )
    .unwrap();
    assert!(matches!(
        expired.verify_for_authority(
            &domain_id,
            &owner_wallet.public_key(),
            &delegate_wallet.public_key(),
            &peer_id,
            DelegationScope::Serve,
            fixture["negative"]["domain_delegation_expired"]["verification_now"]
                .as_str()
                .unwrap(),
        ),
        Err(DomainError::DelegationExpired { .. })
    ));
}
