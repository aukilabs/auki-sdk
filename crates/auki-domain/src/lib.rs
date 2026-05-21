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

// UniFFI scaffolding. Each annotated `Record` / `Enum` / `Object` /
// `Error` derive emits `impl FfiConverter<crate::UniFfiTag> for X`, and
// `UniFfiTag` is only defined where `setup_scaffolding!()` is invoked.
// Without this, building `--features swift-bindings` fails before the
// binding crate ever links. Gated so default builds stay scaffolding-free.
#[cfg(feature = "swift-bindings")]
uniffi::setup_scaffolding!();

#[cfg(feature = "browser_runtime")]
pub mod browser_session;
#[cfg(feature = "native_runtime")]
pub mod cluster_manager;
pub mod cluster_membership;
#[cfg(feature = "native_runtime")]
pub mod stream_manifest;

#[cfg(feature = "native_runtime")]
pub use auki_network::registries_protocol::RegistryKind;
#[cfg(feature = "native_runtime")]
pub use auki_network::resources_protocol::{
    ResourceEntry, ResourceKind, ResourcePinholeIntrinsics, ResourceQuat, ResourceSpatialTransform,
    ResourceVec3, ResourcesRequest, ResourcesResponse, SensorStreamResource, TransformEdgeResource,
};
#[cfg(feature = "native_runtime")]
pub use auki_registry::{ClockRegistryEntry, FrameRegistryEntry, SensorRegistryEntry};
#[cfg(feature = "native_runtime")]
pub use auki_time::{ClockTransformEstimate, DomainClockEstimate};
#[cfg(feature = "native_runtime")]
pub use cluster_manager::{
    AdmitError, BootstrapError, ClusterManager, ClusterTarget, CreateClusterError, DaemonInfo,
    DiagnosticMessage, DiscoveryClientError, DiscoveryClusterEntry, DomainClockEstimateUnavailable,
    DomainTimeNowError, FetchParticipantInfoError, FetchRegistryEntryError,
    FetchResourcesCatalogError, FetchSensorsCatalogError, InboundDiagnosticMessage,
    JoinClusterError, LIVENESS_CHECK_INTERVAL, ResourceCatalogProvider, SensorCatalogProvider,
    SensorEntry, SensorsRequest, SensorsResponse, elect_successor,
};
pub use cluster_membership::{ClusterMember, ClusterMembership};
#[cfg(feature = "native_runtime")]
pub use stream_manifest::{BuildStreamManifestError, StreamManifestBuilder};
