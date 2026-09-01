//! Feature-gated Swift adapters over the SDK-owned portable Rust protocols.

#[cfg(feature = "blob")]
mod blob;
#[cfg(feature = "catalog")]
mod catalog;
#[cfg(any(
    feature = "info",
    feature = "catalog",
    feature = "registry",
    feature = "blob"
))]
mod finite_support;
#[cfg(feature = "info")]
mod info;
#[cfg(feature = "message")]
mod message;
#[cfg(feature = "registry")]
mod registry;
#[cfg(feature = "standard-protocols")]
mod standard;
#[cfg(feature = "stream")]
mod stream;
