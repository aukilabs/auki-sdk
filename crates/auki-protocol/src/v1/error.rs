//! Stable v1 failure-code constants.
//!
//! These string values mirror the baseline RFC failure-code registry.

/// Protocol version is not supported by the receiver.
pub const PROTOCOL_UNSUPPORTED_VERSION: &str = "protocol.unsupported_version";
/// Handshake frame or message is malformed.
pub const HANDSHAKE_INVALID_MESSAGE: &str = "handshake.invalid_message";
/// Required handshake material is missing.
pub const HANDSHAKE_MISSING_REQUIRED_MATERIAL: &str = "handshake.missing_required_material";
/// Peer binding is missing.
pub const IDENTITY_MISSING_PEER_BINDING: &str = "identity.missing_peer_binding";
/// Peer binding is malformed or invalid.
pub const IDENTITY_INVALID_PEER_BINDING: &str = "identity.invalid_peer_binding";
/// Peer binding peer id does not match the transport-authenticated peer id.
pub const IDENTITY_PEER_ID_MISMATCH: &str = "identity.peer_id_mismatch";
/// Signature verification failed.
pub const IDENTITY_INVALID_SIGNATURE: &str = "identity.invalid_signature";
/// Peer binding is too old under local freshness policy.
pub const IDENTITY_BINDING_TOO_OLD: &str = "identity.binding_too_old";
/// Peer binding was issued too far in the future under local policy.
pub const IDENTITY_BINDING_FROM_FUTURE: &str = "identity.binding_from_future";
/// Domain declaration is malformed or invalid.
pub const DOMAIN_INVALID_DECLARATION: &str = "domain.invalid_declaration";
/// Domain id does not match recomputed or nested domain id.
pub const DOMAIN_ID_MISMATCH: &str = "domain.id_mismatch";
/// Required domain delegation is missing.
pub const DOMAIN_MISSING_DELEGATION: &str = "domain.missing_delegation";
/// Domain delegation is malformed or invalid.
pub const DOMAIN_INVALID_DELEGATION: &str = "domain.invalid_delegation";
/// Domain delegation has expired.
pub const DOMAIN_EXPIRED_DELEGATION: &str = "domain.expired_delegation";
/// Peer was rejected by peer authorization policy.
pub const AUTHORIZATION_PEER_REJECTED: &str = "authorization.peer_rejected";
/// Domain was rejected by local domain policy.
pub const POLICY_DOMAIN_REJECTED: &str = "policy.domain_rejected";
/// Requested offer is unknown.
pub const OFFER_UNKNOWN_OFFER: &str = "offer.unknown_offer";
/// Offer domain is not in the accepted served-domain set.
pub const OFFER_DOMAIN_NOT_SERVED: &str = "offer.domain_not_served";
/// Offer kind is unsupported.
pub const OFFER_UNSUPPORTED_KIND: &str = "offer.unsupported_kind";
/// Offer access mode is unsupported.
pub const OFFER_UNSUPPORTED_ACCESS_MODE: &str = "offer.unsupported_access_mode";
/// Offer payload type is unsupported.
pub const OFFER_UNSUPPORTED_PAYLOAD_TYPE: &str = "offer.unsupported_payload_type";
/// Offer-catalog request is malformed.
pub const OFFER_INVALID_CATALOG_REQUEST: &str = "offer.invalid_catalog_request";
/// Offer-catalog response is malformed.
pub const OFFER_INVALID_CATALOG_RESPONSE: &str = "offer.invalid_catalog_response";
/// Individual offer is malformed.
pub const OFFER_INVALID_OFFER: &str = "offer.invalid_offer";
/// Offer catalog is temporarily unavailable.
pub const OFFER_CATALOG_UNAVAILABLE: &str = "offer.catalog_unavailable";
/// Offer is temporarily unavailable.
pub const OFFER_TEMPORARILY_UNAVAILABLE: &str = "offer.temporarily_unavailable";
/// Offer is stale under local freshness policy.
pub const OFFER_STALE: &str = "offer.stale";
/// Spatial message envelope is malformed.
pub const MESSAGE_INVALID_ENVELOPE: &str = "message.invalid_envelope";
/// Spatial message payload is malformed.
pub const MESSAGE_INVALID_PAYLOAD: &str = "message.invalid_payload";
/// Message or frame payload is too large.
pub const MESSAGE_PAYLOAD_TOO_LARGE: &str = "message.payload_too_large";
/// Sequence gap was observed.
pub const MESSAGE_SEQUENCE_GAP: &str = "message.sequence_gap";
/// Get request is malformed.
pub const GET_INVALID_REQUEST: &str = "get.invalid_request";
/// Subscribe request is malformed.
pub const SUBSCRIBE_INVALID_REQUEST: &str = "subscribe.invalid_request";
/// Subscribe stream was closed because the consumer could not keep up.
pub const SUBSCRIBE_BACKPRESSURE: &str = "subscribe.backpressure";
/// Offer loading failed.
pub const OFFER_LOAD_FAILED: &str = "offer.load_failed";
/// Transport or framing failed before a structured failure could be returned.
pub const TRANSPORT_FAILED: &str = "transport.failed";
