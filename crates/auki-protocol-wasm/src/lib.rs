//! Browser-facing wasm-bindgen adapter over `auki-protocol`.

use auki_identity::{PublicKey as WalletPublicKey, Wallet};
use auki_protocol::v1::{
    authority::{DeclaredDomain, PeerAuthorization, ServedDomainAuthority},
    base64url,
    domain::{
        DOMAIN_NONCE_LEN, DelegationScope, DomainDeclaration, DomainDelegation,
        DomainDelegationParams, DomainError,
    },
    error, frame,
    get::{GET_PROTOCOL_ID, GetRequest, GetResponse},
    handshake::{CLUSTER_LIFECYCLE_V1, PeerHandshake},
    identity::{PeerBinding, PeerBindingError},
    message::{ErrorObject, SpatialMessage},
    offer::{
        OFFER_CATALOG_PROTOCOL_ID, OfferCatalogPath, OfferCatalogRequest, OfferCatalogResponse,
    },
    status::StatusSnapshot,
    subscribe::{SUBSCRIBE_PROTOCOL_ID, SubscribeEnd, SubscribeRequest, SubscribeStartResult},
};
use libp2p_identity::PeerId;
use serde::Serialize as _;
use serde_json::{Map, Value, json};
use std::{fmt, str::FromStr};
use wasm_bindgen::prelude::*;

type ProtocolResult<T> = Result<T, ProtocolWasmError>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProtocolWasmError {
    kind: &'static str,
    message: String,
    failure_code: Option<&'static str>,
}

impl ProtocolWasmError {
    fn new(kind: &'static str, error: impl fmt::Display) -> Self {
        Self {
            kind,
            message: error.to_string(),
            failure_code: None,
        }
    }

    fn with_failure_code(
        kind: &'static str,
        error: impl fmt::Display,
        failure_code: &'static str,
    ) -> Self {
        Self {
            kind,
            message: error.to_string(),
            failure_code: Some(failure_code),
        }
    }

    fn into_js(self) -> JsValue {
        let mut object = Map::new();
        object.insert("kind".to_owned(), Value::String(self.kind.to_owned()));
        object.insert("message".to_owned(), Value::String(self.message.clone()));
        if let Some(failure_code) = self.failure_code {
            object.insert(
                "failure_code".to_owned(),
                Value::String(failure_code.to_owned()),
            );
        }

        Value::Object(object)
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or_else(|_| JsValue::from_str(&self.message))
    }
}

impl fmt::Display for ProtocolWasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn value_to_js(value: Value) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|error| ProtocolWasmError::new("serde", error).into_js())
}

fn js_to_value(value: JsValue) -> Result<Value, JsValue> {
    serde_wasm_bindgen::from_value(value)
        .map_err(|error| ProtocolWasmError::new("js_value", error).into_js())
}

fn js_to_optional_value(value: JsValue) -> Result<Option<Value>, JsValue> {
    if value.is_undefined() || value.is_null() {
        Ok(None)
    } else {
        js_to_value(value).map(Some)
    }
}

fn js_to_optional_object(value: JsValue, kind: &'static str) -> Result<Option<Value>, JsValue> {
    let Some(value) = js_to_optional_value(value)? else {
        return Ok(None);
    };
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err(ProtocolWasmError::new(kind, "expected a json object").into_js())
    }
}

fn js_to_string_vec(value: JsValue, kind: &'static str) -> Result<Vec<String>, JsValue> {
    if value.is_undefined() || value.is_null() {
        Ok(Vec::new())
    } else {
        serde_wasm_bindgen::from_value(value)
            .map_err(|error| ProtocolWasmError::new(kind, error).into_js())
    }
}

fn js_to_optional_u64(value: JsValue, kind: &'static str) -> Result<Option<u64>, JsValue> {
    if value.is_undefined() || value.is_null() {
        Ok(None)
    } else {
        serde_wasm_bindgen::from_value(value)
            .map(Some)
            .map_err(|error| ProtocolWasmError::new(kind, error).into_js())
    }
}

fn ok_value(result: ProtocolResult<Value>) -> Result<JsValue, JsValue> {
    value_to_js(result.map_err(ProtocolWasmError::into_js)?)
}

/// Return the wrapped protocol version string.
#[wasm_bindgen(js_name = protocolVersion)]
pub fn protocol_version() -> String {
    "auki.protocol.v1".to_owned()
}

/// Return the v1 protocol ids and stable failure-code strings.
#[wasm_bindgen(js_name = protocolConstants)]
pub fn protocol_constants() -> Result<JsValue, JsValue> {
    value_to_js(protocol_constants_value())
}

/// Encode an unsigned LEB128 frame length.
#[wasm_bindgen(js_name = encodeLength)]
pub fn encode_length(value: u32) -> Vec<u8> {
    frame::encode_length(u64::from(value))
}

/// Decode an unsigned LEB128 frame length.
#[wasm_bindgen(js_name = decodeLength)]
pub fn decode_length(input: &[u8], max_body_len: u32) -> Result<JsValue, JsValue> {
    frame::decode_length(input, u64::from(max_body_len))
        .map(|(value, consumed)| json!({ "value": value, "consumed": consumed }))
        .map_err(|error| ProtocolWasmError::new("frame", error).into_js())
        .and_then(value_to_js)
}

/// Encode a v1 JSON frame using `auki-protocol`.
#[wasm_bindgen(js_name = encodeJsonFrame)]
pub fn encode_json_frame(value: JsValue, max_body_len: u32) -> Result<Vec<u8>, JsValue> {
    let value = js_to_value(value)?;
    encode_json_frame_value(&value, max_body_len).map_err(ProtocolWasmError::into_js)
}

/// Decode one v1 JSON frame using `auki-protocol`.
#[wasm_bindgen(js_name = decodeJsonFrame)]
pub fn decode_json_frame(input: &[u8], max_body_len: u32) -> Result<JsValue, JsValue> {
    ok_value(decode_json_frame_value(input, max_body_len))
}

/// Return the wallet public key for a 32-byte wallet seed.
#[wasm_bindgen(js_name = walletPublicKeyFromSeed)]
pub fn wallet_public_key_from_seed(seed: &[u8]) -> Result<String, JsValue> {
    wallet_public_key_from_seed_value(seed).map_err(ProtocolWasmError::into_js)
}

