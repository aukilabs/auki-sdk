//! Python adapters for the SDK-owned portable protocols.

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
mod support;

#[cfg(all(
    test,
    feature = "info",
    feature = "blob",
    feature = "message",
    feature = "stream"
))]
mod transport_tests;

use pyo3::{prelude::*, types::PyModule};

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(feature = "info")]
    info::register(module)?;
    #[cfg(feature = "catalog")]
    catalog::register(module)?;
    #[cfg(feature = "registry")]
    registry::register(module)?;
    #[cfg(feature = "blob")]
    blob::register(module)?;
    #[cfg(feature = "message")]
    message::register(module)?;
    #[cfg(feature = "stream")]
    stream::register(module)?;
    Ok(())
}

#[cfg(all(test, feature = "standard-protocols"))]
mod tests {
    use super::*;

    #[test]
    fn module_exposes_finite_and_message_clients_and_endpoints() {
        Python::with_gil(|py| {
            let module = PyModule::new_bound(py, "auki_sdk").unwrap();
            register(&module).unwrap();
            for name in [
                "AukiInfoClient",
                "AukiInfoEndpoint",
                "AukiCatalogClient",
                "AukiCatalogEndpoint",
                "AukiRegistryClient",
                "AukiRegistryEndpoint",
                "AukiBlobClient",
                "AukiBlobEndpoint",
                "AukiMessageEndpoint",
                "AukiMessageReceiver",
                "AukiStreamClient",
                "AukiStreamEndpoint",
                "AukiStreamSubscription",
            ] {
                assert!(module.getattr(name).is_ok(), "missing {name}");
            }
        });
    }
}
