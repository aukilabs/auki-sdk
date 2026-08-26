use std::time::{Duration, SystemTime, UNIX_EPOCH};

use auki_p2p::{
    ApplicationProtocol, DdsTokenVerifier, DdsVerificationKeys, DomainAuthority, Error, ExactRoute,
    Identity, Node, NodeObservationEvent, NodeObservationStatus, P2PAccessClaims,
    P2pCredentialError, PeerDisappearanceReason, PeerRole, ProtocolSpec, SessionRequirements,
    SignedApplicationMetadata, SignedP2pCredential, DOMAIN_SERVER_MAX_DOMAINS, P2P_TOKEN_AUDIENCE,
    P2P_TOKEN_CLOCK_SKEW, P2P_TOKEN_ISSUER, P2P_TOKEN_MAX_APPLICATION_NAME_BYTES,
    P2P_TOKEN_MAX_APPLICATION_VERSION_BYTES, P2P_TOKEN_MAX_PEER_TYPE_BYTES, P2P_TOKEN_MAX_SCOPES,
    P2P_TOKEN_MAX_SCOPE_BYTES, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL, P2P_TOKEN_TYPE,
};
use futures::io::{AsyncReadExt, AsyncWriteExt};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use libp2p::{identity::PublicKey, Multiaddr};
use serde::Serialize;
use tokio::sync::broadcast::error::TryRecvError;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const ROTATED_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgwRbuxaM6rEI3vYEl
vRmIEsc1QtC3uPMWvXo1xXt+CcOhRANCAAQDFwBFAujMsiq78IWbq5vz0QSWEdc7
7h5NE8sDwgD6Js22t9Ztq84hhkS3Aad4m9FOi8evk5QYW7ef+Bc2oZsr
-----END PRIVATE KEY-----"#;

const ROTATED_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

const TEST_PROTOCOL: &str = "/auki-p2p/test/0.1.0";
type ClaimsMutation = Box<dyn Fn(&mut P2PAccessClaims)>;

#[test]
fn identity_public_key_is_bound_to_its_peer_id() {
    let identity = Identity::generate();
    let public_key = PublicKey::try_decode_protobuf(&identity.public_key_protobuf()).unwrap();
    assert_eq!(public_key.to_peer_id(), identity.peer_id());
}

