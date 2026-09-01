#[cfg(feature = "blob")]
mod blob;
#[cfg(feature = "catalog")]
mod catalog;
#[cfg(feature = "info")]
mod info;
#[cfg(feature = "registry")]
mod registry;

#[cfg(feature = "blob")]
pub use blob::AukiBlobClient;
#[cfg(feature = "catalog")]
pub use catalog::AukiCatalogClient;
#[cfg(feature = "info")]
pub use info::AukiInfoClient;
#[cfg(feature = "registry")]
pub use registry::AukiRegistryClient;
