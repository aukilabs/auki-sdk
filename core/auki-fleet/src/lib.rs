//! Bounded, read-only machine observations. No peer, worker or scheduler runs.
mod error;
mod types;

pub use error::FleetError;
pub use types::*;

use auki_auth::{AuthSession, DomainAccessProvider, InventoryNode, InventoryRobot};
use auki_dms::jobs::{
    AukiDmsJobs, BusyNodeSnapshot, DomainJobsClient, JobListQuery, JobMode, JobTaskStatus,
    JobsLimits,
};
use chrono::{DateTime, Utc};
use futures::{StreamExt, stream};
use std::{
    collections::{BTreeMap, HashSet},
    future::Future,
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct FleetLimits {
    /// Overall deadline including authentication and all enrichment requests.
    pub snapshot_timeout: Duration,
    pub max_job_pages: usize,
    pub max_job_details: usize,
    pub detail_concurrency: usize,
    pub max_activity: usize,
}

impl Default for FleetLimits {
    fn default() -> Self {
        Self {
            snapshot_timeout: Duration::from_secs(60),
            max_job_pages: 4,
            max_job_details: 100,
            detail_concurrency: 4,
            max_activity: 1024,
        }
    }
}

#[derive(Clone)]
pub struct AukiFleet {
    session: AuthSession,
    jobs: AukiDmsJobs,
    limits: FleetLimits,
}

impl AukiFleet {
    pub fn new(session: AuthSession, dms_url: &str) -> Result<Self, FleetError> {
        Self::with_limits(session, dms_url, FleetLimits::default())
    }

    pub fn with_limits(
        session: AuthSession,
        dms_url: &str,
        limits: FleetLimits,
    ) -> Result<Self, FleetError> {
        if limits.snapshot_timeout.is_zero()
            || limits.snapshot_timeout > Duration::from_secs(300)
            || !(1..=20).contains(&limits.max_job_pages)
            || !(1..=500).contains(&limits.max_job_details)
            || !(1..=16).contains(&limits.detail_concurrency)
            || !(1..=10_000).contains(&limits.max_activity)
        {
            return Err(FleetError::InvalidInput("invalid snapshot limits"));
        }
        let jobs = AukiDmsJobs::with_limits(session.clone(), dms_url, JobsLimits::default())?;
        Ok(Self {
            session,
            jobs,
            limits,
        })
    }

    pub fn in_domain(&self, domain_id: Uuid) -> DomainFleetClient {
        DomainFleetClient {
            session: self.session.clone(),
            jobs: self.jobs.in_domain(domain_id),
            limits: self.limits.clone(),
            lifetime: Arc::new(Lifetime {
                closed: CancellationToken::new(),
                active: RwLock::new(()),
            }),
        }
    }
}

struct Lifetime {
    closed: CancellationToken,
    active: RwLock<()>,
}

#[derive(Clone)]
pub struct DomainFleetClient {
    session: AuthSession,
    jobs: DomainJobsClient,
    limits: FleetLimits,
    lifetime: Arc<Lifetime>,
}

impl DomainFleetClient {
    pub fn domain_id(&self) -> Uuid {
        self.jobs.domain_id()
    }

    pub async fn close(&self) {
        self.lifetime.closed.cancel();
        let _drained = self.lifetime.active.write().await;
        self.jobs.close().await;
    }

    pub async fn list(&self, query: &FleetQuery) -> Result<FleetSnapshot, FleetError> {
        self.list_with_cancellation(query, &CancellationToken::new())
            .await
    }

    pub async fn list_with_cancellation(
        &self,
        query: &FleetQuery,
        cancellation: &CancellationToken,
    ) -> Result<FleetSnapshot, FleetError> {
        self.run(cancellation, async {
            validate_query(query)?;
            let mut snapshot = self.snapshot(FleetView::Domain, cancellation).await?;
            snapshot
                .machines
                .retain(|machine| matches_capabilities(&machine.capabilities, query));
            Ok(snapshot)
        })
        .await
    }

    pub async fn compute_pool(
        &self,
        query: &ComputePoolQuery,
    ) -> Result<FleetSnapshot, FleetError> {
        self.compute_pool_with_cancellation(query, &CancellationToken::new())
            .await
    }

    pub async fn compute_pool_with_cancellation(
        &self,
        query: &ComputePoolQuery,
        cancellation: &CancellationToken,
    ) -> Result<FleetSnapshot, FleetError> {
        self.run(cancellation, async {
            let filter = FleetQuery {
                capabilities: query.capabilities.clone(),
                match_all_capabilities: query.match_all_capabilities,
            };
            validate_query(&filter)?;
            let mut snapshot = self.snapshot(FleetView::ComputePool, cancellation).await?;
            let mode = match query.mode {
                JobMode::Public => "public",
                JobMode::Dedicated => "dedicated",
            };
            snapshot.machines.retain(|machine| {
                machine.mode == mode && matches_capabilities(&machine.capabilities, &filter)
            });
            Ok(snapshot)
        })
        .await
    }

    async fn run<T>(
        &self,
        cancellation: &CancellationToken,
        operation: impl Future<Output = Result<T, FleetError>>,
    ) -> Result<T, FleetError> {
        let _active = self.lifetime.active.read().await;
        tokio::select! {
            biased;
            _ = self.lifetime.closed.cancelled() => Err(FleetError::Closed),
            _ = self.session.wait_closed() => Err(FleetError::Auth(auki_auth::Error::SessionClosed)),
            _ = cancellation.cancelled() => Err(FleetError::Cancelled),
            _ = futures_timer::Delay::new(self.limits.snapshot_timeout) => Err(FleetError::TimedOut),
            result = async {
                if self.domain_id().is_nil() { return Err(FleetError::InvalidInput("expected non-nil Domain UUID")); }
                operation.await
            } => result,
        }
    }

    async fn snapshot(
        &self,
        view: FleetView,
        cancellation: &CancellationToken,
    ) -> Result<FleetSnapshot, FleetError> {
        // These futures stay owned by the snapshot; cancellation drops and aborts
        // their transports without detached tasks or a second refresh owner.
        let nodes = async {
            let result = self
                .session
                .inventory_nodes(cancellation)
                .await
                .map_err(FleetError::from);
            source(FleetSource::Nodes, result)
        };
        let robots = async {
            if view == FleetView::ComputePool {
                return Ok((None, None));
            }
            let result = self
                .session
                .inventory_robots(self.domain_id(), cancellation)
                .await
                .map_err(FleetError::from);
            let (records, report) = source(FleetSource::Robots, result)?;
            Ok::<_, FleetError>((records, Some(report)))
        };
        let busy = async {
            let result = self
                .jobs
                .busy_nodes_with_cancellation(cancellation)
                .await
                .map_err(FleetError::from);
            source(FleetSource::Busy, result)
        };
        let (
            (nodes, node_report),
            (robots, robot_report),
            (busy, busy_report),
            (activity, job_report),
        ) = futures::try_join!(nodes, robots, busy, self.activity(cancellation))?;
        let mut machines = BTreeMap::new();
        let mut sources = vec![node_report.clone(), job_report, busy_report.clone()];
        if let Some(report) = robot_report {
            for robot in robots.unwrap_or_default() {
                let machine = robot_machine(robot, report.observed_at);
                machines.insert(machine.id, machine);
            }
            sources.insert(0, report);
        }
        let workers: HashSet<_> = activity.iter().map(|task| task.worker_id).collect();
        for node in nodes.unwrap_or_default() {
            if machines.contains_key(&node.id) {
                return Err(FleetError::InvalidResponse(
                    "worker ID belongs to both a robot and a node",
                ));
            }
            if (view == FleetView::ComputePool && compute_capabilities(&node.capabilities))
                || (view == FleetView::Domain && workers.contains(&node.id))
            {
                let mut machine = node_machine(node, node_report.observed_at);
                if view == FleetView::Domain {
                    machine.association = FleetAssociation::ActiveTask;
                }
                machines.insert(machine.id, machine);
            }
        }
        let mut unresolved_activity = Vec::new();
        for task in activity {
            if let Some(machine) = machines.get_mut(&task.worker_id) {
                machine.activity.push(task);
            } else if view == FleetView::Domain {
                unresolved_activity.push(task);
            }
        }
        for machine in machines.values_mut() {
            apply_work(machine, busy.as_ref(), busy_report.observed_at);
            machine
                .activity
                .sort_by_key(|task| (task.job_id, task.task_id));
        }
        unresolved_activity.sort_by_key(|task| (task.job_id, task.task_id));
        Ok(FleetSnapshot {
            domain_id: self.domain_id(),
            view,
            observed_at: Utc::now(),
            complete: unresolved_activity.is_empty()
                && sources
                    .iter()
                    .all(|source| source.state == FleetSourceState::Complete),
            machines: machines.into_values().collect(),
            unresolved_activity,
            sources,
        })
    }

    async fn activity(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(Vec<FleetActivity>, FleetSourceReport), FleetError> {
        let mut report = report(FleetSource::Jobs, FleetSourceState::Complete);
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut ids = HashSet::new();
        let mut jobs = Vec::new();
        for page_number in 0..self.limits.max_job_pages {
            let result = self
                .jobs
                .list_with_cancellation(
                    &JobListQuery {
                        limit: 50,
                        cursor,
                        ..Default::default()
                    },
                    cancellation,
                )
                .await;
            let page = match result {
                Ok(page) => page,
                Err(error) => {
                    if page_number == 0 {
                        report = source::<()>(FleetSource::Jobs, Err(error.into()))?.1;
                    } else {
                        report.record_error(error.into())?;
                    }
                    break;
                }
            };
            for item in page.items {
                if !ids.insert(item.job.id) {
                    report.partial("changing_job_pages");
                    continue;
                }
                // Include canceled/failed jobs whose tasks are still draining.
                if item.tasks_summary.leased > 0 || item.tasks_summary.running > 0 {
                    if jobs.len() == self.limits.max_job_details {
                        report.partial("job_detail_limit");
                    } else {
                        jobs.push(item.job.id);
                    }
                }
            }
            cursor = page.next_cursor;
            let Some(next) = &cursor else {
                break;
            };
            if !cursors.insert(next.clone()) {
                report.partial("repeated_job_cursor");
                break;
            }
            if page_number + 1 == self.limits.max_job_pages {
                report.partial("job_page_limit");
            }
        }
        let mut requests = stream::iter(jobs)
            .map(|job| async move { self.jobs.get_with_cancellation(job, cancellation).await })
            .buffer_unordered(self.limits.detail_concurrency);
        let mut activity = Vec::new();
        let mut task_ids = HashSet::new();
        while let Some(result) = requests.next().await {
            let details = match result {
                Ok(details) => details,
                Err(error) => {
                    report.record_error(error.into())?;
                    continue;
                }
            };
            let observed_at = Utc::now();
            for task in details.tasks {
                if !matches!(task.status, JobTaskStatus::Leased | JobTaskStatus::Running) {
                    continue;
                }
                let Some(worker_id) = task.reserved_by else {
                    report.partial("unassigned_active_task");
                    continue;
                };
                if worker_id.is_nil() || task.id.is_nil() || !task_ids.insert(task.id) {
                    return Err(FleetError::InvalidResponse(
                        "invalid or duplicate active task identity",
                    ));
                }
                if activity.len() == self.limits.max_activity {
                    report.partial("activity_limit");
                    continue;
                }
                activity.push(FleetActivity {
                    worker_id,
                    job_id: details.job.id,
                    task_id: task.id,
                    task_status: task.status,
                    job_status: details.job.status,
                    mode: task.mode,
                    capability: task.capability,
                    lease_expires_at: task.lease_expires_at,
                    last_heartbeat_at: task.last_heartbeat_at,
                    updated_at: task.updated_at,
                    observed_at,
                });
            }
        }
        report.observed_at = Utc::now();
        Ok((activity, report))
    }
}

fn report(source: FleetSource, state: FleetSourceState) -> FleetSourceReport {
    FleetSourceReport {
        source,
        state,
        observed_at: Utc::now(),
        codes: vec![],
        http_status: None,
    }
}

impl FleetSourceReport {
    fn partial(&mut self, code: &str) {
        self.state = FleetSourceState::Partial;
        if !self.codes.iter().any(|existing| existing == code) {
            self.codes.push(code.into());
        }
    }
    fn record_error(&mut self, error: FleetError) -> Result<(), FleetError> {
        if error.fatal() {
            return Err(error);
        }
        self.partial(error.code());
        self.http_status = error.http_status();
        Ok(())
    }
}

fn source<T>(
    source: FleetSource,
    result: Result<Option<T>, FleetError>,
) -> Result<(Option<T>, FleetSourceReport), FleetError> {
    match result {
        Ok(Some(value)) => Ok((Some(value), report(source, FleetSourceState::Complete))),
        Ok(None) => {
            let mut report = report(source, FleetSourceState::Unsupported);
            report.codes.push("unsupported_grant_profile".into());
            Ok((None, report))
        }
        Err(error) if error.fatal() => Err(error),
        Err(error) => {
            let denied = matches!(error.http_status(), Some(401 | 403))
                || error.code() == "authorization_denied";
            let mut report = report(
                source,
                if denied {
                    FleetSourceState::Denied
                } else {
                    FleetSourceState::Unavailable
                },
            );
            report.codes.push(error.code().into());
            report.http_status = error.http_status();
            Ok((None, report))
        }
    }
}

fn validate_query(query: &FleetQuery) -> Result<(), FleetError> {
    if query.capabilities.len() > 64
        || query
            .capabilities
            .iter()
            .any(|cap| cap.trim().is_empty() || cap.len() > 1024)
    {
        return Err(FleetError::InvalidInput(
            "at most 64 nonempty capabilities of at most 1024 bytes",
        ));
    }
    Ok(())
}

fn matches_capabilities(capabilities: &[String], query: &FleetQuery) -> bool {
    query.capabilities.is_empty()
        || if query.match_all_capabilities {
            query
                .capabilities
                .iter()
                .all(|cap| capabilities.contains(cap))
        } else {
            query
                .capabilities
                .iter()
                .any(|cap| capabilities.contains(cap))
        }
}

fn compute_capabilities(capabilities: &[String]) -> bool {
    capabilities.iter().any(|cap| {
        !matches!(
            cap.as_str(),
            "/legacy-domain-server/v0" | "/p2p/circuit-relay/v1"
        )
    })
}

fn presence(status: &str) -> FleetPresence {
    match status {
        "online" => FleetPresence::Online,
        "offline" => FleetPresence::Offline,
        _ => FleetPresence::Unknown,
    }
}

fn node_machine(node: InventoryNode, observed: DateTime<Utc>) -> FleetMachine {
    FleetMachine {
        kind: FleetMachineKind::Compute,
        id: node.id,
        organization_id: node.organization_id,
        name: node.name,
        capabilities: node.capabilities,
        mode: node.mode,
        association: FleetAssociation::Candidate,
        presence: presence(&node.status),
        provider_status: node.status,
        presence_observed_at: observed,
        last_seen_at: None,
        active_lease_expires_at: None,
        work_state: FleetWorkState::Unknown,
        work_observed_at: None,
        activity: vec![],
    }
}

fn robot_machine(robot: InventoryRobot, observed: DateTime<Utc>) -> FleetMachine {
    FleetMachine {
        kind: FleetMachineKind::Robot,
        id: robot.id,
        organization_id: robot.organization_id,
        name: robot.name,
        capabilities: robot.capabilities,
        mode: "dedicated".into(),
        association: FleetAssociation::Assigned,
        presence: presence(&robot.status),
        provider_status: robot.status,
        presence_observed_at: observed,
        last_seen_at: robot.last_seen_at,
        active_lease_expires_at: robot.active_lease_expires_at,
        work_state: FleetWorkState::Unknown,
        work_observed_at: None,
        activity: vec![],
    }
}

fn apply_work(
    machine: &mut FleetMachine,
    busy: Option<&BusyNodeSnapshot>,
    observed: DateTime<Utc>,
) {
    let Some(busy) = busy else {
        return;
    };
    machine.work_observed_at = Some(observed);
    if let Some(entry) = busy.nodes.iter().find(|entry| entry.node_id == machine.id) {
        // An expired observed lease or a different current assignment indicates
        // inconsistent snapshots. Retain the observations without claiming busy.
        let consistent = machine.activity.iter().all(|task| {
            task.task_id == entry.task_id
                && task.job_id == entry.job_id
                && task.mode == entry.task_mode
                && task
                    .lease_expires_at
                    .is_some_and(|expiry| expiry > Utc::now())
        });
        if consistent {
            machine.work_state = FleetWorkState::Busy;
        }
    } else if (machine.mode == "public"
        || (machine.mode == "dedicated" && machine.organization_id == busy.organization_id))
        && machine.presence == FleetPresence::Online
        && machine.activity.is_empty()
        && machine
            .active_lease_expires_at
            .is_none_or(|expiry| expiry <= Utc::now())
    {
        machine.work_state = FleetWorkState::Idle;
    }
}