/// Create a wallet-signed peer binding.
#[wasm_bindgen(js_name = createPeerBinding)]
pub fn create_peer_binding(
    wallet_seed: &[u8],
    peer_id: &str,
    issued_at: &str,
    label: Option<String>,
) -> Result<JsValue, JsValue> {
    ok_value(create_peer_binding_value(
        wallet_seed,
        peer_id,
        issued_at,
        label.as_deref(),
    ))
}

/// Parse a peer binding and validate its protocol shape.
#[wasm_bindgen(js_name = parsePeerBinding)]
pub fn parse_peer_binding(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_peer_binding_value(js_to_value(value)?))
}

/// Verify a peer binding for the transport-authenticated peer id.
#[wasm_bindgen(js_name = verifyPeerBinding)]
pub fn verify_peer_binding(
    value: JsValue,
    authenticated_peer_id: &str,
) -> Result<JsValue, JsValue> {
    ok_value(verify_peer_binding_value(
        js_to_value(value)?,
        authenticated_peer_id,
    ))
}

/// Create a signed domain declaration.
#[wasm_bindgen(js_name = createDomainDeclaration)]
pub fn create_domain_declaration(
    owner_seed: &[u8],
    nonce: &[u8],
    label: Option<String>,
) -> Result<JsValue, JsValue> {
    ok_value(create_domain_declaration_value(
        owner_seed,
        nonce,
        label.as_deref(),
    ))
}

/// Parse a domain declaration and validate its protocol shape.
#[wasm_bindgen(js_name = parseDomainDeclaration)]
pub fn parse_domain_declaration(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_domain_declaration_value(js_to_value(value)?))
}

/// Verify a signed domain declaration.
#[wasm_bindgen(js_name = verifyDomainDeclaration)]
pub fn verify_domain_declaration(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(verify_domain_declaration_value(js_to_value(value)?))
}

/// Create a signed domain delegation.
#[wasm_bindgen(js_name = createDomainDelegation)]
pub fn create_domain_delegation(owner_seed: &[u8], params: JsValue) -> Result<JsValue, JsValue> {
    ok_value(create_domain_delegation_value(
        owner_seed,
        js_to_value(params)?,
    ))
}

/// Parse a domain delegation and validate its protocol shape.
#[wasm_bindgen(js_name = parseDomainDelegation)]
pub fn parse_domain_delegation(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_domain_delegation_value(js_to_value(value)?))
}

/// Verify a signed domain delegation.
#[wasm_bindgen(js_name = verifyDomainDelegation)]
pub fn verify_domain_delegation(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(verify_domain_delegation_value(js_to_value(value)?))
}

/// Create an offer-catalog path for handshake advertisement.
#[wasm_bindgen(js_name = createOfferCatalogPath)]
pub fn create_offer_catalog_path(metadata: JsValue) -> Result<JsValue, JsValue> {
    let metadata = js_to_optional_object(metadata, "offer_catalog_path")?;
    ok_value(
        OfferCatalogPath::create(metadata)
            .map(OfferCatalogPath::into_value)
            .map_err(|error| ProtocolWasmError::new("offer_catalog_path", error)),
    )
}

/// Parse an offer-catalog path and validate its protocol shape.
#[wasm_bindgen(js_name = parseOfferCatalogPath)]
pub fn parse_offer_catalog_path(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_offer_catalog_path_value(js_to_value(value)?))
}

/// Create a peer lifecycle handshake.
#[wasm_bindgen(js_name = createPeerHandshake)]
pub fn create_peer_handshake(
    peer_binding: JsValue,
    declared_domains: JsValue,
    offer_catalog: JsValue,
) -> Result<JsValue, JsValue> {
    ok_value(create_peer_handshake_value(
        js_to_value(peer_binding)?,
        js_to_value(declared_domains)?,
        js_to_optional_value(offer_catalog)?,
    ))
}

/// Parse a peer lifecycle handshake and validate its protocol shape.
#[wasm_bindgen(js_name = parsePeerHandshake)]
pub fn parse_peer_handshake(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_peer_handshake_value(js_to_value(value)?))
}

/// Validate a parsed peer handshake against the transport-authenticated peer id.
#[wasm_bindgen(js_name = validatePeerHandshakeAuthority)]
pub fn validate_peer_handshake_authority(
    handshake: JsValue,
    authenticated_peer_id: &str,
    peer_authorized: bool,
    now: &str,
) -> Result<JsValue, JsValue> {
    ok_value(validate_peer_handshake_authority_value(
        js_to_value(handshake)?,
        authenticated_peer_id,
        peer_authorized,
        now,
    ))
}

/// Create an offer-catalog request.
#[wasm_bindgen(js_name = createOfferCatalogRequest)]
pub fn create_offer_catalog_request(
    domain_ids: JsValue,
    kinds: JsValue,
    include_inline_registry_entries: bool,
) -> Result<JsValue, JsValue> {
    let domain_ids = js_to_string_vec(domain_ids, "offer_catalog_request")?;
    let kinds = js_to_string_vec(kinds, "offer_catalog_request")?;
    ok_value(
        OfferCatalogRequest::create(domain_ids, kinds, include_inline_registry_entries)
            .map(OfferCatalogRequest::into_value)
            .map_err(|error| {
                let failure_code = error.failure_code();
                ProtocolWasmError::with_failure_code("offer_catalog_request", error, failure_code)
            }),
    )
}

/// Parse an offer-catalog request and validate its protocol shape.
#[wasm_bindgen(js_name = parseOfferCatalogRequest)]
pub fn parse_offer_catalog_request(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_offer_catalog_request_value(js_to_value(value)?))
}

/// Parse an offer-catalog response and validate its protocol shape.
#[wasm_bindgen(js_name = parseOfferCatalogResponse)]
pub fn parse_offer_catalog_response(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_offer_catalog_response_value(js_to_value(value)?))
}

/// Create a Get request.
#[wasm_bindgen(js_name = createGetRequest)]
pub fn create_get_request(
    domain_id: &str,
    offer_id: &str,
    params: JsValue,
    accepted_payload_types: JsValue,
    max_payload_bytes: JsValue,
) -> Result<JsValue, JsValue> {
    let params = js_to_optional_object(params, "get_request")?;
    let accepted_payload_types = js_to_string_vec(accepted_payload_types, "get_request")?;
    let max_payload_bytes = js_to_optional_u64(max_payload_bytes, "get_request")?;
    ok_value(
        GetRequest::create(
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_payload_bytes,
        )
        .map(GetRequest::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("get_request", error, failure_code)
        }),
    )
}

