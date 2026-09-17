use auki_dms::jobs::{JobMode, JobStatus, JobTaskStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FleetQuery {
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub match_all_capabilities: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputePoolQuery {
    pub mode: JobMode,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub match_all_capabilities: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetView {
    Domain,
    ComputePool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetMachineKind {
    Robot,
    Compute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetAssociation {
    Assigned,
    ActiveTask,
    Candidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetPresence {
    Online,
    Offline,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetWorkState {
    Idle,
    Busy,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetSource {
    Robots,
    Nodes,
    Jobs,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetSourceState {
    Complete,
    Partial,
    Denied,
    Unsupported,
    Unavailable,
}

/// Coverage of one source, not a guarantee that several services were observed
/// atomically. Codes are redacted categories, never backend response messages.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FleetSourceReport {
    pub source: FleetSource,
    pub state: FleetSourceState,
    pub observed_at: DateTime<Utc>,
    pub codes: Vec<String>,
    pub http_status: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FleetActivity {
    pub worker_id: Uuid,
    pub job_id: Uuid,
    pub task_id: Uuid,
    pub task_status: JobTaskStatus,
    pub job_status: JobStatus,
    pub mode: JobMode,
    pub capability: String,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FleetMachine {
    pub kind: FleetMachineKind,
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub capabilities: Vec<String>,
    pub mode: String,
    pub association: FleetAssociation,
    pub presence: FleetPresence,
    pub provider_status: String,
    pub presence_observed_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub active_lease_expires_at: Option<DateTime<Utc>>,
    pub work_state: FleetWorkState,
    pub work_observed_at: Option<DateTime<Utc>>,
    pub activity: Vec<FleetActivity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FleetSnapshot {
    pub domain_id: Uuid,
    pub view: FleetView,
    pub observed_at: DateTime<Utc>,
    pub machines: Vec<FleetMachine>,
    /// Visible Domain tasks whose worker could not be joined to DDS inventory.
    pub unresolved_activity: Vec<FleetActivity>,
    pub sources: Vec<FleetSourceReport>,
    pub complete: bool,
}
