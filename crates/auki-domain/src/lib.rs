//! Cluster lifecycle for the Auki SDK.
//!
//! Owns the cluster membership document ([`ClusterMembership`]),
//! the join protocol, the Manager state machine, peer-side heartbeat,
//! successor election, and Manager-handoff orchestration.
//!
//! Not the home for `convert_time` / `convert_pose` — those operate
//! inside a cluster but live elsewhere. Not the home for log-writing
//! session lifecycle either.
//!
//! ## Status
//!
//! Cluster membership document type lands first. Manager state
//! machine, join protocol, heartbeat, election, and handoff follow.

#![warn(missing_docs)]

#[cfg(feature = "browser_runtime")]
pub mod browser_session;
#[cfg(feature = "native_runtime")]
pub mod cluster_manager;
pub mod cluster_membership;
#[cfg(feature = "native_runtime")]
pub mod domain;
#[cfg(feature = "native_runtime")]
pub mod stream_manifest;

#[cfg(feature = "native_runtime")]
pub use auki_network::SessionHandle;
#[cfg(feature = "native_runtime")]
pub use auki_network::registries_protocol::RegistryKind;
#[cfg(feature = "native_runtime")]
pub use auki_network::resources_protocol::{ResourceEntry, ResourcesRequest, ResourcesResponse};
#[cfg(feature = "native_runtime")]
pub use auki_network::stream_protocol::{ReadFrom, map::MapUpdate};
#[cfg(feature = "native_runtime")]
pub use auki_network::{
    MapLogResource, MessageChannelResource, MessageChannelSender, ResourceEntryV3, DeviceModelResource,
    ResourceVariantV3, ResourcesRequestV3, ResourcesResponseV3, ResourcesResponseV4,
};
#[cfg(feature = "native_runtime")]
pub use auki_registry::{
    ClockRegistryEntry, FrameRegistryEntry, MapRegistryEntry, DeviceModelRegistryEntry, SensorRegistryEntry,
};
#[cfg(feature = "native_runtime")]
pub use auki_time::{ClockTransformEstimate, DomainClockEstimate};
#[cfg(feature = "native_runtime")]
pub use cluster_manager::{
    AdmitError, BootstrapError, ClusterManager, ClusterTarget, CreateClusterError, DaemonInfo,
    DiagnosticMessage, DiscoveryClientError, DiscoveryClusterEntry, DomainClockEstimateUnavailable,
    DomainTimeNowError, FetchMapCatalogError, FetchDeviceModelCatalogError, FetchParticipantInfoError, FetchRegistryEntryError,
    FetchResourcesCatalogError, FetchResourcesCatalogV3Error, InboundDiagnosticMessage,
    JoinClusterError, LIVENESS_CHECK_INTERVAL, ManagerLossAction, ResourceCatalogProvider,
    decide_manager_loss_action, elect_successor,
};
pub use cluster_membership::{ClusterMember, ClusterMembership};
#[cfg(feature = "native_runtime")]
pub use domain::{
    Domain, DomainBuilder, DomainBuilderError, DomainConfig, DomainError, DomainOpenMapStreamError,
    DomainOpenMessageChannelError, DomainSendMessageError, MessageChannelReceiver, MessageEvent,
    catalog_of, map_catalog_of,
};
#[cfg(feature = "native_runtime")]
pub use stream_manifest::{BuildStreamManifestError, StreamManifestBuilder};
