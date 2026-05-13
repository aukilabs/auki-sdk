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

pub mod cluster_manager;
pub mod cluster_membership;

pub use cluster_manager::{
    AdmitError, ClusterManager, CreateClusterError, DaemonInfo, FetchParticipantInfoError,
    FetchSensorsCatalogError, JoinClusterError, MANAGER_HEARTBEAT_INTERVAL, SensorCatalogProvider,
    SensorEntry, SensorsResponse, elect_successor,
};
pub use cluster_membership::{ClusterMember, ClusterMembership};
