//! Pure protocol crate for the RFC-first Auki networking path.
//!
//! This crate owns deterministic protocol behavior and validation helpers. It
//! intentionally avoids libp2p runtime, tokio task, Discovery-client, and
//! application-lifecycle concerns.

#![warn(missing_docs)]

/// Version 1 protocol types and helpers.
pub mod v1;
