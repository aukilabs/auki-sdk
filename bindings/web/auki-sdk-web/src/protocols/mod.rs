#[cfg(feature = "blob")]
mod blob;
#[cfg(feature = "catalog")]
mod catalog;
#[cfg(feature = "info")]
mod info;
#[cfg(feature = "message")]
mod message;
#[cfg(feature = "registry")]
mod registry;
#[cfg(feature = "stream")]
mod stream;

#[cfg(feature = "blob")]
pub use blob::AukiBlobClient;
#[cfg(feature = "catalog")]
pub use catalog::AukiCatalogClient;
#[cfg(feature = "info")]
pub use info::AukiInfoClient;
#[cfg(feature = "message")]
pub use message::{AukiMessageClient, AukiMessageEndpoint, AukiMessageReceiver, AukiMessageSender};
#[cfg(feature = "registry")]
pub use registry::AukiRegistryClient;
#[cfg(feature = "stream")]
pub use stream::{AukiStreamClient, AukiStreamSubscription};
