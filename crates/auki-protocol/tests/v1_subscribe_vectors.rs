use auki_protocol::v1::{
    error,
    message::SpatialMessage,
    subscribe::{
        SubscribeAcceptError, SubscribeEnd, SubscribeEndError, SubscribeRequest,
        SubscribeRequestError, SubscribeStartResult, SubscribeStartResultBody,
        SubscribeStartResultError,
    },
};
use serde_json::Value;

const V1_SUBSCRIBE_VECTORS: &str = include_str!("../vectors/v1_subscribe.json");

fn fixture() -> Value {
    serde_json::from_str(V1_SUBSCRIBE_VECTORS).expect("valid Subscribe fixture")
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
fn positive_subscribe_vectors_match_implementation() {
    let fixture = fixture();
    assert_eq!(
        fixture["name"],
        Value::String("auki.protocol.v1.subscribe.vectors".to_owned())
    );

    let request_object = positive_object(&fixture, "request");
    let request_expected = positive_expected(&fixture, "request");
    let request = SubscribeRequest::from_value(request_object.clone()).unwrap();

    assert_eq!(request.value(), request_object);
    assert_eq!(request.domain_id, input(&fixture, "domain_id"));
    assert_eq!(request.offer_id, input(&fixture, "offer_id"));
    assert_eq!(request.accepted_payload_types, vec!["auki.frame"]);
    assert_eq!(
        request.max_message_bytes,
        Some(request_expected["max_message_bytes"].as_u64().unwrap())
    );
    assert!(request.accepts_payload_type(input(&fixture, "selected_payload_type")));

    let accept_object = positive_object(&fixture, "accept_start_result");
    let accept_expected = positive_expected(&fixture, "accept_start_result");
    let start = SubscribeStartResult::from_value(accept_object.clone()).unwrap();
    assert_eq!(start.value(), accept_object);
    let SubscribeStartResultBody::Accept(accept) = start.body else {
        panic!("expected accepted Subscribe start result");
    };

    accept.validate_for_request(&request).unwrap();
    assert_eq!(accept.domain_id, input(&fixture, "domain_id"));
    assert_eq!(accept.offer_id, input(&fixture, "offer_id"));
    assert_eq!(
        accept.payload.payload_type,
        input(&fixture, "selected_payload_type")
    );
    assert_eq!(
        accept.initial_sequence,
        Some(accept_expected["initial_sequence"].as_u64().unwrap())
    );
    assert_eq!(
        accept.generated_at.as_deref(),
        accept_expected["generated_at"].as_str()
    );
    assert_eq!(
        accept.registry_refs.len() as u64,
        accept_expected["registry_refs"].as_u64().unwrap()
    );

    let data_object = positive_object(&fixture, "data_message");
    let data_expected = positive_expected(&fixture, "data_message");
    let data = SpatialMessage::from_value(data_object.clone()).unwrap();
    accept
        .validate_data_message(&data, request.max_message_bytes)
        .unwrap();
    assert_eq!(data.value(), data_object);
    assert_eq!(data.sequence, Some(input_u64(&fixture, "initial_sequence")));
    assert_eq!(
        data.raw_payload_len() as u64,
        data_expected["raw_payload_len"].as_u64().unwrap()
    );

    let reject_object = positive_object(&fixture, "reject_start_result");
    let reject_expected = positive_expected(&fixture, "reject_start_result");
    let start = SubscribeStartResult::from_value(reject_object.clone()).unwrap();
    assert_eq!(start.value(), reject_object);
    let SubscribeStartResultBody::Reject(reject) = start.body else {
        panic!("expected rejected Subscribe start result");
    };
    assert_eq!(reject.error.code, reject_expected["code"]);

    let end_object = positive_object(&fixture, "end_message");
    let end_expected = positive_expected(&fixture, "end_message");
    let end = SubscribeEnd::from_value(end_object.clone()).unwrap();
    assert_eq!(end.value(), end_object);
    assert_eq!(
        end.reason.as_str(),
        end_expected["reason"].as_str().unwrap()
    );
    assert_eq!(end.retryable, end_expected["retryable"].as_bool());
    end.validate_for_offer(input(&fixture, "domain_id"), input(&fixture, "offer_id"))
        .unwrap();
}

#[test]
fn negative_subscribe_request_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "request_invalid_domain_id"),
        error::SUBSCRIBE_INVALID_REQUEST
    );
    let invalid_domain = SubscribeRequest::from_value(
        negative_object(&fixture, "request_invalid_domain_id").clone(),
    )
    .unwrap_err();
    assert!(matches!(
        invalid_domain,
        SubscribeRequestError::InvalidDomainId { .. }
    ));
    assert_eq!(
        invalid_domain.failure_code(),
        error::SUBSCRIBE_INVALID_REQUEST
    );

    assert_eq!(
        expected_negative(&fixture, "request_zero_max_message_bytes"),
        error::SUBSCRIBE_INVALID_REQUEST
    );
    let zero_limit = SubscribeRequest::from_value(
        negative_object(&fixture, "request_zero_max_message_bytes").clone(),
    )
    .unwrap_err();
    assert_eq!(zero_limit, SubscribeRequestError::MaxMessageBytesZero);
    assert_eq!(zero_limit.failure_code(), error::SUBSCRIBE_INVALID_REQUEST);
}

