//! Cross-repository Stage 1 release vector.
//!
//! Posemesh carries the same JSON bytes and independently checks the identity,
//! signed DDS claims, authentication frame, and resource payload. This test
//! additionally round-trips the retained v0.2 wire contract.

// The release vector also verifies ES256 through jsonwebtoken/ring, which is a
// native test dependency. Portable codecs remain covered by the Wasm all-target check.
#![cfg(not(target_arch = "wasm32"))]

use auki_p2p::{
    Identity, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims,
};
use auki_protocols::catalog::v2::{
    ID as RESOURCES_V0_2_0, ResourcesRequest, ResourcesResponse, read_resources_request,
    read_resources_response, write_resources_request, write_resources_response,
};
use futures::io::Cursor;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;

const VECTOR_JSON: &str = include_str!("fixtures/stage1_cross_repository_vector.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage1Vector {
    schema_version: u32,
    protocol_id: String,
    domain_id: String,
    identity_seed_hex: String,
    peer_id: String,
    dds_es256_public_key_pem: String,
    signed_claims: P2PAccessClaims,
    signed_credential: String,
    mutual_auth_frame_hex: String,
    resources_request_frame_hex: String,
    resources_response: ResourcesResponse,
    resources_response_frame_hex: String,
}

fn vector() -> Stage1Vector {
    serde_json::from_str(VECTOR_JSON).expect("Stage 1 vector must use the locked schema")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn identity_seed(encoded: &str) -> [u8; 32] {
    let bytes = hex::decode(encoded).expect("identity seed must be hexadecimal");
    bytes
        .try_into()
        .expect("identity seed must contain exactly 32 bytes")
}

fn authentication_frame(credential: &str) -> Vec<u8> {
    let mut frame = (credential.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(credential.as_bytes());
    // Both authenticated sides exchange this acceptance byte only after
    // verifying the other side's complete credential and Noise Peer ID.
    frame.push(1);
    frame
}

#[tokio::test]
async fn sdk_matches_the_cross_repository_stage1_vector() {
    let fixture = vector();
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.protocol_id, RESOURCES_V0_2_0);

    let identity = Identity::from_ed25519_seed(&identity_seed(&fixture.identity_seed_hex));
    assert_eq!(identity.peer_id().to_string(), fixture.peer_id);
    assert_eq!(fixture.signed_claims.peer_id, fixture.peer_id);
    assert_eq!(fixture.signed_claims.domain_ids, [fixture.domain_id]);
    assert_eq!(fixture.signed_claims.token_type, P2P_TOKEN_TYPE);
    assert_eq!(fixture.signed_claims.iss, P2P_TOKEN_ISSUER);
    assert_eq!(fixture.signed_claims.aud, [P2P_TOKEN_AUDIENCE]);
    assert_eq!(
        fixture.signed_claims.exp - fixture.signed_claims.iat,
        P2P_TOKEN_TTL.as_secs()
    );

    // The vector is deliberately time-independent: its historical timestamps
    // are not accepted as live authority, but its ES256 signature and exact
    // bounded claim schema remain stable release evidence.
    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.set_audience(&[P2P_TOKEN_AUDIENCE]);
    validation.set_issuer(&[P2P_TOKEN_ISSUER]);
    let decoded = decode::<P2PAccessClaims>(
        &fixture.signed_credential,
        &DecodingKey::from_ec_pem(fixture.dds_es256_public_key_pem.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap();
    assert_eq!(decoded.claims, fixture.signed_claims);
    assert_eq!(
        hex(&authentication_frame(&fixture.signed_credential)),
        fixture.mutual_auth_frame_hex
    );

    let request_bytes = hex::decode(&fixture.resources_request_frame_hex).unwrap();
    let request = read_resources_request(&mut Cursor::new(request_bytes.clone()))
        .await
        .unwrap();
    assert_eq!(request, ResourcesRequest::all());
    let mut encoded_request = Vec::new();
    write_resources_request(&mut encoded_request, &request)
        .await
        .unwrap();
    assert_eq!(encoded_request, request_bytes);

    let response_bytes = hex::decode(&fixture.resources_response_frame_hex).unwrap();
    let response = read_resources_response(&mut Cursor::new(response_bytes.clone()))
        .await
        .unwrap();
    assert_eq!(response, fixture.resources_response);
    assert_eq!(response.resources.len(), 1);
    assert_eq!(response.resources[0].writer_peer_id, fixture.peer_id);
    let mut encoded_response = Vec::new();
    write_resources_response(&mut encoded_response, &response)
        .await
        .unwrap();
    assert_eq!(encoded_response, response_bytes);
}