/// Parse a Get request and validate its protocol shape.
#[wasm_bindgen(js_name = parseGetRequest)]
pub fn parse_get_request(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_get_request_value(js_to_value(value)?))
}

/// Parse a Get response and validate its protocol shape.
#[wasm_bindgen(js_name = parseGetResponse)]
pub fn parse_get_response(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_get_response_value(js_to_value(value)?))
}

/// Validate a successful Get response against its request and selected payload type.
#[wasm_bindgen(js_name = validateGetResponseForRequest)]
pub fn validate_get_response_for_request(
    request: JsValue,
    response: JsValue,
    selected_payload_type: &str,
) -> Result<JsValue, JsValue> {
    ok_value(validate_get_response_for_request_value(
        js_to_value(request)?,
        js_to_value(response)?,
        selected_payload_type,
    ))
}

/// Create a Subscribe request.
#[wasm_bindgen(js_name = createSubscribeRequest)]
pub fn create_subscribe_request(
    domain_id: &str,
    offer_id: &str,
    params: JsValue,
    accepted_payload_types: JsValue,
    max_message_bytes: JsValue,
) -> Result<JsValue, JsValue> {
    let params = js_to_optional_object(params, "subscribe_request")?;
    let accepted_payload_types = js_to_string_vec(accepted_payload_types, "subscribe_request")?;
    let max_message_bytes = js_to_optional_u64(max_message_bytes, "subscribe_request")?;
    ok_value(
        SubscribeRequest::create(
            domain_id,
            offer_id,
            params,
            accepted_payload_types,
            max_message_bytes,
        )
        .map(SubscribeRequest::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_request", error, failure_code)
        }),
    )
}

/// Parse a Subscribe request and validate its protocol shape.
#[wasm_bindgen(js_name = parseSubscribeRequest)]
pub fn parse_subscribe_request(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_subscribe_request_value(js_to_value(value)?))
}

/// Parse the first Subscribe stream result.
#[wasm_bindgen(js_name = parseSubscribeStartResult)]
pub fn parse_subscribe_start_result(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_subscribe_start_result_value(js_to_value(value)?))
}

/// Validate a Subscribe start result against its original request.
#[wasm_bindgen(js_name = validateSubscribeStartForRequest)]
pub fn validate_subscribe_start_for_request(
    request: JsValue,
    start_result: JsValue,
) -> Result<JsValue, JsValue> {
    ok_value(validate_subscribe_start_for_request_value(
        js_to_value(request)?,
        js_to_value(start_result)?,
    ))
}

/// Parse a Subscribe end message.
#[wasm_bindgen(js_name = parseSubscribeEnd)]
pub fn parse_subscribe_end(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_subscribe_end_value(js_to_value(value)?))
}

/// Validate a Subscribe end message against an accepted subscription path.
#[wasm_bindgen(js_name = validateSubscribeEndForOffer)]
pub fn validate_subscribe_end_for_offer(
    end: JsValue,
    domain_id: &str,
    offer_id: &str,
) -> Result<JsValue, JsValue> {
    ok_value(validate_subscribe_end_for_offer_value(
        js_to_value(end)?,
        domain_id,
        offer_id,
    ))
}

/// Parse a spatial message envelope.
#[wasm_bindgen(js_name = parseSpatialMessage)]
pub fn parse_spatial_message(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_spatial_message_value(js_to_value(value)?))
}

/// Validate a spatial message against an accepted offer path and payload type.
#[wasm_bindgen(js_name = validateSpatialMessageForOffer)]
pub fn validate_spatial_message_for_offer(
    message: JsValue,
    domain_id: &str,
    offer_id: &str,
    selected_payload_type: &str,
) -> Result<JsValue, JsValue> {
    ok_value(validate_spatial_message_for_offer_value(
        js_to_value(message)?,
        domain_id,
        offer_id,
        selected_payload_type,
    ))
}

/// Validate a Subscribe data message against an accepted Subscribe start result.
#[wasm_bindgen(js_name = validateSubscribeDataMessage)]
pub fn validate_subscribe_data_message(
    accepted_start_result: JsValue,
    message: JsValue,
    actual_body_len: JsValue,
    max_message_bytes: JsValue,
) -> Result<JsValue, JsValue> {
    ok_value(validate_subscribe_data_message_value(
        js_to_value(accepted_start_result)?,
        js_to_value(message)?,
        js_to_optional_u64(actual_body_len, "subscribe_data")?,
        js_to_optional_u64(max_message_bytes, "subscribe_data")?,
    ))
}

/// Parse a protocol error object.
#[wasm_bindgen(js_name = parseErrorObject)]
pub fn parse_error_object(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_error_object_value(js_to_value(value)?))
}

/// Parse a status snapshot.
#[wasm_bindgen(js_name = parseStatusSnapshot)]
pub fn parse_status_snapshot(value: JsValue) -> Result<JsValue, JsValue> {
    ok_value(parse_status_snapshot_value(js_to_value(value)?))
}

fn protocol_constants_value() -> Value {
    json!({
        "version": "auki.protocol.v1",
        "lifecycle_version": CLUSTER_LIFECYCLE_V1,
        "stream_protocols": {
            "offer_catalog": OFFER_CATALOG_PROTOCOL_ID,
            "get": GET_PROTOCOL_ID,
            "subscribe": SUBSCRIBE_PROTOCOL_ID,
        },
        "failure_codes": {
            "protocol_unsupported_version": error::PROTOCOL_UNSUPPORTED_VERSION,
            "handshake_invalid_message": error::HANDSHAKE_INVALID_MESSAGE,
            "identity_invalid_peer_binding": error::IDENTITY_INVALID_PEER_BINDING,
            "identity_peer_id_mismatch": error::IDENTITY_PEER_ID_MISMATCH,
            "identity_invalid_signature": error::IDENTITY_INVALID_SIGNATURE,
            "domain_invalid_declaration": error::DOMAIN_INVALID_DECLARATION,
            "domain_id_mismatch": error::DOMAIN_ID_MISMATCH,
            "domain_missing_delegation": error::DOMAIN_MISSING_DELEGATION,
            "domain_invalid_delegation": error::DOMAIN_INVALID_DELEGATION,
            "domain_expired_delegation": error::DOMAIN_EXPIRED_DELEGATION,
            "offer_invalid_catalog_request": error::OFFER_INVALID_CATALOG_REQUEST,
            "offer_invalid_catalog_response": error::OFFER_INVALID_CATALOG_RESPONSE,
            "message_invalid_envelope": error::MESSAGE_INVALID_ENVELOPE,
            "message_invalid_payload": error::MESSAGE_INVALID_PAYLOAD,
            "message_payload_too_large": error::MESSAGE_PAYLOAD_TOO_LARGE,
            "get_invalid_request": error::GET_INVALID_REQUEST,
            "subscribe_invalid_request": error::SUBSCRIBE_INVALID_REQUEST,
            "transport_failed": error::TRANSPORT_FAILED,
        }
    })
}

