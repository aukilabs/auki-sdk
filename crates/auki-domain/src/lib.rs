//! Authenticated Domain lifecycle and retained Auki application protocols.
//!
//! A [`Domain`] owns one stable P2P identity, one authenticated node, and one
//! exact DDS Domain UUID. Hosts provide signed authority and explicit routes;
//! the crate has no Manager, membership, election, Discovery, or Domain-time
//! control plane.

#![warn(missing_docs)]

#[cfg(feature = "native_runtime")]
mod authenticated_runtime;
#[cfg(feature = "native_runtime")]
pub mod domain;
#[cfg(feature = "native_runtime")]
mod resource_catalog;
#[cfg(feature = "native_runtime")]
pub mod stream_manifest;

#[cfg(feature = "native_runtime")]
pub use auki_network::{
    MapCatalogProvider, MapLogResource, MessageChannelResource, ResourceEntryV3, ResourceVariantV3,
    ResourcesRequestV3, ResourcesResponseV3, ResourcesResponseV4,
    info_protocol::AuthenticatedParticipantInfo,
    registries_protocol::{RegistryKind, RegistryListEntry},
    resources_protocol::{ResourceEntry, ResourcesRequest, ResourcesResponse},
    stream_protocol::{ReadFrom, StreamRequest, map::MapUpdate},
    stream_runtime::{
        SourceStream, StreamDispatch, StreamEntry, StreamError, StreamItem, StreamProvider,
        StreamSubscription, decline_all_streams,
    },
};
#[cfg(feature = "native_runtime")]
pub use auki_p2p::{DdsVerificationKeys, Identity, Multiaddr, PeerId, SignedP2pCredential};
#[cfg(feature = "native_runtime")]
pub use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorRegistryEntry,
};

#[cfg(feature = "native_runtime")]
pub use authenticated_runtime::{
    AuthenticatedDomainError, DomainRollbackError,
    authority::{DomainAuthority, DomainAuthorityError},
    blobs::BlobsV1Error,
    info_v1::{InfoV1Error, ParticipantInfoProvider},
    messages::{
        MessageChannelRegistrationError, MessagesV1Error, OpenMessageChannelError, SendMessageError,
    },
    peers::{
        DomainPeerInfoError, DomainPeers, KnownPeer, KnownPeerEvent, KnownPeerRecvError,
        KnownPeerSnapshot, KnownPeerSubscription,
    },
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols, DomainRouteAttempt,
    },
    registries::RegistriesError,
    resources_v2::ResourcesV2Error,
    resources_v3::ResourcesV3Error,
    resources_v4::ResourcesV4Error,
    routes::{DomainRouteSnapshot, DomainRoutes, DomainRoutesError, PeerRoutes},
    status::{DomainFailure, DomainStatus},
    storage::StorageError,
    streams::StreamsError,
};
#[cfg(feature = "native_runtime")]
pub use domain::{
    Domain, DomainBuilder, DomainBuilderError, DomainConfig, DomainError, DomainOpenMapStreamError,
    DomainSendMessageError, MessageChannelReceiver, MessageChannelSender, MessageEvent, catalog_of,
    map_catalog_of,
};
#[cfg(feature = "native_runtime")]
pub use resource_catalog::ResourceCatalogProvider;
#[cfg(feature = "native_runtime")]
pub use stream_manifest::{BuildStreamManifestError, StreamManifestBuilder};
