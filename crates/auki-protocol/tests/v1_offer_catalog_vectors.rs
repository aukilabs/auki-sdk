use auki_protocol::v1::{
    error,
    offer::{
        OfferAccessMode, OfferCatalogRequest, OfferCatalogRequestError, OfferCatalogResponse,
        OfferCatalogResponseError, OfferError, OfferStatus, RegistryReferenceError,
    },
};
use serde_json::Value;

const V1_OFFER_CATALOG_VECTORS: &str = include_str!("../vectors/v1_offer_catalogs.json");

fn fixture() -> Value {
    serde_json::from_str(V1_OFFER_CATALOG_VECTORS).expect("valid offer-catalog fixture")
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
fn positive_offer_catalog_vectors_match_implementation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.offer_catalog.vectors".to_owned())
    );

    let request_object = positive_object(&fixture, "filtered_request");
    let request_expected = positive_expected(&fixture, "filtered_request");
    let request = OfferCatalogRequest::from_value(request_object.clone()).unwrap();

    assert_eq!(request.value(), request_object);
    assert_eq!(request.domain_ids, vec![input(&fixture, "domain_id")]);
    assert_eq!(request.kinds, vec!["sensor.frame"]);
    assert!(request.include_inline_registry_entries);
    assert_eq!(request_expected["domain_ids"], request_object["domain_ids"]);
    assert_eq!(request_expected["kinds"], request_object["kinds"]);
    assert_eq!(
        request_expected["include_inline_registry_entries"],
        request_object["include_inline_registry_entries"]
    );

    let response_object = positive_object(&fixture, "response_with_offer");
    let response_expected = positive_expected(&fixture, "response_with_offer");
    let response = OfferCatalogResponse::from_value(response_object.clone()).unwrap();

    assert_eq!(response.value(), response_object);
    assert_eq!(
        response.generated_at.as_deref(),
        response_expected["generated_at"].as_str()
    );
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(
        response.diagnostics[0]["code"],
        response_expected["diagnostic_code"]
    );
    assert_eq!(response.offers.len(), 1);

    let offer = &response.offers[0];
    assert_eq!(offer.offer_id, response_expected["offer_id"]);
    assert_eq!(offer.domain_id, input(&fixture, "domain_id"));
    assert_eq!(offer.kind, "sensor.frame");
    assert_eq!(offer.status, OfferStatus::Available);
    assert_eq!(
        offer.access_modes,
        vec![OfferAccessMode::Get, OfferAccessMode::Subscribe]
    );
    assert_eq!(offer.payload.payload_type, "auki.frame");
    assert_eq!(offer.registry_refs.len(), 1);
    assert_eq!(
        offer.registry_refs[0].hash,
        response_expected["registry_hash"]
    );
    assert_eq!(
        offer.registry_refs[0].canonical_json.as_deref(),
        Some(input(&fixture, "canonical_clock_json"))
    );
}

#[test]
fn negative_offer_catalog_request_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "request_invalid_domain_id"),
        error::OFFER_INVALID_CATALOG_REQUEST
    );
    let error = OfferCatalogRequest::from_value(
        negative_object(&fixture, "request_invalid_domain_id").clone(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        OfferCatalogRequestError::InvalidDomainId { index: 0, .. }
    ));
    assert_eq!(error.failure_code(), error::OFFER_INVALID_CATALOG_REQUEST);
}

#[test]
fn negative_offer_catalog_response_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "response_duplicate_offer"),
        error::OFFER_INVALID_CATALOG_RESPONSE
    );
    let duplicate = OfferCatalogResponse::from_value(
        negative_object(&fixture, "response_duplicate_offer").clone(),
    )
    .unwrap_err();
    assert_eq!(
        duplicate,
        OfferCatalogResponseError::DuplicateOffer {
            domain_id: input(&fixture, "domain_id").to_owned(),
            offer_id: "camera-main".to_owned(),
        }
    );
    assert_eq!(
        duplicate.failure_code(),
        error::OFFER_INVALID_CATALOG_RESPONSE
    );

    assert_eq!(
        expected_negative(&fixture, "response_registry_hash_mismatch"),
        error::OFFER_INVALID_CATALOG_RESPONSE
    );
    assert_eq!(
        fixture["negative"]["response_registry_hash_mismatch"]["expected_offer"]
            .as_str()
            .unwrap(),
        error::OFFER_INVALID_OFFER
    );
    let hash_mismatch = OfferCatalogResponse::from_value(
        negative_object(&fixture, "response_registry_hash_mismatch").clone(),
    )
    .unwrap_err();
    assert_eq!(
        hash_mismatch.failure_code(),
        error::OFFER_INVALID_CATALOG_RESPONSE
    );
    assert!(matches!(
        hash_mismatch,
        OfferCatalogResponseError::InvalidOffer {
            index: 0,
            error: OfferError::InvalidRegistryReference {
                index: 0,
                error: RegistryReferenceError::CanonicalJsonHashMismatch { .. },
            },
        }
    ));
}