fn encode_json_frame_value(value: &Value, max_body_len: u32) -> ProtocolResult<Vec<u8>> {
    frame::encode_json_frame(value, u64::from(max_body_len))
        .map_err(|error| ProtocolWasmError::new("frame", error))
}

fn decode_json_frame_value(input: &[u8], max_body_len: u32) -> ProtocolResult<Value> {
    frame::decode_json_frame(input, u64::from(max_body_len))
        .map(|(value, consumed)| json!({ "value": value, "consumed": consumed }))
        .map_err(|error| ProtocolWasmError::new("frame", error))
}

fn wallet_from_seed(seed: &[u8]) -> ProtocolResult<std::sync::Arc<Wallet>> {
    Wallet::from_seed(seed.to_vec()).map_err(|error| ProtocolWasmError::new("wallet_seed", error))
}

fn wallet_public_key_from_seed_value(seed: &[u8]) -> ProtocolResult<String> {
    let wallet = wallet_from_seed(seed)?;
    Ok(base64url::encode(&wallet.public_key().0))
}

fn parse_peer_id(peer_id: &str, kind: &'static str) -> ProtocolResult<PeerId> {
    PeerId::from_str(peer_id).map_err(|error| ProtocolWasmError::new(kind, error))
}

fn parse_wallet_public_key(
    encoded: &str,
    kind: &'static str,
    failure_code: &'static str,
) -> ProtocolResult<WalletPublicKey> {
    base64url::decode_exact::<32>(encoded)
        .map(WalletPublicKey)
        .map_err(|error| ProtocolWasmError::with_failure_code(kind, error, failure_code))
}

fn create_peer_binding_value(
    wallet_seed: &[u8],
    peer_id: &str,
    issued_at: &str,
    label: Option<&str>,
) -> ProtocolResult<Value> {
    let wallet = wallet_from_seed(wallet_seed)?;
    let peer_id = parse_peer_id(peer_id, "peer_id")?;
    PeerBinding::create(&wallet, &peer_id, issued_at, label)
        .map(PeerBinding::into_value)
        .map_err(peer_binding_error)
}

fn parse_peer_binding_value(value: Value) -> ProtocolResult<Value> {
    PeerBinding::from_value(value)
        .map(PeerBinding::into_value)
        .map_err(peer_binding_error)
}

fn verify_peer_binding_value(value: Value, authenticated_peer_id: &str) -> ProtocolResult<Value> {
    let peer_id = parse_peer_id(authenticated_peer_id, "peer_id")?;
    let binding = PeerBinding::from_value(value).map_err(peer_binding_error)?;
    binding
        .verify_for_peer_id(&peer_id)
        .map(|verified| {
            json!({
                "wallet_public_key": base64url::encode(&verified.wallet_public_key.0),
                "peer_id": verified.peer_id.to_string(),
                "issued_at": verified.issued_at,
                "label": verified.label,
            })
        })
        .map_err(peer_binding_error)
}

fn create_domain_declaration_value(
    owner_seed: &[u8],
    nonce: &[u8],
    label: Option<&str>,
) -> ProtocolResult<Value> {
    let wallet = wallet_from_seed(owner_seed)?;
    let nonce: [u8; DOMAIN_NONCE_LEN] = nonce.try_into().map_err(|_| {
        ProtocolWasmError::new(
            "domain_nonce",
            format!("domain nonce must be {DOMAIN_NONCE_LEN} bytes"),
        )
    })?;
    DomainDeclaration::create(&wallet, &nonce, label)
        .map(DomainDeclaration::into_value)
        .map_err(domain_declaration_error)
}

fn parse_domain_declaration_value(value: Value) -> ProtocolResult<Value> {
    DomainDeclaration::from_value(value)
        .map(DomainDeclaration::into_value)
        .map_err(domain_declaration_error)
}

fn verify_domain_declaration_value(value: Value) -> ProtocolResult<Value> {
    let declaration = DomainDeclaration::from_value(value).map_err(domain_declaration_error)?;
    declaration
        .verify()
        .map(|verified| {
            json!({
                "domain_id": verified.domain_id,
                "domain_owner_public_key": base64url::encode(&verified.domain_owner_public_key.0),
                "nonce": base64url::encode(&verified.nonce),
                "label": verified.label,
            })
        })
        .map_err(domain_declaration_error)
}

