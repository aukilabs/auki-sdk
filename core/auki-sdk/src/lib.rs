//! Shared credentials, Domain HTTP clients and authenticated Auki peers.
//!
//! [`AukiPeerConfig`] defines the intentionally small host contract and
//! [`AukiPeer`] retains one authenticated transport, its renewable authority, and
//! optional DMS-backed relay reachability for their complete shared lifetime.
//! [`AukiDomains`] and [`AukiDomainData`] use the same [`AukiCredential`] without
//! starting a peer or configuring DMS.

mod bootstrap;
mod config;
mod discovery;
mod protocol_contract;
mod resolution;
mod runtime_policy;
mod served_protocols;
mod status;

#[cfg(not(target_arch = "wasm32"))]
pub use auki_tasks::{
    AukiComputeCredential, AukiDmsTasks, AukiRobotCredential, ComputeConfig, MachineCredential,
    RobotConfig, TaskAccessToken, TaskContext, TaskCredential, TaskError, TaskHandler, TaskLease,
    TaskOutcome, TaskResult, TaskSpec, TasksConfig,
};

#[cfg(not(target_arch = "wasm32"))]
mod task_peer;
#[cfg(not(target_arch = "wasm32"))]
pub use task_peer::{AukiTaskPeer, AukiTaskPeerConfig, TaskPeerContext};

#[cfg(not(target_arch = "wasm32"))]
mod authorization;
#[cfg(any(test, target_arch = "wasm32"))]
mod browser_booking;
#[cfg(target_arch = "wasm32")]
mod browser_peer_runtime;
#[cfg(target_arch = "wasm32")]
mod browser_protocols;
#[cfg(not(target_arch = "wasm32"))]
mod context;
#[cfg(not(target_arch = "wasm32"))]
mod known_peers;
#[cfg(not(target_arch = "wasm32"))]
mod peer_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod protocols;

#[cfg(not(target_arch = "wasm32"))]
mod authority;

#[cfg(not(target_arch = "wasm32"))]
mod relay;

#[cfg(not(target_arch = "wasm32"))]
pub use auki_auth::AppCredentials;
pub use auki_auth::{
    AukiCredential, AuthClient, AuthEnvironment, AuthFailureKind, AuthLimits, AuthSession,
    Credentials, DomainChoice, DomainDescriptor, DomainSelection, Error as AuthError, PreparedPeer,
    PrincipalKind, SecretString, ZitadelSessionCredentials, ZitadelSessionStore,
};
pub use auki_dms::jobs::{
    AukiDmsJobs, DomainJobsClient, JobCancellation, JobDetails, JobEdge, JobEstimate,
    JobEstimateTask, JobListItem, JobListQuery, JobMode, JobPage, JobReceipt, JobRecord, JobSpec,
    JobStatus, JobTask, JobTaskSpec, JobTaskStatus, JobTaskSummary, JobsError, JobsLimits,
};
pub use auki_domain_client::{
    AukiDomainData, AukiDomains, DataError, DataLimits, DataListQuery, DataMetadata, DataWrite,
    DiscoveredDomain, DomainDataClient, DomainDiscoveryPage, DomainDiscoveryQuery, DomainListQuery,
    DomainPage, DomainPermission, DomainSummary, Portal, PortalDomain, PortalId, PortalPose,
    TransferOptions,
};
pub use auki_fleet::{
    AukiFleet, ComputePoolQuery, DomainFleetClient, FleetActivity, FleetAssociation, FleetError,
    FleetLimits, FleetMachine, FleetMachineKind, FleetPresence, FleetQuery, FleetSnapshot,
    FleetSource, FleetSourceReport, FleetSourceState, FleetView, FleetWorkState,
};
#[cfg(target_arch = "wasm32")]
pub use auki_p2p::BrowserAuthenticatedRouteStream as AuthenticatedRouteStream;
pub use auki_p2p::{
    AuthenticatedPeer, DdsVerificationKeys, Identity, Multiaddr, P2PAccessClaims, PeerId,
    RelayCircuitRoutes, SignedP2pCredential,
};
#[cfg(not(target_arch = "wasm32"))]
pub use auki_p2p::{
    AuthenticatedRouteStream, RouteCatalogError, RouteCatalogStatus, RouteFence, RouteSnapshot,
    validate_relay_circuit_routes,
};
#[cfg(not(target_arch = "wasm32"))]
pub use authority::{ExternalAuthorityRefreshRequest, ExternalAuthorityUpdate};
#[cfg(not(target_arch = "wasm32"))]
pub use authorization::{
    AukiPeerAuthorization, AukiPeerAuthorizationError, AukiPeerAuthorizationSnapshot,
};
pub use bootstrap::{AukiPeerBootstrap, AukiPeerBootstrapError};
#[cfg(target_arch = "wasm32")]
pub use browser_peer_runtime::{
    AukiPeer, AukiPeerError, AukiPeerLifecycle, AukiPeerReachability, AukiPeerShutdownError,
    AukiPeerStartError,
};
#[cfg(target_arch = "wasm32")]
pub use browser_protocols::{AukiPeerProtocols, AukiProtocolRegistration};
#[cfg(not(target_arch = "wasm32"))]
pub use config::InitialPeerRoutes;
pub use config::{
    AukiPeerConfig, AukiPeerConfigError, AukiRelayConfig, AukiRelayConfigError, AukiRelayMode,
    DEV_DMS_BASE_URL,
};
#[cfg(not(target_arch = "wasm32"))]
pub use context::{AukiPeerProtocolContext, AukiPeerRoutes, AukiPeerRoutesError};
pub use discovery::{
    AukiDiscovery, AukiDiscoveryCandidate, AukiDiscoveryError, AukiDiscoverySource,
    DEV_DDS_BASE_URL, DdsTrackerConfig, DdsTrackerMode,
};
#[cfg(not(target_arch = "wasm32"))]
pub use known_peers::{
    AukiKnownPeer, AukiKnownPeerEvent, AukiKnownPeerRecvError, AukiKnownPeerSnapshot,
    AukiKnownPeerSubscription, AukiKnownPeers,
};
#[cfg(not(target_arch = "wasm32"))]
pub use peer_runtime::{
    AukiPeer, AukiPeerAuthorityError, AukiPeerLifecycle, AukiPeerRelayError, AukiPeerShutdownError,
    AukiPeerStartError, AukiPeerTransportError, ExternalAuthorityControl,
    ExternalAuthorityReplaceOutcome,
};
pub use protocol_contract::{
    AukiProtocolError, AukiProtocolRouteAttempt, AukiProtocolSpec, AukiProtocolStream,
};
#[cfg(not(target_arch = "wasm32"))]
pub use protocols::{AukiPeerProtocols, AukiProtocolRegistration};
pub use resolution::{AukiPeerConnectError, AukiPeerIdentity, AukiResolvedPeer};
#[cfg(not(target_arch = "wasm32"))]
pub use status::AukiPeerStatus;
pub use status::{AukiPeerExit, AukiPeerFailure};
