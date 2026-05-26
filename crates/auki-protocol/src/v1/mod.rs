//! Version 1 protocol types and helpers.

/// Authority-chain validation helpers.
pub mod authority;
/// Canonical base64url-without-padding helpers.
pub mod base64url;
/// Domain authority helpers.
pub mod domain;
/// Stable v1 failure-code constants.
pub mod error;
/// V1 JSON frame encoding and decoding.
pub mod frame;
/// Get protocol helpers.
pub mod get;
/// V1 peer handshake message helpers.
pub mod handshake;
/// Peer identity authority objects.
pub mod identity;
/// Strict JSON parsing helpers.
pub mod json;
/// Spatial message and error-object helpers.
pub mod message;
/// Offer-catalog protocol helpers.
pub mod offer;