fn create_domain_delegation_value(owner_seed: &[u8], params: Value) -> ProtocolResult<Value> {
    let wallet = wallet_from_seed(owner_seed)?;
    let params = params_object(
        &params,
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let domain_id = required_param_string(
        params,
        "domain_id",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let delegate_wallet_public_key = required_param_string(
        params,
        "delegate_wallet_public_key",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let delegate_peer_id = required_param_string(
        params,
        "delegate_peer_id",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let scopes = required_param_string_array(
        params,
        "scopes",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let valid_from = required_param_string(
        params,
        "valid_from",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let expires_at = required_param_string(
        params,
        "expires_at",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let label = optional_param_string(
        params,
        "label",
        "domain_delegation",
        error::DOMAIN_INVALID_DELEGATION,
    )?;

    let delegate_wallet_public_key = parse_wallet_public_key(
        delegate_wallet_public_key,
        "delegate_wallet_public_key",
        error::DOMAIN_INVALID_DELEGATION,
    )?;
    let delegate_peer_id = parse_peer_id(delegate_peer_id, "delegate_peer_id")?;
    let scopes = scopes
        .iter()
        .map(|scope| {
            DelegationScope::from_str(scope)
                .map_err(|error| domain_delegation_error_with_kind("delegation_scopes", error))
        })
        .collect::<ProtocolResult<Vec<_>>>()?;

    DomainDelegation::create(
        &wallet,
        DomainDelegationParams {
            domain_id,
            delegate_wallet_public_key: &delegate_wallet_public_key,
            delegate_peer_id: &delegate_peer_id,
            scopes: &scopes,
            valid_from,
            expires_at,
            label,
        },
    )
    .map(DomainDelegation::into_value)
    .map_err(domain_delegation_error)
}

fn params_object<'a>(
    value: &'a Value,
    kind: &'static str,
    failure_code: &'static str,
) -> ProtocolResult<&'a Map<String, Value>> {
    value.as_object().ok_or_else(|| {
        ProtocolWasmError::with_failure_code(kind, "params must be an object", failure_code)
    })
}

fn required_param_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    kind: &'static str,
    failure_code: &'static str,
) -> ProtocolResult<&'a str> {
    object
        .get(field)
        .ok_or_else(|| {
            ProtocolWasmError::with_failure_code(
                kind,
                format!("params missing {field}"),
                failure_code,
            )
        })?
        .as_str()
        .ok_or_else(|| {
            ProtocolWasmError::with_failure_code(
                kind,
                format!("params field {field} must be a string"),
                failure_code,
            )
        })
}

fn optional_param_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
    kind: &'static str,
    failure_code: &'static str,
) -> ProtocolResult<Option<&'a str>> {
    object
        .get(field)
        .map(|value| {
            value.as_str().ok_or_else(|| {
                ProtocolWasmError::with_failure_code(
                    kind,
                    format!("params field {field} must be a string"),
                    failure_code,
                )
            })
        })
        .transpose()
}

fn required_param_string_array(
    object: &Map<String, Value>,
    field: &'static str,
    kind: &'static str,
    failure_code: &'static str,
) -> ProtocolResult<Vec<String>> {
    let values = object
        .get(field)
        .ok_or_else(|| {
            ProtocolWasmError::with_failure_code(
                kind,
                format!("params missing {field}"),
                failure_code,
            )
        })?
        .as_array()
        .ok_or_else(|| {
            ProtocolWasmError::with_failure_code(
                kind,
                format!("params field {field} must be an array"),
                failure_code,
            )
        })?;

    values
        .iter()
        .map(|value| {
            value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                ProtocolWasmError::with_failure_code(
                    kind,
                    format!("params field {field} must be an array of strings"),
                    failure_code,
                )
            })
        })
        .collect()
}

fn parse_domain_delegation_value(value: Value) -> ProtocolResult<Value> {
    DomainDelegation::from_value(value)
        .map(DomainDelegation::into_value)
        .map_err(domain_delegation_error)
}

fn verify_domain_delegation_value(value: Value) -> ProtocolResult<Value> {
    let delegation = DomainDelegation::from_value(value).map_err(domain_delegation_error)?;
    delegation
        .verify()
        .map(|verified| {
            json!({
                "domain_id": verified.domain_id,
                "domain_owner_public_key": base64url::encode(&verified.domain_owner_public_key.0),
                "delegate_wallet_public_key": base64url::encode(&verified.delegate_wallet_public_key.0),
                "delegate_peer_id": verified.delegate_peer_id.to_string(),
                "scopes": verified.scopes.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
                "valid_from": verified.valid_from,
                "expires_at": verified.expires_at,
                "label": verified.label,
            })
        })
        .map_err(domain_delegation_error)
}

fn parse_offer_catalog_path_value(value: Value) -> ProtocolResult<Value> {
    OfferCatalogPath::from_value(value)
        .map(OfferCatalogPath::into_value)
        .map_err(|error| ProtocolWasmError::new("offer_catalog_path", error))
}

fn create_peer_handshake_value(
    peer_binding: Value,
    declared_domains: Value,
    offer_catalog: Option<Value>,
) -> ProtocolResult<Value> {
    let peer_binding = PeerBinding::from_value(peer_binding).map_err(peer_binding_error)?;
    let declared_domains = parse_declared_domain_array(declared_domains)?;
    let handshake = if let Some(offer_catalog) = offer_catalog {
        let offer_catalog = OfferCatalogPath::from_value(offer_catalog).map_err(|error| {
            ProtocolWasmError::with_failure_code(
                "offer_catalog_path",
                error,
                error::HANDSHAKE_INVALID_MESSAGE,
            )
        })?;
        PeerHandshake::create_with_offer_catalog(peer_binding, declared_domains, offer_catalog)
    } else {
        PeerHandshake::create(peer_binding, declared_domains)
    };
    Ok(handshake.into_value())
}

fn parse_declared_domain_array(value: Value) -> ProtocolResult<Vec<DeclaredDomain>> {
    let values = value.as_array().ok_or_else(|| {
        ProtocolWasmError::with_failure_code(
            "declared_domains",
            "declared domains must be an array",
            error::HANDSHAKE_INVALID_MESSAGE,
        )
    })?;
    values
        .iter()
        .cloned()
        .map(|value| {
            DeclaredDomain::from_value(value).map_err(|error| {
                ProtocolWasmError::with_failure_code(
                    "declared_domain",
                    error,
                    error::HANDSHAKE_INVALID_MESSAGE,
                )
            })
        })
        .collect()
}

fn parse_peer_handshake_value(value: Value) -> ProtocolResult<Value> {
    PeerHandshake::from_value(value)
        .map(PeerHandshake::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("peer_handshake", error, failure_code)
        })
}

fn validate_peer_handshake_authority_value(
    handshake: Value,
    authenticated_peer_id: &str,
    peer_authorized: bool,
    now: &str,
) -> ProtocolResult<Value> {
    let authenticated_peer_id = parse_peer_id(authenticated_peer_id, "peer_id")?;
    let handshake = PeerHandshake::from_value(handshake).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("peer_handshake", error, failure_code)
    })?;
    let peer_authorization = if peer_authorized {
        PeerAuthorization::Authorized
    } else {
        PeerAuthorization::Rejected
    };
    let authority = handshake
        .validate_authority(&authenticated_peer_id, peer_authorization, now)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("authority_chain", error, failure_code)
        })?;

    Ok(json!({
        "peer": {
            "wallet_public_key": base64url::encode(&authority.peer.wallet_public_key.0),
            "peer_id": authority.peer.peer_id.to_string(),
            "issued_at": authority.peer.issued_at,
            "label": authority.peer.label,
        },
        "accepted_served_domains": authority.accepted_served_domains.iter().map(|accepted| {
            json!({
                "domain_id": accepted.domain_id,
                "authority": match accepted.authority {
                    ServedDomainAuthority::DirectOwner => "direct_owner",
                    ServedDomainAuthority::Delegated => "delegated",
                },
            })
        }).collect::<Vec<_>>(),
        "rejected_declared_domains": authority.rejected_declared_domains.iter().map(|rejected| {
            json!({
                "domain_id": rejected.domain_id,
                "failure_code": rejected.failure_code,
                "reason": format!("{:?}", rejected.reason),
            })
        }).collect::<Vec<_>>(),
    }))
}

