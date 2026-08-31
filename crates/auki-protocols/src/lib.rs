//! Transport-neutral wire contracts for authenticated Auki protocols.
//!
//! Every protocol family is compile-time opt-in and the default feature set is
//! empty. Exact wire versions are explicit in module paths, for example
//! `catalog::v3` and `stream::v2`. Hosting, registration, authorization,
//! providers, and task lifecycle belong to the runtime using this crate.

#![forbid(unsafe_code)]

pub mod ids;

#[cfg(any(
    feature = "blob-endpoint",
    feature = "catalog-endpoint",
    feature = "info-endpoint",
    feature = "message-endpoint",
    feature = "registry-endpoint",
    feature = "stream-endpoint",
))]
mod endpoint_support;

#[cfg(feature = "blob")]
pub mod blob;
#[cfg(feature = "catalog")]
pub mod catalog;
#[cfg(feature = "info")]
pub mod info;
#[cfg(any(feature = "catalog", feature = "message"))]
pub mod message;
#[cfg(feature = "registry")]
pub mod registry;
#[cfg(all(feature = "session-adapter", not(target_arch = "wasm32")))]
pub mod session_adapter;
#[cfg(feature = "stream")]
pub mod stream;

/// Exact protocol identifiers compiled into this crate instance.
///
/// This reports compile-time wire support only. A runtime must still opt in to
/// serving each exact protocol version.
pub const SUPPORTED_IDS: &[&str] = &[
    #[cfg(feature = "info")]
    info::v1::ID,
    #[cfg(feature = "catalog")]
    catalog::v2::ID,
    #[cfg(feature = "catalog")]
    catalog::v3::ID,
    #[cfg(feature = "catalog")]
    catalog::v4::ID,
    #[cfg(feature = "registry")]
    registry::v2::ID,
    #[cfg(feature = "registry")]
    registry::v3::ID,
    #[cfg(feature = "blob")]
    blob::v1::ID,
    #[cfg(feature = "message")]
    message::v1::ID,
    #[cfg(feature = "stream")]
    stream::v2::ID,
];
