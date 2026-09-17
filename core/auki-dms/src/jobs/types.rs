use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

fn object() -> Value {
    Value::Object(Default::default())
}
fn attempts() -> u32 {
    3
}
fn page_limit() -> u32 {
    50
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobMode {
    #[default]
    Public,
    Dedicated,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobTaskStatus {
    Queued,
    Leased,
    Running,
    Completed,
    Failed,
    Canceled,
}

/// DMS edges connect stages. Multiple tasks in a stage share its dependencies.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobEdge {
    pub from: String,
    pub to: String,
}

/// A single task or a graph of tasks. The client supplies the selected Domain;
/// DMS validates dependencies, capacity, admission, and pricing.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobSpec {
    pub label: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default = "object")]
    pub meta: Value,
    pub tasks: Vec<JobTaskSpec>,
    #[serde(default)]
    pub edges: Vec<JobEdge>,
}

impl JobSpec {
    pub fn single(label: impl Into<String>, task: JobTaskSpec) -> Self {
        Self {
            label: label.into(),
            priority: 0,
            meta: object(),
            tasks: vec![task],
            edges: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobTaskSpec {
    pub label: String,
    pub stage: String,
    /// An exact worker capability, including third-party names in dedicated mode.
    pub capability: String,
    #[serde(default)]
    pub mode: JobMode,
    #[serde(default)]
    pub capability_filters: BTreeMap<String, String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub inputs_cids: Vec<String>,
    #[serde(default)]
    pub outputs_prefix: Option<String>,
    #[serde(default = "object")]
    pub meta: Value,
    #[serde(default = "attempts")]
    pub max_attempts: u32,
}

impl JobTaskSpec {
    pub fn new(stage: impl Into<String>, capability: impl Into<String>) -> Self {
        let stage = stage.into();
        Self {
            label: stage.clone(),
            stage,
            capability: capability.into(),
            mode: JobMode::Public,
            capability_filters: BTreeMap::new(),
            priority: 0,
            inputs_cids: Vec::new(),
            outputs_prefix: None,
            meta: object(),
            max_attempts: attempts(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobListQuery {
    #[serde(default = "page_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub status: Option<JobStatus>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub match_all_capabilities: bool,
}

impl Default for JobListQuery {
    fn default() -> Self {
        Self {
            limit: page_limit(),
            cursor: None,
            status: None,
            capabilities: Vec::new(),
            match_all_capabilities: false,
        }
    }
}

/// Credit amounts remain decimal strings; bindings must not round through floats.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobEstimate {
    pub total: String,
    pub tasks: Vec<JobEstimateTask>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobEstimateTask {
    pub label: String,
    pub stage: String,
    pub capability: String,
    pub mode: JobMode,
    pub billing_units: String,
    pub estimated_credit_cost: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobRecord {
    pub id: Uuid,
    pub label: String,
    pub domain_id: Uuid,
    pub status: JobStatus,
    pub priority: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub organization_id: Option<Uuid>,
    #[serde(default = "object")]
    pub meta: Value,
    pub credit_lock_id: Option<Uuid>,
    pub credit_lock_amount: Option<String>,
    pub credit_locked_at: Option<DateTime<Utc>>,
    pub credit_released_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobTaskSummary {
    pub queued: u32,
    pub leased: u32,
    pub running: u32,
    pub completed: u32,
    pub failed: u32,
    pub canceled: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobListItem {
    pub job: JobRecord,
    pub tasks_summary: JobTaskSummary,
}

/// One backend page. `next_cursor` is opaque and no pagination workaround is
/// applied for provider bugs or changes while the collection is being read.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobPage {
    pub items: Vec<JobListItem>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobDetails {
    pub job: JobRecord,
    pub tasks_summary: JobTaskSummary,
    pub tasks: Vec<JobTask>,
    pub receipts: Vec<JobReceipt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobTask {
    pub id: Uuid,
    pub job_id: Uuid,
    pub label: String,
    pub stage: String,
    pub capability: String,
    pub capability_filters: BTreeMap<String, String>,
    pub status: JobTaskStatus,
    pub deps_remaining: u32,
    pub priority: i32,
    pub inputs_cids: Vec<String>,
    pub outputs_prefix: Option<String>,
    pub organization_id: Option<Uuid>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub reserved_by: Option<Uuid>,
    /// Worker-defined progress and recent events live in `progress` and `events`.
    pub meta: Value,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub mode: JobMode,
    pub billing_units: String,
    pub estimated_credit_cost: Option<String>,
    pub debited_amount: Option<String>,
    pub debited_at: Option<DateTime<Utc>>,
}

/// A task can have multiple receipts across attempts; outputs are returned as
/// opaque references. Reading their content uses a separate Domain data client.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobReceipt {
    pub id: Uuid,
    pub job_id: Uuid,
    pub task_id: Uuid,
    pub node_id: Option<Uuid>,
    pub outputs: Vec<String>,
    pub meta: Value,
    pub created_at: DateTime<Utc>,
}

/// Acknowledgement only. Running tasks may still be stopping after the job
/// becomes canceled; inspect its tasks to observe execution state.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JobCancellation {
    pub id: Uuid,
    pub status: JobStatus,
    pub updated_at: DateTime<Utc>,
}