fn parse_offer_catalog_request_value(value: Value) -> ProtocolResult<Value> {
    OfferCatalogRequest::from_value(value)
        .map(OfferCatalogRequest::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("offer_catalog_request", error, failure_code)
        })
}

fn parse_offer_catalog_response_value(value: Value) -> ProtocolResult<Value> {
    OfferCatalogResponse::from_value(value)
        .map(OfferCatalogResponse::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("offer_catalog_response", error, failure_code)
        })
}

fn parse_get_request_value(value: Value) -> ProtocolResult<Value> {
    GetRequest::from_value(value)
        .map(GetRequest::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("get_request", error, failure_code)
        })
}

fn parse_get_response_value(value: Value) -> ProtocolResult<Value> {
    GetResponse::from_value(value)
        .map(GetResponse::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("get_response", error, failure_code)
        })
}

fn validate_get_response_for_request_value(
    request: Value,
    response: Value,
    selected_payload_type: &str,
) -> ProtocolResult<Value> {
    let request = GetRequest::from_value(request).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("get_request", error, failure_code)
    })?;
    let response = GetResponse::from_value(response).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("get_response", error, failure_code)
    })?;
    let message = response
        .validate_success_for_request(&request, selected_payload_type)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("get_response", error, failure_code)
        })?;
    Ok(message.value().clone())
}

fn parse_subscribe_request_value(value: Value) -> ProtocolResult<Value> {
    SubscribeRequest::from_value(value)
        .map(SubscribeRequest::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_request", error, failure_code)
        })
}

fn parse_subscribe_start_result_value(value: Value) -> ProtocolResult<Value> {
    SubscribeStartResult::from_value(value)
        .map(SubscribeStartResult::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_start_result", error, failure_code)
        })
}

fn validate_subscribe_start_for_request_value(
    request: Value,
    start_result: Value,
) -> ProtocolResult<Value> {
    let request = SubscribeRequest::from_value(request).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("subscribe_request", error, failure_code)
    })?;
    let start = SubscribeStartResult::from_value(start_result).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("subscribe_start_result", error, failure_code)
    })?;

    if let Some(accept) = start.accept_body() {
        accept.validate_for_request(&request).map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_accept", error, failure_code)
        })?;
        return Ok(json!({ "accepted": true, "accept": accept.value().clone() }));
    }

    let reject = start
        .reject_body()
        .expect("parsed Subscribe start result is accept or reject");
    Ok(json!({ "accepted": false, "reject": reject.value().clone() }))
}

fn parse_subscribe_end_value(value: Value) -> ProtocolResult<Value> {
    SubscribeEnd::from_value(value)
        .map(SubscribeEnd::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_end", error, failure_code)
        })
}

fn validate_subscribe_end_for_offer_value(
    end: Value,
    domain_id: &str,
    offer_id: &str,
) -> ProtocolResult<Value> {
    let end = SubscribeEnd::from_value(end).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("subscribe_end", error, failure_code)
    })?;
    end.validate_for_offer(domain_id, offer_id)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("subscribe_end", error, failure_code)
        })?;
    Ok(end.into_value())
}

fn parse_spatial_message_value(value: Value) -> ProtocolResult<Value> {
    SpatialMessage::from_value(value)
        .map(SpatialMessage::into_value)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("spatial_message", error, failure_code)
        })
}

fn validate_spatial_message_for_offer_value(
    message: Value,
    domain_id: &str,
    offer_id: &str,
    selected_payload_type: &str,
) -> ProtocolResult<Value> {
    let message = SpatialMessage::from_value(message).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("spatial_message", error, failure_code)
    })?;
    message
        .validate_for_offer(domain_id, offer_id, selected_payload_type)
        .map_err(|error| {
            let failure_code = error.failure_code();
            ProtocolWasmError::with_failure_code("spatial_message", error, failure_code)
        })?;
    Ok(message.into_value())
}

fn validate_subscribe_data_message_value(
    accepted_start_result: Value,
    message: Value,
    actual_body_len: Option<u64>,
    max_message_bytes: Option<u64>,
) -> ProtocolResult<Value> {
    let start = SubscribeStartResult::from_value(accepted_start_result).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("subscribe_start_result", error, failure_code)
    })?;
    let accept = start.accept_body().ok_or_else(|| {
        ProtocolWasmError::with_failure_code(
            "subscribe_start_result",
            "Subscribe start result is not an accept result",
            error::MESSAGE_INVALID_ENVELOPE,
        )
    })?;
    let message = SpatialMessage::from_value(message).map_err(|error| {
        let failure_code = error.failure_code();
        ProtocolWasmError::with_failure_code("spatial_message", error, failure_code)
    })?;

    if let Some(actual_body_len) = actual_body_len {
        let actual_body_len = usize::try_from(actual_body_len).map_err(|_| {
            ProtocolWasmError::with_failure_code(
                "subscribe_data",
                "actual body length exceeds local usize range",
                error::MESSAGE_PAYLOAD_TOO_LARGE,
            )
        })?;
        accept
            .validate_data_message_with_body_len(&message, actual_body_len, max_message_bytes)
            .map_err(|error| {
                let failure_code = error.failure_code();
                ProtocolWasmError::with_failure_code("subscribe_data", error, failure_code)
            })?;
    } else {
        accept
            .validate_data_message(&message, max_message_bytes)
            .map_err(|error| {
                let failure_code = error.failure_code();
                ProtocolWasmError::with_failure_code("subscribe_data", error, failure_code)
            })?;
    }

    Ok(message.into_value())
}

fn parse_error_object_value(value: Value) -> ProtocolResult<Value> {
    ErrorObject::from_value(value)
        .map(ErrorObject::into_value)
        .map_err(|error| ProtocolWasmError::new("error_object", error))
}

fn parse_status_snapshot_value(value: Value) -> ProtocolResult<Value> {
    StatusSnapshot::from_value(value)
        .map(StatusSnapshot::into_value)
        .map_err(|error| ProtocolWasmError::new("status_snapshot", error))
}

