//! Resource catalog protocols.

#[cfg(feature = "catalog-endpoint")]
mod endpoint;

pub mod v2;
pub mod v3;
pub mod v4;

#[cfg(feature = "catalog-endpoint")]
pub use endpoint::{
    CATALOG_MAX_CONCURRENCY, CATALOG_OPERATION_TIMEOUT, CatalogClient, CatalogEndpoint,
    CatalogEndpointError, CatalogOperation, CatalogProvider, maps_protocol_spec,
    resources_protocol_spec,
};
