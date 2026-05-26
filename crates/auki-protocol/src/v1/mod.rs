//! Version 1 protocol types and helpers.

/// Canonical base64url-without-padding helpers.
pub mod base64url;
/// Stable v1 failure-code constants.
pub mod error;
/// V1 JSON frame encoding and decoding.
pub mod frame;
/// Peer identity authority objects.
pub mod identity;
/// Strict JSON parsing helpers.
pub mod json;
