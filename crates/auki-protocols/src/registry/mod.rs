//! Content-addressed registry protocols.

mod wire;

/// Get-only registry protocol version 0.2.0.
pub mod v2 {
    pub use super::wire::{
        MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError, RegistryEntryEnvelope, RegistryKind,
        RegistryRequestV2 as RegistryRequest, RegistryResponseV2 as RegistryResponse,
        read_registry_request_v2 as read_registry_request,
        read_registry_response_v2 as read_registry_response,
        write_registry_request_v2 as write_registry_request,
        write_registry_response_v2 as write_registry_response,
    };

    /// Exact authenticated registry 0.2.0 protocol identifier.
    pub const ID: &str = crate::ids::REGISTRIES_V0_2_0;
}

/// List-and-fetch registry protocol version 0.3.0.
pub mod v3 {
    pub use super::wire::{
        MAX_REGISTRIES_FRAME_BYTES, RegistriesProtocolError, RegistryEntryEnvelope, RegistryKind,
        RegistryListEntry, RegistryRequest, RegistryResponse, read_registry_request,
        read_registry_response, write_registry_request, write_registry_response,
    };

    /// Exact authenticated registry 0.3.0 protocol identifier.
    pub const ID: &str = crate::ids::REGISTRIES_V0_3_0;
}
