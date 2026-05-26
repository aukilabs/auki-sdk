use auki_protocol::v1::{
    error,
    get::{GetRequest, GetRequestError, GetResponse, GetResponseBody, GetResponseError},
};
use serde_json::Value;

const V1_GET_VECTORS: &str = include_str!("../vectors/v1_get.json");

fn fixture() -> Value {
    serde_json::from_str(V1_GET_VECTORS).expect("valid Get fixture")
}

fn input<'a>(fixture: &'a Value, key: &str) -> &'a str {
    fixture["inputs"][key].as_str().expect("input string")
}

fn input_u64(fixture: &Value, key: &str) -> u64 {
    fixture["inputs"][key].as_u64().expect("input u64")
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
fn positive_get_vectors_match_implementation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.get.vectors".to_owned())
    );

    let request_object = positive_object(&fixture, "request");
    let request_expected = positive_expected(&fixture, "request");
    let request = GetRequest::from_value(request_object.clone()).unwrap();

    assert_eq!(request.value(), request_object);
    assert_eq!(request.domain_id, input(&fixture, "domain_id"));
    assert_eq!(request.offer_id, input(&fixture, "offer_id"));
    assert_eq!(request.accepted_payload_types, vec!["auki.frame"]);
    assert_eq!(
        request.max_payload_bytes,
        Some(request_expected["max_payload_bytes"].as_u64().unwrap())
    );
    assert!(request.accepts_payload_type(input(&fixture, "selected_payload_type")));

    let success_object = positive_object(&fixture, "success_response");
    let success_expected = positive_expected(&fixture, "success_response");
    let success = GetResponse::from_value(success_object.clone()).unwrap();
    let message = success
        .validate_success_for_request(&request, input(&fixture, "selected_payload_type"))
        .unwrap();

    assert_eq!(success.value(), success_object);
    assert_eq!(
        message.generated_at.as_deref(),
        success_expected["generated_at"].as_str()
    );
    assert_eq!(
        message.raw_payload_len() as u64,
        success_expected["raw_payload_len"].as_u64().unwrap()
    );

    let failure_object = positive_object(&fixture, "failure_response");
    let failure_expected = positive_expected(&fixture, "failure_response");
    let failure = GetResponse::from_value(failure_object.clone()).unwrap();

    assert_eq!(failure.value(), failure_object);
    let GetResponseBody::Error(error_object) = failure.body else {
        panic!("expected failed Get response");
    };
    assert_eq!(error_object.code, failure_expected["code"]);
}

#[test]
fn negative_get_request_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "request_invalid_domain_id"),
        error::GET_INVALID_REQUEST
    );
    let invalid_domain =
        GetRequest::from_value(negative_object(&fixture, "request_invalid_domain_id").clone())
            .unwrap_err();
    assert!(matches!(
        invalid_domain,
        GetRequestError::InvalidDomainId { .. }
    ));
    assert_eq!(invalid_domain.failure_code(), error::GET_INVALID_REQUEST);

    assert_eq!(
        expected_negative(&fixture, "request_zero_max_payload_bytes"),
        error::GET_INVALID_REQUEST
    );
    let zero_limit =
        GetRequest::from_value(negative_object(&fixture, "request_zero_max_payload_bytes").clone())
            .unwrap_err();
    assert_eq!(zero_limit, GetRequestError::MaxPayloadBytesZero);
    assert_eq!(zero_limit.failure_code(), error::GET_INVALID_REQUEST);
}

#[test]
fn negative_get_response_vectors_fail_as_locked() {
    let fixture = fixture();
    let request = GetRequest::from_value(positive_object(&fixture, "request").clone()).unwrap();

    assert_eq!(
        expected_negative(&fixture, "response_multiple_bodies"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let multiple_bodies =
        GetResponse::from_value(negative_object(&fixture, "response_multiple_bodies").clone())
            .unwrap_err();
    assert_eq!(multiple_bodies, GetResponseError::MultipleBodies);
    assert_eq!(
        multiple_bodies.failure_code(),
        error::MESSAGE_INVALID_ENVELOPE
    );

    assert_eq!(
        expected_negative(&fixture, "response_offer_mismatch"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let offer_mismatch =
        GetResponse::from_value(negative_object(&fixture, "response_offer_mismatch").clone())
            .unwrap();
    let offer_error = offer_mismatch
        .validate_success_for_request(&request, input(&fixture, "selected_payload_type"))
        .unwrap_err();
    assert_eq!(offer_error.failure_code(), error::MESSAGE_INVALID_ENVELOPE);

    assert_eq!(
        expected_negative(&fixture, "response_payload_type_mismatch"),
        error::MESSAGE_INVALID_PAYLOAD
    );
    let payload_mismatch = GetResponse::from_value(
        negative_object(&fixture, "response_payload_type_mismatch").clone(),
    )
    .unwrap();
    let payload_error = payload_mismatch
        .validate_success_for_request(&request, input(&fixture, "selected_payload_type"))
        .unwrap_err();
    assert_eq!(payload_error.failure_code(), error::MESSAGE_INVALID_PAYLOAD);

    assert_eq!(
        expected_negative(&fixture, "response_payload_type_not_accepted"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let payload_type_not_accepted = GetResponse::from_value(
        negative_object(&fixture, "response_payload_type_not_accepted").clone(),
    )
    .unwrap();
    let payload_type_not_accepted_error = payload_type_not_accepted
        .validate_success_for_request(
            &request,
            fixture["negative"]["response_payload_type_not_accepted"]["selected_payload_type"]
                .as_str()
                .unwrap(),
        )
        .unwrap_err();
    assert_eq!(
        payload_type_not_accepted_error,
        GetResponseError::PayloadTypeNotAccepted {
            payload_type: "other.payload".to_owned(),
        }
    );
    assert_eq!(
        payload_type_not_accepted_error.failure_code(),
        error::MESSAGE_INVALID_ENVELOPE
    );

    assert_eq!(
        expected_negative(&fixture, "response_payload_too_large"),
        error::MESSAGE_PAYLOAD_TOO_LARGE
    );
    let small_request = GetRequest::create(
        input(&fixture, "domain_id"),
        input(&fixture, "offer_id"),
        None,
        vec![input(&fixture, "selected_payload_type").to_owned()],
        Some(
            fixture["negative"]["response_payload_too_large"]["request_max_payload_bytes"]
                .as_u64()
                .unwrap(),
        ),
    )
    .unwrap();
    assert!(small_request.max_payload_bytes.unwrap() < input_u64(&fixture, "max_payload_bytes"));

    let too_large =
        GetResponse::from_value(negative_object(&fixture, "response_payload_too_large").clone())
            .unwrap();
    let too_large_error = too_large
        .validate_success_for_request(&small_request, input(&fixture, "selected_payload_type"))
        .unwrap_err();
    assert_eq!(
        too_large_error.failure_code(),
        error::MESSAGE_PAYLOAD_TOO_LARGE
    );
}