fn peer_binding_error(error: PeerBindingError) -> ProtocolWasmError {
    let failure_code = match &error {
        PeerBindingError::InvalidSignature => error::IDENTITY_INVALID_SIGNATURE,
        PeerBindingError::PeerIdMismatch { .. } => error::IDENTITY_PEER_ID_MISMATCH,
        _ => error::IDENTITY_INVALID_PEER_BINDING,
    };
    ProtocolWasmError::with_failure_code("peer_binding", error, failure_code)
}

fn domain_declaration_error(error: DomainError) -> ProtocolWasmError {
    let failure_code = match &error {
        DomainError::DomainIdMismatch { .. } => error::DOMAIN_ID_MISMATCH,
        _ => error::DOMAIN_INVALID_DECLARATION,
    };
    ProtocolWasmError::with_failure_code("domain_declaration", error, failure_code)
}

fn domain_delegation_error(error: DomainError) -> ProtocolWasmError {
    domain_delegation_error_with_kind("domain_delegation", error)
}

fn domain_delegation_error_with_kind(kind: &'static str, error: DomainError) -> ProtocolWasmError {
    let failure_code = match &error {
        DomainError::DelegationExpired { .. } => error::DOMAIN_EXPIRED_DELEGATION,
        _ => error::DOMAIN_INVALID_DELEGATION,
    };
    ProtocolWasmError::with_failure_code(kind, error, failure_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1_GET_VECTORS: &str = include_str!("../../auki-protocol/vectors/v1_get.json");
    const V1_HANDSHAKE_VECTORS: &str =
        include_str!("../../auki-protocol/vectors/v1_handshakes.json");
    const V1_JSON_FRAME_VECTORS: &str =
        include_str!("../../auki-protocol/vectors/v1_json_frames.json");
    const V1_OFFER_CATALOG_VECTORS: &str =
        include_str!("../../auki-protocol/vectors/v1_offer_catalogs.json");
    const V1_STATUS_VECTORS: &str = include_str!("../../auki-protocol/vectors/v1_status.json");
    const V1_SUBSCRIBE_VECTORS: &str =
        include_str!("../../auki-protocol/vectors/v1_subscribe.json");

    const PEER_ID: &str = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
    const ISSUED_AT: &str = "2026-05-26T12:00:00Z";
    const VALID_FROM: &str = "2026-05-26T00:00:00Z";
    const EXPIRES_AT: &str = "2026-05-27T00:00:00Z";

    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).expect("valid vector fixture")
    }

    fn positive_object<'a>(fixture: &'a Value, key: &str) -> &'a Value {
        &fixture["positive"][key]["object"]
    }

    fn negative_object<'a>(fixture: &'a Value, key: &str) -> &'a Value {
        &fixture["negative"][key]["object"]
    }

    fn input<'a>(fixture: &'a Value, key: &str) -> &'a str {
        fixture["inputs"][key].as_str().expect("input string")
    }

    fn bytes_from_hex(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0, "hex string must have an even length");
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex byte"))
            .collect()
    }

    #[test]
    fn frame_vectors_roundtrip_through_protocol_adapter() {
        let fixture = fixture(V1_JSON_FRAME_VECTORS);
        for vector in fixture["vectors"].as_array().expect("vectors array") {
            let body = vector["body_utf8"].as_str().expect("body_utf8 string");
            let body_len = vector["body_len"].as_u64().expect("body_len u64") as u32;
            let frame = bytes_from_hex(vector["frame_hex"].as_str().expect("frame hex"));
            let value: Value = serde_json::from_str(body).expect("valid JSON body");

            assert_eq!(encode_json_frame_value(&value, body_len).unwrap(), frame);

            let decoded = decode_json_frame_value(&frame, body_len).unwrap();
            assert_eq!(decoded["value"], value);
            assert_eq!(decoded["consumed"], Value::from(frame.len() as u64));
        }
    }

    #[test]
    fn vector_parsers_delegate_to_auki_protocol() {
        let handshakes = fixture(V1_HANDSHAKE_VECTORS);
        assert_eq!(
            parse_peer_handshake_value(
                positive_object(&handshakes, "delegated_serving_peer").clone()
            )
            .unwrap(),
            positive_object(&handshakes, "delegated_serving_peer").clone()
        );
        assert_eq!(
            parse_peer_handshake_value(
                negative_object(&handshakes, "missing_required_lifecycle_version").clone()
            )
            .unwrap_err()
            .failure_code,
            Some(error::PROTOCOL_UNSUPPORTED_VERSION)
        );

        let offers = fixture(V1_OFFER_CATALOG_VECTORS);
        assert_eq!(
            parse_offer_catalog_request_value(positive_object(&offers, "filtered_request").clone())
                .unwrap(),
            positive_object(&offers, "filtered_request").clone()
        );
        assert_eq!(
            parse_offer_catalog_response_value(
                positive_object(&offers, "response_with_offer").clone()
            )
            .unwrap(),
            positive_object(&offers, "response_with_offer").clone()
        );
        assert_eq!(
            parse_offer_catalog_request_value(
                negative_object(&offers, "request_invalid_domain_id").clone()
            )
            .unwrap_err()
            .failure_code,
            Some(error::OFFER_INVALID_CATALOG_REQUEST)
        );

        let get = fixture(V1_GET_VECTORS);
        assert_eq!(
            parse_get_request_value(positive_object(&get, "request").clone()).unwrap(),
            positive_object(&get, "request").clone()
        );
        assert_eq!(
            parse_get_response_value(positive_object(&get, "success_response").clone()).unwrap(),
            positive_object(&get, "success_response").clone()
        );
        assert_eq!(
            parse_get_request_value(negative_object(&get, "request_invalid_domain_id").clone())
                .unwrap_err()
                .failure_code,
            Some(error::GET_INVALID_REQUEST)
        );

        let subscribe = fixture(V1_SUBSCRIBE_VECTORS);
        assert_eq!(
            parse_subscribe_request_value(positive_object(&subscribe, "request").clone()).unwrap(),
            positive_object(&subscribe, "request").clone()
        );
        assert_eq!(
            parse_subscribe_start_result_value(
                positive_object(&subscribe, "accept_start_result").clone()
            )
            .unwrap(),
            positive_object(&subscribe, "accept_start_result").clone()
        );
        assert_eq!(
            parse_spatial_message_value(positive_object(&subscribe, "data_message").clone())
                .unwrap(),
            positive_object(&subscribe, "data_message").clone()
        );
        assert_eq!(
            parse_subscribe_end_value(positive_object(&subscribe, "end_message").clone()).unwrap(),
            positive_object(&subscribe, "end_message").clone()
        );
        assert_eq!(
            parse_subscribe_request_value(
                negative_object(&subscribe, "request_invalid_domain_id").clone()
            )
            .unwrap_err()
            .failure_code,
            Some(error::SUBSCRIBE_INVALID_REQUEST)
        );

        let status = fixture(V1_STATUS_VECTORS);
        assert_eq!(
            parse_status_snapshot_value(positive_object(&status, "full_snapshot").clone()).unwrap(),
            positive_object(&status, "full_snapshot").clone()
        );
        assert_eq!(
            parse_status_snapshot_value(negative_object(&status, "unsupported_type").clone())
                .unwrap_err()
                .kind,
            "status_snapshot"
        );
    }

    #[test]
    fn request_bound_validators_delegate_to_auki_protocol() {
        let handshakes = fixture(V1_HANDSHAKE_VECTORS);
        let authority = validate_peer_handshake_authority_value(
            positive_object(&handshakes, "delegated_serving_peer").clone(),
            input(&handshakes, "delegate_peer_id"),
            true,
            input(&handshakes, "verification_now"),
        )
        .unwrap();
        assert_eq!(
            authority["accepted_served_domains"][0]["domain_id"],
            input(&handshakes, "domain_id")
        );

        let get = fixture(V1_GET_VECTORS);
        assert_eq!(
            validate_get_response_for_request_value(
                positive_object(&get, "request").clone(),
                positive_object(&get, "success_response").clone(),
                input(&get, "selected_payload_type"),
            )
            .unwrap(),
            positive_object(&get, "success_response")["message"].clone()
        );
        assert_eq!(
            validate_get_response_for_request_value(
                positive_object(&get, "request").clone(),
                negative_object(&get, "response_offer_mismatch").clone(),
                input(&get, "selected_payload_type"),
            )
            .unwrap_err()
            .failure_code,
            Some(error::MESSAGE_INVALID_ENVELOPE)
        );

        let subscribe = fixture(V1_SUBSCRIBE_VECTORS);
        let start = validate_subscribe_start_for_request_value(
            positive_object(&subscribe, "request").clone(),
            positive_object(&subscribe, "accept_start_result").clone(),
        )
        .unwrap();
        assert_eq!(start["accepted"], true);
        assert_eq!(
            validate_subscribe_start_for_request_value(
                positive_object(&subscribe, "request").clone(),
                negative_object(&subscribe, "accept_offer_mismatch").clone(),
            )
            .unwrap_err()
            .failure_code,
            Some(error::MESSAGE_INVALID_ENVELOPE)
        );

        assert_eq!(
            validate_subscribe_data_message_value(
                positive_object(&subscribe, "accept_start_result").clone(),
                positive_object(&subscribe, "data_message").clone(),
                None,
                None,
            )
            .unwrap(),
            positive_object(&subscribe, "data_message").clone()
        );
        assert_eq!(
            validate_subscribe_data_message_value(
                positive_object(&subscribe, "accept_start_result").clone(),
                negative_object(&subscribe, "data_message_payload_type_mismatch").clone(),
                None,
                None,
            )
            .unwrap_err()
            .failure_code,
            Some(error::MESSAGE_INVALID_PAYLOAD)
        );

        assert_eq!(
            validate_subscribe_end_for_offer_value(
                positive_object(&subscribe, "end_message").clone(),
                input(&subscribe, "domain_id"),
                input(&subscribe, "offer_id"),
            )
            .unwrap(),
            positive_object(&subscribe, "end_message").clone()
        );
    }

    #[test]
    fn authority_constructors_are_protocol_backed() {
        let owner_seed = [3u8; 32];
        let delegate_seed = [4u8; 32];
        let nonce = [7u8; DOMAIN_NONCE_LEN];

        let binding =
            create_peer_binding_value(&owner_seed, PEER_ID, ISSUED_AT, Some("browser")).unwrap();
        let verified_binding = verify_peer_binding_value(binding.clone(), PEER_ID).unwrap();
        assert_eq!(verified_binding["peer_id"], PEER_ID);
        assert_eq!(
            parse_peer_binding_value(binding.clone()).unwrap()["type"],
            "auki.peer_binding.v1"
        );

        let declaration =
            create_domain_declaration_value(&owner_seed, &nonce, Some("demo-domain")).unwrap();
        let verified_declaration = verify_domain_declaration_value(declaration.clone()).unwrap();
        let domain_id = verified_declaration["domain_id"].as_str().unwrap();

        let delegate_public_key = wallet_public_key_from_seed_value(&delegate_seed).unwrap();
        let delegation = create_domain_delegation_value(
            &owner_seed,
            json!({
                "domain_id": domain_id,
                "delegate_wallet_public_key": delegate_public_key,
                "delegate_peer_id": PEER_ID,
                "scopes": ["advertise", "serve"],
                "valid_from": VALID_FROM,
                "expires_at": EXPIRES_AT,
                "label": "sentinel",
            }),
        )
        .unwrap();
        let verified_delegation = verify_domain_delegation_value(delegation).unwrap();

        assert_eq!(verified_delegation["domain_id"], domain_id);
        assert_eq!(verified_delegation["delegate_peer_id"], PEER_ID);
        assert_eq!(verified_delegation["scopes"], json!(["advertise", "serve"]));
    }

    #[test]
    fn handshake_constructor_uses_protocol_types() {
        let seed = [3u8; 32];
        let nonce = [7u8; DOMAIN_NONCE_LEN];
        let binding = create_peer_binding_value(&seed, PEER_ID, ISSUED_AT, None).unwrap();
        let declaration = create_domain_declaration_value(&seed, &nonce, None).unwrap();
        let domain_id = verify_domain_declaration_value(declaration.clone()).unwrap()["domain_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let declared_domains = json!([{
            "domain_id": domain_id,
            "domain_declaration": declaration,
        }]);
        let offer_catalog = OfferCatalogPath::create(None).unwrap().into_value();

        let handshake =
            create_peer_handshake_value(binding, declared_domains, Some(offer_catalog)).unwrap();
        assert_eq!(
            parse_peer_handshake_value(handshake.clone()).unwrap(),
            handshake
        );
        assert_eq!(handshake["type"], "auki.peer_handshake.v1");
        assert_eq!(
            handshake["supported_lifecycle_versions"],
            json!([CLUSTER_LIFECYCLE_V1])
        );
    }
}