#[tokio::test]
async fn domain_authority_is_the_redacting_key_credential_and_challenge_boundary() {
    let identity = Identity::generate();
    let expected_peer_id = identity.peer_id();
    let node = Node::start(identity, verifier(), std::iter::empty::<Multiaddr>()).unwrap();
    let authority: DomainAuthority = node.authority();
    assert_eq!(authority.peer_id(), expected_peer_id);

    let public_key = PublicKey::try_decode_protobuf(&authority.peer_public_key_protobuf()).unwrap();
    assert_eq!(public_key.to_peer_id(), expected_peer_id);
    let challenge = b"host-owned DDS challenge";
    let signature = authority.sign_peer_challenge(challenge).unwrap();
    assert!(public_key.verify(challenge, &signature));
    assert!(!public_key.verify(b"different challenge", &signature));

    authority
        .install_verification_keys(DdsVerificationKeys::new(
            1,
            ROTATED_DDS_PUBLIC_KEY.to_vec(),
            Some(TEST_DDS_PUBLIC_KEY.to_vec()),
        ))
        .await
        .unwrap();
    let domain_id = Uuid::new_v4().to_string();
    let claims = claims(
        expected_peer_id,
        PeerRole::Compute,
        vec![domain_id.clone()],
        unix_time(),
    );
    let compact = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(ROTATED_DDS_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    authority
        .install_credential(SignedP2pCredential::new(compact.clone()).unwrap())
        .await
        .unwrap();
    assert_eq!(authority.current_claims().await.unwrap(), claims);

    assert!(matches!(
        authority
            .install_verification_keys(DdsVerificationKeys::new(
                0,
                TEST_DDS_PUBLIC_KEY.to_vec(),
                None,
            ))
            .await,
        Err(Error::StaleVerificationKeyGeneration {
            current: 1,
            proposed: 0,
        })
    ));
    assert_eq!(
        authority
            .require(Uuid::parse_str(&domain_id).unwrap())
            .await
            .unwrap(),
        claims
    );

    let debug = format!("{authority:?}");
    assert!(debug.contains(&expected_peer_id.to_string()));
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(&compact));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn wait_for_listeners_is_immediately_ready_without_configured_listeners() {
    let node = Node::start(
        Identity::generate(),
        verifier(),
        std::iter::empty::<Multiaddr>(),
    )
    .unwrap();

    let addresses = tokio::time::timeout(Duration::from_millis(100), node.wait_for_listeners())
        .await
        .expect("zero-listener readiness unexpectedly blocked")
        .unwrap();

    assert!(addresses.is_empty());
    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_listeners_waits_for_every_configured_listener() {
    let node = Node::start(
        Identity::generate(),
        verifier(),
        [
            "/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().unwrap(),
            "/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().unwrap(),
        ],
    )
    .unwrap();

    let addresses = tokio::time::timeout(Duration::from_secs(5), node.wait_for_listeners())
        .await
        .expect("configured listeners did not become ready")
        .unwrap();

    assert_eq!(addresses.len(), 2);
    assert_ne!(addresses[0], addresses[1]);
    assert!(addresses
        .iter()
        .all(|address| address.to_string().starts_with("/ip4/127.0.0.1/tcp/")));
    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_listeners_reports_an_occupied_tcp_port() {
    let occupied = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let requested = format!("/ip4/127.0.0.1/tcp/{port}")
        .parse::<Multiaddr>()
        .unwrap();
    let error = match Node::start(Identity::generate(), verifier(), [requested.clone()]) {
        Ok(node) => tokio::time::timeout(Duration::from_secs(5), node.wait_for_listeners())
            .await
            .expect("occupied listener did not resolve to a deterministic failure")
            .unwrap_err(),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        Error::Listen { address, .. } if address == requested.to_string()
    ));
}

#[test]
fn verifier_enforces_the_exact_dds_claim_profile() {
    let verifier = verifier();
    let identity = Identity::generate();
    let domain_id = Uuid::new_v4().to_string();
    let now = unix_time();

    for role in [PeerRole::Robot, PeerRole::Compute, PeerRole::DomainServer] {
        let single_domain = claims(identity.peer_id(), role, vec![domain_id.clone()], now);
        let verified = verifier.verify(&sign(&single_domain)).unwrap();
        assert_eq!(verified.peer_type.as_deref(), Some(role.as_str()));

        let multi_domain = claims(
            identity.peer_id(),
            role,
            vec![domain_id.clone(), Uuid::new_v4().to_string()],
            now,
        );
        assert_eq!(
            verifier
                .verify(&sign(&multi_domain))
                .unwrap()
                .domain_ids
                .len(),
            2
        );
    }

    let mut cases: Vec<(&str, ClaimsMutation)> = vec![
        (
            "wrong type",
            Box::new(|claims| claims.token_type = "other".into()),
        ),
        ("wrong issuer", Box::new(|claims| claims.iss = "api".into())),
        ("missing audience", Box::new(|claims| claims.aud.clear())),
        (
            "extra audience",
            Box::new(|claims| claims.aud.push("other".into())),
        ),
        (
            "invalid subject",
            Box::new(|claims| claims.sub = "not-a-uuid".into()),
        ),
        (
            "invalid peer id",
            Box::new(|claims| claims.peer_id = "not-a-peer".into()),
        ),
        (
            "missing domain",
            Box::new(|claims| claims.domain_ids.clear()),
        ),
        (
            "invalid domain",
            Box::new(|claims| claims.domain_ids = vec!["not-a-uuid".into()]),
        ),
        (
            "duplicate domain",
            Box::new(|claims| {
                claims.peer_type = Some(PeerRole::DomainServer.to_string());
                claims.domain_ids.push(claims.domain_ids[0].clone());
            }),
        ),
        (
            "noncanonical subject",
            Box::new(|claims| claims.sub = "550E8400-E29B-41D4-A716-446655440000".into()),
        ),
        (
            "noncanonical domain",
            Box::new(|claims| {
                claims.domain_ids = vec!["550E8400-E29B-41D4-A716-446655440000".into()]
            }),
        ),
        (
            "unbounded peer type",
            Box::new(|claims| {
                claims.peer_type = Some("x".repeat(P2P_TOKEN_MAX_PEER_TYPE_BYTES + 1))
            }),
        ),
        (
            "non-visible peer type",
            Box::new(|claims| claims.peer_type = Some("native app".into())),
        ),
        (
            "too many scopes",
            Box::new(|claims| {
                claims.scopes = (0..=P2P_TOKEN_MAX_SCOPES)
                    .map(|index| format!("scope:{index}"))
                    .collect()
            }),
        ),
        (
            "unbounded scope",
            Box::new(|claims| claims.scopes = vec!["x".repeat(P2P_TOKEN_MAX_SCOPE_BYTES + 1)]),
        ),
        (
            "duplicate scope",
            Box::new(|claims| claims.scopes = vec!["custom:r".into(), "custom:r".into()]),
        ),
        (
            "short ttl",
            Box::new(|claims| claims.exp = claims.iat + P2P_TOKEN_TTL.as_secs() - 1),
        ),
        (
            "long ttl",
            Box::new(|claims| claims.exp = claims.iat + P2P_TOKEN_TTL.as_secs() + 1),
        ),
    ];

    for (name, mutate) in cases.drain(..) {
        let mut invalid = claims(
            identity.peer_id(),
            PeerRole::Robot,
            vec![domain_id.clone()],
            now,
        );
        mutate(&mut invalid);
        assert!(verifier.verify(&sign(&invalid)).is_err(), "accepted {name}");
    }

    let too_many_domains = (0..=DOMAIN_SERVER_MAX_DOMAINS)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    let too_many_claims = claims(
        identity.peer_id(),
        PeerRole::DomainServer,
        too_many_domains,
        now,
    );
    assert!(verifier.verify(&sign(&too_many_claims)).is_err());

    let maximum_domains = (0..DOMAIN_SERVER_MAX_DOMAINS)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    let maximum_claims = claims(
        identity.peer_id(),
        PeerRole::DomainServer,
        maximum_domains,
        now,
    );
    assert!(verifier.verify(&sign(&maximum_claims)).is_ok());

    let valid_claims = claims(
        identity.peer_id(),
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
        now,
    );
    let mut unknown_role = serde_json::to_value(&valid_claims).unwrap();
    unknown_role["peer_type"] = serde_json::json!("native_app");
    unknown_role["scopes"] = serde_json::json!(["custom:r", "feature:dataset"]);
    assert!(verifier.verify(&sign(&unknown_role)).is_ok());

    let mut missing_diagnostics = serde_json::to_value(&valid_claims).unwrap();
    missing_diagnostics
        .as_object_mut()
        .unwrap()
        .remove("peer_type");
    missing_diagnostics
        .as_object_mut()
        .unwrap()
        .remove("scopes");
    assert!(verifier.verify(&sign(&missing_diagnostics)).is_ok());

    let mut application_claims = valid_claims.clone();
    application_claims.application = Some(SignedApplicationMetadata {
        name: "future-client!".into(),
        version: "unknown/version?".into(),
    });
    assert_eq!(
        verifier
            .verify(&sign(&application_claims))
            .unwrap()
            .application,
        application_claims.application
    );

    let application_base = serde_json::to_value(&valid_claims).unwrap();
    for (name, application) in [
        ("missing name", serde_json::json!({"version": "1"})),
        ("missing version", serde_json::json!({"name": "client"})),
        (
            "empty name",
            serde_json::json!({"name": "", "version": "1"}),
        ),
        (
            "empty version",
            serde_json::json!({"name": "client", "version": ""}),
        ),
        (
            "control in name",
            serde_json::json!({"name": "client\n", "version": "1"}),
        ),
        (
            "control in version",
            serde_json::json!({"name": "client", "version": "1\t0"}),
        ),
        (
            "oversized name",
            serde_json::json!({
                "name": "x".repeat(P2P_TOKEN_MAX_APPLICATION_NAME_BYTES + 1),
                "version": "1"
            }),
        ),
        (
            "oversized version",
            serde_json::json!({
                "name": "client",
                "version": "1".repeat(P2P_TOKEN_MAX_APPLICATION_VERSION_BYTES + 1)
            }),
        ),
        (
            "extra nested field",
            serde_json::json!({"name": "client", "version": "1", "role": "admin"}),
        ),
    ] {
        let mut invalid = application_base.clone();
        invalid["application"] = application;
        assert!(verifier.verify(&sign(&invalid)).is_err(), "accepted {name}");
    }
    let mut unknown_outer_field = application_base;
    unknown_outer_field["application_name"] = serde_json::json!("client");
    assert!(verifier.verify(&sign(&unknown_outer_field)).is_err());

    let mut missing_issued_at = serde_json::to_value(&valid_claims).unwrap();
    missing_issued_at.as_object_mut().unwrap().remove("iat");
    assert!(verifier.verify(&sign(&missing_issued_at)).is_err());

    let mut missing_expiration = serde_json::to_value(&valid_claims).unwrap();
    missing_expiration.as_object_mut().unwrap().remove("exp");
    assert!(verifier.verify(&sign(&missing_expiration)).is_err());

    let expired = claims(
        identity.peer_id(),
        PeerRole::Robot,
        vec![domain_id],
        now - P2P_TOKEN_TTL.as_secs() - 1,
    );
    assert!(verifier.verify(&sign(&expired)).is_err());

    let literally_expired = claims(
        identity.peer_id(),
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
        now - P2P_TOKEN_TTL.as_secs(),
    );
    assert!(verifier.verify(&sign(&literally_expired)).is_err());
    let within_future_skew = claims(
        identity.peer_id(),
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
        now + P2P_TOKEN_CLOCK_SKEW.as_secs(),
    );
    assert!(verifier.verify(&sign(&within_future_skew)).is_ok());
    let beyond_future_skew = claims(
        identity.peer_id(),
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
        now + P2P_TOKEN_CLOCK_SKEW.as_secs() + 1,
    );
    assert!(verifier.verify(&sign(&beyond_future_skew)).is_err());
    let mut nbf_within_skew = valid_claims.clone();
    nbf_within_skew.nbf = Some(now + P2P_TOKEN_CLOCK_SKEW.as_secs());
    assert!(verifier.verify(&sign(&nbf_within_skew)).is_ok());
    let mut nbf_beyond_skew = valid_claims.clone();
    nbf_beyond_skew.nbf = Some(now + P2P_TOKEN_CLOCK_SKEW.as_secs() + 1);
    assert!(verifier.verify(&sign(&nbf_beyond_skew)).is_err());

    let mut bad_signature = sign(&valid_claims).into_bytes();
    let last = bad_signature.last_mut().unwrap();
    *last = if *last == b'A' { b'B' } else { b'A' };
    assert!(verifier
        .verify(&String::from_utf8(bad_signature).unwrap())
        .is_err());

    let wrong_algorithm = encode(
        &Header::new(Algorithm::HS256),
        &valid_claims,
        &EncodingKey::from_secret(b"not-an-es256-key"),
    )
    .unwrap();
    assert!(verifier.verify(&wrong_algorithm).is_err());
}

#[tokio::test]
async fn verifier_rotates_with_one_bounded_previous_key() {
    let verifier = DdsTokenVerifier::from_keys(DdsVerificationKeys::new(
        7,
        TEST_DDS_PUBLIC_KEY.to_vec(),
        None,
    ))
    .unwrap();
    let identity = Identity::generate();
    let peer_id = identity.peer_id();
    let node = Node::start(identity, verifier.clone(), std::iter::empty::<Multiaddr>()).unwrap();
    let authority = node.authority();
    let claims = claims(
        peer_id,
        PeerRole::Robot,
        vec![Uuid::new_v4().to_string()],
        unix_time(),
    );
    let old_token = sign(&claims);
    assert!(verifier.verify(&old_token).is_ok());

    authority
        .install_verification_keys(DdsVerificationKeys::new(
            8,
            ROTATED_DDS_PUBLIC_KEY.to_vec(),
            Some(TEST_DDS_PUBLIC_KEY.to_vec()),
        ))
        .await
        .unwrap();
    let new_token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(ROTATED_DDS_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    assert!(verifier.verify(&old_token).is_ok());
    assert!(verifier.verify(&new_token).is_ok());

    assert!(matches!(
        authority
            .install_verification_keys(DdsVerificationKeys::new(
                7,
                TEST_DDS_PUBLIC_KEY.to_vec(),
                None,
            ))
            .await,
        Err(Error::StaleVerificationKeyGeneration { .. })
    ));
    assert!(matches!(
        authority
            .install_verification_keys(DdsVerificationKeys::new(
                8,
                TEST_DDS_PUBLIC_KEY.to_vec(),
                None,
            ))
            .await,
        Err(Error::VerificationKeyGenerationConflict(8))
    ));
    assert!(matches!(
        authority
            .install_verification_keys(DdsVerificationKeys::new(
                9,
                ROTATED_DDS_PUBLIC_KEY.to_vec(),
                None,
            ))
            .await,
        Err(Error::VerificationKeyOverlapActive)
    ));
    node.shutdown().await.unwrap();
}

#[test]
fn application_protocol_accepts_sdk_and_posemesh_namespaces_only() {
    for valid in [
        "/auki/auth/1/resources/0.2.0",
        "/auki/auth/1/message/0.1.0",
        "/auki-p2p/dataset/0",
    ] {
        ApplicationProtocol::new(valid).unwrap();
    }
    for invalid in [
        "/auki/auth/2/resources/0.2.0",
        "/auki/auth/1/resources/latest",
        "/auki/resources/0.2.0",
        "/auki-p2p/dataset/latest",
        "/auki-p2p/nested/dataset/0",
        "/auki-p2p/Dataset/1",
        "/auki-p2p/data\nset/1",
        "/auki-p2p/dataset/1..0",
        "/auki-p2p/dataset/1-",
    ] {
        assert!(
            ApplicationProtocol::new(invalid).is_err(),
            "accepted {invalid}"
        );
    }
    assert!(ApplicationProtocol::new(format!("/auki-p2p/{}/1", "a".repeat(65))).is_err());
    assert!(ApplicationProtocol::new(format!("/auki-p2p/dataset/{}", "1".repeat(33))).is_err());
    assert!(ApplicationProtocol::new(format!(
        "/auki/auth/1/{}/{}",
        "a".repeat(64),
        "1".repeat(32)
    ))
    .is_ok());
    assert!(SessionRequirements::new("550E8400-E29B-41D4-A716-446655440000").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_exchange_bytes_only_after_mutual_authentication() {
    let domain_id = Uuid::new_v4().to_string();
    let domain_uuid = Uuid::parse_str(&domain_id).unwrap();
    let robot = listening_node();
    let compute = listening_node();
    let robot_observations = robot.observations();
    let compute_observations = compute.observations();
    let mut robot_events = robot_observations.subscribe();
    let mut compute_events = compute_observations.subscribe();
    install_current_token(&robot, PeerRole::Robot, vec![domain_id.clone()]).await;
    let mut compute_claims = claims(
        compute.peer_id(),
        PeerRole::Compute,
        vec![domain_id.clone()],
        unix_time(),
    );
    compute_claims.peer_type = Some("native_app".into());
    compute_claims.scopes = vec!["custom:r".into(), "feature:dataset".into()];
    compute_claims.application = Some(SignedApplicationMetadata {
        name: "diagnostic-client".into(),
        version: "1.0.0".into(),
    });
    install_signed_token(&compute, sign(&compute_claims)).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = robot
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let robot_peer_id = robot.peer_id();
    let robot_address = listen_address(&robot).await;

    let server = tokio::spawn(async move {
        let mut stream = incoming.accept().await.unwrap().unwrap();
        assert_eq!(
            stream.remote_peer().peer_type.as_deref(),
            Some("native_app")
        );
        assert_eq!(stream.remote_peer().scopes, ["custom:r", "feature:dataset"]);
        assert_eq!(
            stream.remote_peer().application,
            Some(SignedApplicationMetadata {
                name: "diagnostic-client".into(),
                version: "1.0.0".into(),
            })
        );
        let mut request = [0; 4];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        stream.write_all(b"pong").await.unwrap();
        stream.flush().await.unwrap();
    });

    let mut stream = compute
        .open(
            robot_peer_id,
            vec![robot_address],
            protocol,
            SessionRequirements::new(&domain_id)
                .unwrap()
                .with_expected_remote_peer_id(robot_peer_id),
        )
        .await
        .unwrap();
    assert_eq!(stream.remote_peer().peer_id, robot_peer_id);
    stream.write_all(b"ping").await.unwrap();
    stream.flush().await.unwrap();
    let mut response = [0; 4];
    stream.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"pong");
    server.await.unwrap();

    let robot_snapshot = robot_observations.snapshot();
    assert_eq!(robot_snapshot.status(), NodeObservationStatus::Running);
    assert_eq!(robot_snapshot.peers().len(), 1);
    let observed_compute = &robot_snapshot.peers()[0];
    assert_eq!(observed_compute.domain_id(), domain_uuid);
    assert_eq!(observed_compute.peer().peer_id, compute.peer_id());
    assert_eq!(
        observed_compute.peer().peer_type.as_deref(),
        Some("native_app")
    );
    assert_eq!(
        observed_compute.peer().application,
        Some(SignedApplicationMetadata {
            name: "diagnostic-client".into(),
            version: "1.0.0".into(),
        })
    );
    assert!(!observed_compute.connection_ids().is_empty());

    let compute_snapshot = compute_observations.snapshot();
    assert_eq!(compute_snapshot.status(), NodeObservationStatus::Running);
    assert_eq!(compute_snapshot.peers().len(), 1);
    let observed_robot = &compute_snapshot.peers()[0];
    assert_eq!(observed_robot.domain_id(), domain_uuid);
    assert_eq!(observed_robot.peer().peer_id, robot_peer_id);
    assert_eq!(
        observed_robot.peer().peer_type.as_deref(),
        Some(PeerRole::Robot.as_str())
    );
    assert!(!observed_robot.connection_ids().is_empty());

    assert!(matches!(
        robot_events.try_recv(),
        Ok(NodeObservationEvent::Appeared(observation))
            if observation.peer().peer_id == compute.peer_id()
                && observation.domain_id() == domain_uuid
    ));
    assert!(matches!(
        compute_events.try_recv(),
        Ok(NodeObservationEvent::Appeared(observation))
            if observation.peer().peer_id == robot_peer_id
                && observation.domain_id() == domain_uuid
    ));

    robot.shutdown().await.unwrap();
    assert_eq!(
        robot_observations.snapshot().status(),
        NodeObservationStatus::Stopped,
        "shutdown ACK returned before observations reached terminal state"
    );
    assert!(matches!(
        robot_events.try_recv(),
        Ok(NodeObservationEvent::Disappeared {
            reason: PeerDisappearanceReason::NodeStopped,
            ..
        })
    ));
    assert_eq!(
        robot_events.try_recv(),
        Ok(NodeObservationEvent::StatusChanged(
            NodeObservationStatus::Stopped
        ))
    );
    compute.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn final_real_connection_close_removes_both_authenticated_observations() {
    let domain_id = Uuid::new_v4().to_string();
    let robot = listening_node();
    let compute = listening_node();
    install_current_token(&robot, PeerRole::Robot, vec![domain_id.clone()]).await;
    install_current_token(&compute, PeerRole::Compute, vec![domain_id.clone()]).await;
    let robot_observations = robot.observations();
    let compute_observations = compute.observations();
    let mut robot_events = robot_observations.subscribe();
    let mut compute_events = compute_observations.subscribe();
    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = robot
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let robot_peer_id = robot.peer_id();
    let robot_address = listen_address(&robot).await;
    let server = tokio::spawn(async move { incoming.accept().await.unwrap().unwrap() });
    let client_stream = compute
        .open(
            robot_peer_id,
            vec![robot_address],
            protocol,
            SessionRequirements::new(&domain_id)
                .unwrap()
                .with_expected_remote_peer_id(robot_peer_id),
        )
        .await
        .unwrap();
    let server_stream = server.await.unwrap();

    assert!(matches!(
        robot_events.recv().await,
        Ok(NodeObservationEvent::Appeared(_))
    ));
    assert!(matches!(
        compute_events.recv().await,
        Ok(NodeObservationEvent::Appeared(_))
    ));
    drop(client_stream);
    drop(server_stream);
    compute.disconnect(robot_peer_id).await.unwrap();

    for events in [&mut compute_events, &mut robot_events] {
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .expect("final connection close was not observed"),
            Ok(NodeObservationEvent::Disappeared {
                reason: PeerDisappearanceReason::FinalConnectionClosed,
                ..
            })
        ));
    }
    assert!(robot_observations.snapshot().peers().is_empty());
    assert!(compute_observations.snapshot().peers().is_empty());

    robot.shutdown().await.unwrap();
    compute.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_runtime_supervises_multiple_independent_authenticated_protocols() {
    let domain_id = Uuid::new_v4().to_string();
    let robot = listening_node();
    let compute = listening_node();
    install_current_token(&robot, PeerRole::Robot, vec![domain_id.clone()]).await;
    install_current_token(&compute, PeerRole::Compute, vec![domain_id.clone()]).await;
    let robot_peer_id = robot.peer_id();
    let robot_address = listen_address(&robot).await;
    let shutdown = CancellationToken::new();

    let mut servers = Vec::new();
    for (name, response) in [
        ("/auki-p2p/runtime-alpha/1", b"alpha".as_slice()),
        ("/auki-p2p/runtime-beta/1", b"beta".as_slice()),
    ] {
        let spec = ProtocolSpec::new(
            ApplicationProtocol::new(name).unwrap(),
            SessionRequirements::new(&domain_id).unwrap(),
        );
        servers.push(
            robot
                .serve(spec, &shutdown, move |mut stream| async move {
                    let mut request = [0_u8; 1];
                    stream.read_exact(&mut request).await.unwrap();
                    assert_eq!(request, [1]);
                    stream.write_all(response).await.unwrap();
                    stream.flush().await.unwrap();
                })
                .unwrap(),
        );
    }

    for (name, expected) in [
        ("/auki-p2p/runtime-alpha/1", b"alpha".as_slice()),
        ("/auki-p2p/runtime-beta/1", b"beta".as_slice()),
    ] {
        let mut stream = compute
            .open_exact_route(
                robot_peer_id,
                ExactRoute::Direct(robot_address.clone()),
                ApplicationProtocol::new(name).unwrap(),
                SessionRequirements::new(&domain_id)
                    .unwrap()
                    .with_expected_remote_peer_id(robot_peer_id),
            )
            .await
            .unwrap();
        stream.write_all(&[1]).await.unwrap();
        stream.flush().await.unwrap();
        let mut response = vec![0_u8; expected.len()];
        stream.read_exact(&mut response).await.unwrap();
        assert_eq!(response, expected);
        stream.close().await.unwrap();
    }

    for server in servers {
        server.shutdown().await.unwrap();
    }
    robot.shutdown().await.unwrap();
    compute.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_domain_is_rejected_before_application_bytes() {
    let robot_domain = Uuid::new_v4().to_string();
    let compute_domain = Uuid::new_v4().to_string();
    let (robot_error, compute_error) =
        rejected_session(robot_domain.clone(), compute_domain, robot_domain).await;

    assert!(matches!(robot_error, Error::RemoteDomainMismatch(_)));
    assert!(matches!(compute_error, Error::RemoteDomainMismatch(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_local_authority_never_yields_an_application_stream() {
    let domain_id = Uuid::new_v4().to_string();
    let server_node = listening_node();
    let anonymous_client = listening_node();
    install_current_token(&server_node, PeerRole::Robot, vec![domain_id.clone()]).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = server_node
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let server_peer_id = server_node.peer_id();
    let server_task = tokio::spawn(async move { incoming.accept().await.unwrap().unwrap_err() });
    let client_error = anonymous_client
        .open(
            server_peer_id,
            vec![listen_address(&server_node).await],
            protocol,
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(client_error, Error::MissingToken));
    assert!(matches!(server_task.await.unwrap(), Error::InvalidToken(_)));
    server_node.shutdown().await.unwrap();
    anonymous_client.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_expected_remote_peer_never_yields_an_application_stream() {
    let domain_id = Uuid::new_v4().to_string();
    let server_node = listening_node();
    let client_node = listening_node();
    install_current_token(&server_node, PeerRole::Robot, vec![domain_id.clone()]).await;
    install_current_token(&client_node, PeerRole::Compute, vec![domain_id.clone()]).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = server_node
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let server_peer_id = server_node.peer_id();
    let wrong_peer_id = Identity::generate().peer_id();
    let client_error = client_node
        .open(
            server_peer_id,
            vec![listen_address(&server_node).await],
            protocol,
            SessionRequirements::new(&domain_id)
                .unwrap()
                .with_expected_remote_peer_id(wrong_peer_id),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        client_error,
        Error::UnexpectedRemotePeer { expected, actual }
            if expected == wrong_peer_id.to_string() && actual == server_peer_id.to_string()
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), incoming.accept())
            .await
            .is_err(),
        "a wrong expected Peer ID unexpectedly reached the application listener"
    );
    server_node.shutdown().await.unwrap();
    client_node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_installed_token_is_rechecked_at_session_time() {
    let domain_id = Uuid::new_v4().to_string();
    let robot = listening_node();
    let compute = listening_node();
    install_current_token(&robot, PeerRole::Robot, vec![domain_id.clone()]).await;

    let nearly_expired = claims(
        compute.peer_id(),
        PeerRole::Compute,
        vec![domain_id.clone()],
        unix_time() - P2P_TOKEN_TTL.as_secs() + 1,
    );
    install_signed_token(&compute, sign(&nearly_expired)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = robot
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let robot_peer_id = robot.peer_id();
    let server = tokio::spawn(async move { incoming.accept().await.unwrap().unwrap_err() });
    let client_error = compute
        .open(
            robot_peer_id,
            vec![listen_address(&robot).await],
            protocol,
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(client_error, Error::InvalidToken(_)));
    assert!(matches!(server.await.unwrap(), Error::InvalidToken(_)));
    robot.shutdown().await.unwrap();
    compute.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn copied_token_cannot_be_installed_for_another_noise_identity() {
    let domain_id = Uuid::new_v4().to_string();
    let original_identity = Identity::generate();
    let copied_claims = claims(
        original_identity.peer_id(),
        PeerRole::Compute,
        vec![domain_id],
        unix_time(),
    );
    let other_node = listening_node();

    let error = other_node
        .authority()
        .install_credential(SignedP2pCredential::new(sign(&copied_claims)).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        P2pCredentialError::InvalidAccessToken(Error::PeerIdMismatch { .. })
    ));
    other_node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn credential_install_is_monotonic_atomic_and_shared_by_every_store() {
    let node = listening_node();
    let first_store = node.authority();
    let second_store = node.authority();
    let domain_id = Uuid::new_v4().to_string();
    let issued_at = unix_time();

    let older = claims(
        node.peer_id(),
        PeerRole::Compute,
        vec![domain_id.clone()],
        issued_at,
    );
    let newer = claims(
        node.peer_id(),
        PeerRole::Compute,
        vec![domain_id],
        issued_at + 1,
    );
    first_store
        .install_credential(SignedP2pCredential::new(sign(&older)).unwrap())
        .await
        .unwrap();
    second_store
        .install_credential(SignedP2pCredential::new(sign(&newer)).unwrap())
        .await
        .unwrap();

    let stale = first_store
        .install_credential(SignedP2pCredential::new(sign(&older)).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        P2pCredentialError::InvalidAccessToken(Error::StaleCredential {
            current_issued_at,
            proposed_issued_at,
        }) if current_issued_at == issued_at + 1 && proposed_issued_at == issued_at
    ));
    assert_eq!(
        first_store.current_claims().await.unwrap().iat,
        issued_at + 1
    );
    assert_eq!(
        second_store.current_claims().await.unwrap().iat,
        issued_at + 1
    );

    first_store.clear_credential().await;
    assert!(second_store.current_claims().await.is_none());
    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_and_conflicting_credential_updates_preserve_current_authority() {
    let node = listening_node();
    let credentials = node.authority();
    let issued_at = unix_time();
    let mut current = claims(
        node.peer_id(),
        PeerRole::Compute,
        vec![Uuid::new_v4().to_string()],
        issued_at,
    );
    credentials
        .install_credential(SignedP2pCredential::new(sign(&current)).unwrap())
        .await
        .unwrap();

    let newer = claims(
        node.peer_id(),
        PeerRole::Compute,
        current.domain_ids.clone(),
        issued_at + 1,
    );
    let inconsistent_expiration = credentials
        .install_credential_checked(
            SignedP2pCredential::new(sign(&newer)).unwrap(),
            chrono::DateTime::from_timestamp((newer.exp + 1) as i64, 0).unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        inconsistent_expiration,
        P2pCredentialError::InvalidExpiration
    ));
    assert_eq!(credentials.current_claims().await.unwrap().iat, issued_at);

    let mut conflicting = current.clone();
    conflicting.peer_type = Some("native_app".into());
    let conflict = credentials
        .install_credential(SignedP2pCredential::new(sign(&conflicting)).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        P2pCredentialError::InvalidAccessToken(Error::CredentialIssuedAtConflict(value))
            if value == issued_at
    ));

    current.peer_id = Identity::generate().peer_id().to_string();
    current.iat += 1;
    current.exp += 1;
    let mismatched = credentials
        .install_credential(SignedP2pCredential::new(sign(&current)).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        mismatched,
        P2pCredentialError::InvalidAccessToken(Error::PeerIdMismatch { .. })
    ));
    assert_eq!(credentials.current_claims().await.unwrap().iat, issued_at);
    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_domain_credential_rejection_preserves_prior_authority_atomically() {
    let node = listening_node();
    let authority = node.authority();
    let required_domain_id = Uuid::new_v4();
    let wrong_domain_id = Uuid::new_v4();
    let issued_at = unix_time();
    let current = claims(
        node.peer_id(),
        PeerRole::Compute,
        vec![required_domain_id.to_string()],
        issued_at,
    );
    authority
        .install_credential_for_domain(
            SignedP2pCredential::new(sign(&current)).unwrap(),
            required_domain_id,
        )
        .await
        .unwrap();

    let wrong_domain = claims(
        node.peer_id(),
        PeerRole::Compute,
        vec![wrong_domain_id.to_string()],
        issued_at + 1,
    );
    let error = authority
        .install_credential_for_domain(
            SignedP2pCredential::new(sign(&wrong_domain)).unwrap(),
            required_domain_id,
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        P2pCredentialError::CredentialDomainMismatch
    ));
    assert_eq!(authority.current_claims().await.unwrap(), current);
    assert_eq!(
        authority.require(required_domain_id).await.unwrap(),
        current
    );
    node.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_credential_updates_cannot_finish_with_older_authority() {
    let node = listening_node();
    let credentials = node.authority();
    let domain_id = Uuid::new_v4().to_string();
    let issued_at = unix_time();
    let mut updates = Vec::new();
    for offset in 0..16_u64 {
        let claims = claims(
            node.peer_id(),
            PeerRole::Compute,
            vec![domain_id.clone()],
            issued_at + offset,
        );
        let store = credentials.clone();
        let signed = SignedP2pCredential::new(sign(&claims)).unwrap();
        updates.push(tokio::spawn(async move {
            store.install_credential(signed).await
        }));
    }
    for update in updates {
        let _ = update.await.unwrap();
    }
    assert_eq!(
        credentials.current_claims().await.unwrap().iat,
        issued_at + 15
    );
    node.shutdown().await.unwrap();
}

#[test]
fn signed_credential_debug_never_exposes_the_compact_token() {
    let marker = "header.payload.super-secret-signature";
    let credential = SignedP2pCredential::new(marker).unwrap();
    let debug = format!("{credential:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(marker));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_stream_after_disconnect_performs_a_fresh_handshake() {
    let domain_id = Uuid::new_v4().to_string();
    let wrong_domain_id = Uuid::new_v4().to_string();
    let robot = listening_node();
    let compute = listening_node();
    install_current_token(&robot, PeerRole::Robot, vec![domain_id.clone()]).await;
    install_current_token(&compute, PeerRole::Compute, vec![domain_id.clone()]).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = robot
        .accept(
            protocol.clone(),
            SessionRequirements::new(&domain_id).unwrap(),
        )
        .unwrap();
    let robot_peer_id = robot.peer_id();
    let robot_address = listen_address(&robot).await;

    let server = tokio::spawn(async move {
        let mut first = incoming.accept().await.unwrap().unwrap();
        let mut request = [0; 1];
        first.read_exact(&mut request).await.unwrap();
        first.write_all(&request).await.unwrap();
        first.flush().await.unwrap();
        drop(first);

        incoming.accept().await.unwrap().unwrap_err()
    });

    let requirements = SessionRequirements::new(&domain_id)
        .unwrap()
        .with_expected_remote_peer_id(robot_peer_id);
    let mut first = compute
        .open(
            robot_peer_id,
            vec![robot_address.clone()],
            protocol.clone(),
            requirements.clone(),
        )
        .await
        .unwrap();
    first.write_all(b"1").await.unwrap();
    first.flush().await.unwrap();
    let mut response = [0; 1];
    first.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"1");
    drop(first);

    let robot_credentials = robot.authority();
    let next_issued_at = robot_credentials.current_claims().await.unwrap().iat + 1;
    let wrong_domain_claims = claims(
        robot.peer_id(),
        PeerRole::Robot,
        vec![wrong_domain_id],
        next_issued_at,
    );
    robot_credentials
        .install_credential(SignedP2pCredential::new(sign(&wrong_domain_claims)).unwrap())
        .await
        .unwrap();
    compute.disconnect(robot_peer_id).await.unwrap();

    let error = compute
        .open(robot_peer_id, vec![robot_address], protocol, requirements)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::RemoteDomainMismatch(_)),
        "unexpected reconnect error: {error:?}"
    );
    assert!(matches!(
        server.await.unwrap(),
        Error::RemoteDomainMismatch(_)
    ));

    robot.shutdown().await.unwrap();
    compute.shutdown().await.unwrap();
}

async fn rejected_session(
    robot_domain: String,
    compute_domain: String,
    required_domain: String,
) -> (Error, Error) {
    let robot = listening_node();
    let compute = listening_node();
    let robot_observations = robot.observations();
    let compute_observations = compute.observations();
    let mut robot_events = robot_observations.subscribe();
    let mut compute_events = compute_observations.subscribe();
    install_current_token(&robot, PeerRole::Robot, vec![robot_domain]).await;
    install_current_token(&compute, PeerRole::Compute, vec![compute_domain]).await;

    let protocol = ApplicationProtocol::new(TEST_PROTOCOL).unwrap();
    let mut incoming = robot
        .accept(
            protocol.clone(),
            SessionRequirements::new(&required_domain).unwrap(),
        )
        .unwrap();
    let robot_peer_id = robot.peer_id();
    let robot_address = listen_address(&robot).await;
    let server = tokio::spawn(async move { incoming.accept().await.unwrap().unwrap_err() });
    let client_error = compute
        .open(
            robot_peer_id,
            vec![robot_address],
            protocol,
            SessionRequirements::new(&required_domain).unwrap(),
        )
        .await
        .unwrap_err();
    let server_error = server.await.unwrap();

    assert!(robot_observations.snapshot().peers().is_empty());
    assert!(compute_observations.snapshot().peers().is_empty());
    assert_eq!(robot_events.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(compute_events.try_recv(), Err(TryRecvError::Empty));

    robot.shutdown().await.unwrap();
    compute.shutdown().await.unwrap();
    (server_error, client_error)
}

fn listening_node() -> Node {
    Node::start(
        Identity::generate(),
        verifier(),
        ["/ip4/127.0.0.1/tcp/0".parse::<Multiaddr>().unwrap()],
    )
    .unwrap()
}

async fn listen_address(node: &Node) -> Multiaddr {
    tokio::time::timeout(Duration::from_secs(5), node.first_listen_address())
        .await
        .expect("listener did not start")
        .unwrap()
}

async fn install_current_token(node: &Node, role: PeerRole, domain_ids: Vec<String>) {
    let claims = claims(node.peer_id(), role, domain_ids, unix_time());
    install_signed_token(node, sign(&claims)).await;
}

async fn install_signed_token(node: &Node, token: String) {
    node.authority()
        .install_credential(SignedP2pCredential::new(token).unwrap())
        .await
        .unwrap();
}

fn claims(
    peer_id: libp2p::PeerId,
    role: PeerRole,
    domain_ids: Vec<String>,
    issued_at: u64,
) -> P2PAccessClaims {
    P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: Uuid::new_v4().to_string(),
        peer_type: Some(role.to_string()),
        peer_id: peer_id.to_string(),
        domain_ids,
        scopes: vec![P2P_TOKEN_SCOPE.into()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: issued_at + P2P_TOKEN_TTL.as_secs(),
    }
}

fn sign(claims: &impl Serialize) -> String {
    encode(
        &Header::new(Algorithm::ES256),
        claims,
        &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

fn verifier() -> DdsTokenVerifier {
    DdsTokenVerifier::from_es256_pem(TEST_DDS_PUBLIC_KEY).unwrap()
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
