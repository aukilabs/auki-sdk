//! Content-addressed registry protocols.

#[cfg(feature = "registry-endpoint")]
mod endpoint;
#[cfg(all(feature = "registry-fs-provider", not(target_arch = "wasm32")))]
mod fs;
mod wire;

#[cfg(feature = "registry-endpoint")]
pub use endpoint::{
    REGISTRY_MAX_CONCURRENCY, REGISTRY_OPERATION_TIMEOUT, RegistryClient, RegistryEndpoint,
    RegistryEndpointError, RegistryOperation, RegistryProvider, registry_protocol_spec,
};
#[cfg(all(feature = "registry-fs-provider", not(target_arch = "wasm32")))]
pub use fs::FsRegistryProvider;

/// List-and-fetch registry protocol version 0.3.0.
pub mod v3 {
    pub use super::wire::{
        MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError, RegistryEntryEnvelope, RegistryKind,
        RegistryListEntry, RegistryRequest, RegistryResponse, read_registry_request,
        read_registry_response, write_registry_request, write_registry_response,
    };

    /// Exact authenticated registry 0.3.0 protocol identifier.
    pub const ID: &str = "/auki/auth/1/registries/0.3.0";
}