#[test]
fn negative_subscribe_start_vectors_fail_as_locked() {
    let fixture = fixture();
    let request =
        SubscribeRequest::from_value(positive_object(&fixture, "request").clone()).unwrap();

    assert_eq!(
        expected_negative(&fixture, "start_unsupported_type"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let unsupported = SubscribeStartResult::from_value(
        negative_object(&fixture, "start_unsupported_type").clone(),
    )
    .unwrap_err();
    assert!(matches!(
        unsupported,
        SubscribeStartResultError::UnsupportedType { .. }
    ));
    assert_eq!(unsupported.failure_code(), error::MESSAGE_INVALID_ENVELOPE);

    assert_eq!(
        expected_negative(&fixture, "accept_offer_mismatch"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let start = SubscribeStartResult::from_value(
        negative_object(&fixture, "accept_offer_mismatch").clone(),
    )
    .unwrap();
    let accept = start.accept_body().expect("accept start result");
    let mismatch = accept.validate_for_request(&request).unwrap_err();
    assert!(matches!(
        mismatch,
        SubscribeAcceptError::RequestMismatch { .. }
    ));
    assert_eq!(mismatch.failure_code(), error::MESSAGE_INVALID_ENVELOPE);

    assert_eq!(
        expected_negative(&fixture, "accept_payload_type_not_accepted"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let start = SubscribeStartResult::from_value(
        negative_object(&fixture, "accept_payload_type_not_accepted").clone(),
    )
    .unwrap();
    let accept = start.accept_body().expect("accept start result");
    let mismatch = accept.validate_for_request(&request).unwrap_err();
    assert_eq!(
        mismatch,
        SubscribeAcceptError::PayloadTypeNotAccepted {
            payload_type: "other.payload".to_owned(),
        }
    );
    assert_eq!(mismatch.failure_code(), error::MESSAGE_INVALID_ENVELOPE);
}

#[test]
fn negative_subscribe_data_vectors_fail_as_locked() {
    let fixture = fixture();
    let start =
        SubscribeStartResult::from_value(positive_object(&fixture, "accept_start_result").clone())
            .unwrap();
    let accept = start.accept_body().expect("accept start result");

    assert_eq!(
        expected_negative(&fixture, "data_message_offer_mismatch"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let offer_mismatch = SpatialMessage::from_value(
        negative_object(&fixture, "data_message_offer_mismatch").clone(),
    )
    .unwrap();
    let offer_error = accept
        .validate_data_message(&offer_mismatch, None)
        .unwrap_err();
    assert_eq!(offer_error.failure_code(), error::MESSAGE_INVALID_ENVELOPE);

    assert_eq!(
        expected_negative(&fixture, "data_message_payload_type_mismatch"),
        error::MESSAGE_INVALID_PAYLOAD
    );
    let payload_mismatch = SpatialMessage::from_value(
        negative_object(&fixture, "data_message_payload_type_mismatch").clone(),
    )
    .unwrap();
    let payload_error = accept
        .validate_data_message(&payload_mismatch, None)
        .unwrap_err();
    assert_eq!(payload_error.failure_code(), error::MESSAGE_INVALID_PAYLOAD);

    assert_eq!(
        expected_negative(&fixture, "data_message_payload_too_large"),
        error::MESSAGE_PAYLOAD_TOO_LARGE
    );
    let too_large = SpatialMessage::from_value(
        negative_object(&fixture, "data_message_payload_too_large").clone(),
    )
    .unwrap();
    let max = fixture["negative"]["data_message_payload_too_large"]["request_max_message_bytes"]
        .as_u64()
        .unwrap();
    assert!(max < input_u64(&fixture, "max_message_bytes"));
    let size_error = accept
        .validate_data_message(&too_large, Some(max))
        .unwrap_err();
    assert_eq!(size_error.failure_code(), error::MESSAGE_PAYLOAD_TOO_LARGE);
}

#[test]
fn negative_subscribe_end_vectors_fail_as_locked() {
    let fixture = fixture();

    assert_eq!(
        expected_negative(&fixture, "end_unknown_reason"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let unknown_reason =
        SubscribeEnd::from_value(negative_object(&fixture, "end_unknown_reason").clone())
            .unwrap_err();
    assert!(matches!(
        unknown_reason,
        SubscribeEndError::UnsupportedReason { .. }
    ));
    assert_eq!(
        unknown_reason.failure_code(),
        error::MESSAGE_INVALID_ENVELOPE
    );

    assert_eq!(
        expected_negative(&fixture, "end_path_mismatch"),
        error::MESSAGE_INVALID_ENVELOPE
    );
    let end =
        SubscribeEnd::from_value(negative_object(&fixture, "end_path_mismatch").clone()).unwrap();
    let path_error = end
        .validate_for_offer(input(&fixture, "domain_id"), input(&fixture, "offer_id"))
        .unwrap_err();
    assert!(matches!(path_error, SubscribeEndError::PathMismatch { .. }));
    assert_eq!(path_error.failure_code(), error::MESSAGE_INVALID_ENVELOPE);
}
