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
mod served_protocols;
#[cfg(feature = "native_runtime")]
pub mod stream_manifest;
#[cfg(feature = "native_runtime")]
mod stream_runtime;

#[cfg(feature = "native_runtime")]
pub use auki_p2p::{DdsVerificationKeys, Identity, Multiaddr, PeerId, SignedP2pCredential};
#[cfg(feature = "native_runtime")]
pub use auki_protocols::{
    catalog::{
        v2::{ResourceEntry, ResourcesRequest, ResourcesResponse},
        v3::{
            MessageChannelResource, ResourceEntry as ResourceEntryV3,
            ResourceVariant as ResourceVariantV3, ResourcesRequest as ResourcesRequestV3,
            ResourcesResponse as ResourcesResponseV3,
        },
        v4::{MapLogResource, ResourcesResponse as ResourcesResponseV4},
    },
    info::v1::AuthenticatedParticipantInfo,
    registry::v3::{RegistryKind, RegistryListEntry},
    stream::v2::{ReadFrom, StreamRequest, map::MapUpdate},
};
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
    relay_reservations::{DomainRelayError, DomainRelayReservations},
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
pub use resource_catalog::{MapCatalogProvider, ResourceCatalogProvider};
#[cfg(feature = "native_runtime")]
pub use served_protocols::ServedProtocols;
#[cfg(feature = "native_runtime")]
pub use stream_manifest::{BuildStreamManifestError, StreamManifestBuilder};
#[cfg(feature = "native_runtime")]
pub use stream_runtime::{
    SourceStream, StreamDispatch, StreamEntry, StreamError, StreamItem, StreamProvider,
    StreamSubscription, decline_all_streams,
};
