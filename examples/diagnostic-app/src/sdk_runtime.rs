use crate::flash::FlashMode;
use crate::tick_report::{PeerTickStats, TickReport, TickReportStore};
use auki_domain::DiagnosticMessage;
use auki_domain::{ClusterManager, ClusterTarget, DaemonInfo};
use auki_identity::{Wallet, load_or_mint_seed};
use auki_network::PeerIdentity;
use auki_network::app_instance;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm, collect_routable_listen_addrs};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub discovery_url: String,
    pub cluster_name: String,
    pub display_name: String,
    pub local_peer_id: Option<String>,
    pub local_peer_suffix: Option<String>,
    pub role: Role,
    pub peer_count: usize,
    pub manager_suffix: Option<String>,
    pub peers: Vec<PeerRow>,
    pub events: Vec<String>,
    pub flash_mode: FlashMode,
    pub domain_mode_available: bool,
    pub domain_status: String,
    pub domain_now_ns: Option<i64>,
    pub domain_offset_ns: Option<i64>,
    pub domain_uncertainty_ns: Option<u64>,
    pub peer_tick_stats: Vec<PeerTickStats>,
    pub join_in_flight: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Unclustered,
    Manager,
    Member,
}

#[derive(Debug, Clone)]
pub struct PeerRow {
    pub suffix: String,
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct ClusterSnapshot {
    pub local_peer_id: String,
    pub role: Role,
    pub peer_count: usize,
    pub manager_suffix: Option<String>,
    pub peers: Vec<PeerRow>,
    pub domain_status: String,
    pub domain_now_ns: Option<i64>,
    pub domain_offset_ns: Option<i64>,
    pub domain_uncertainty_ns: Option<u64>,
}

#[derive(Debug)]
pub enum RuntimeCommand {
    SetDiscoveryUrl(String),
    SetClusterName(String),
    SetDisplayName(String),
    JoinOrCreate,
    LeaveCluster,
    SetFlashMode(FlashMode),
    PublishTickReport(TickReport),
    ClusterJoined(Box<ClusterSnapshot>),
    ClusterJoinFailed(String),
    ClusterLeft,
}

const TICK_REPORT_TOPIC: &str = "diagnostic.tick-report";

impl RuntimeSnapshot {
    fn initial() -> Self {
        Self {
            discovery_url: "http://127.0.0.1:8080".into(),
            cluster_name: "hagall-test".into(),
            display_name: "diagnostic-peer".into(),
            local_peer_id: None,
            local_peer_suffix: None,
            role: Role::Unclustered,
            peer_count: 0,
            manager_suffix: None,
            peers: Vec::new(),
            events: vec!["app started".into()],
            flash_mode: FlashMode::Utc,
            domain_mode_available: false,
            domain_status: "not clustered".into(),
            domain_now_ns: None,
            domain_offset_ns: None,
            domain_uncertainty_ns: None,
            peer_tick_stats: Vec::new(),
            join_in_flight: false,
        }
    }

    #[cfg(test)]
    pub fn default_for_tests() -> Self {
        Self {
            discovery_url: "http://127.0.0.1:8080".into(),
            cluster_name: "hagall-test".into(),
            display_name: "diagnostic-peer".into(),
            local_peer_id: Some("12D3KooWabcdef".into()),
            local_peer_suffix: Some("...abcdef".into()),
            role: Role::Unclustered,
            peer_count: 0,
            manager_suffix: None,
            peers: Vec::new(),
            events: Vec::new(),
            flash_mode: FlashMode::Utc,
            domain_mode_available: false,
            domain_status: "not clustered".into(),
            domain_now_ns: None,
            domain_offset_ns: None,
            domain_uncertainty_ns: None,
            peer_tick_stats: Vec::new(),
            join_in_flight: false,
        }
    }

    pub fn apply_command(&mut self, command: RuntimeCommand) {
        match command {
            RuntimeCommand::SetDiscoveryUrl(value) => self.discovery_url = value,
            RuntimeCommand::SetClusterName(value) => self.cluster_name = value,
            RuntimeCommand::SetDisplayName(value) => self.display_name = value,
            RuntimeCommand::SetFlashMode(FlashMode::Utc) => self.flash_mode = FlashMode::Utc,
            RuntimeCommand::SetFlashMode(FlashMode::Domain) => {
                if self.domain_mode_available {
                    self.flash_mode = FlashMode::Domain;
                } else {
                    self.events.push("domain flash unavailable".into());
                }
            }
            RuntimeCommand::PublishTickReport(_) => {}
            RuntimeCommand::JoinOrCreate => {
                if !self.join_in_flight {
                    self.join_in_flight = true;
                    self.events
                        .push(format!("joining cluster {}", self.cluster_name));
                }
            }
            RuntimeCommand::LeaveCluster => {
                self.join_in_flight = false;
            }
            RuntimeCommand::ClusterJoined(cluster) => {
                let was_unclustered = self.role == Role::Unclustered;
                let was_joining = self.join_in_flight;
                self.join_in_flight = false;
                self.local_peer_suffix = Some(crate::flash::peer_suffix(&cluster.local_peer_id));
                self.local_peer_id = Some(cluster.local_peer_id);
                self.role = cluster.role;
                self.peer_count = cluster.peer_count;
                self.manager_suffix = cluster.manager_suffix;
                self.peers = cluster.peers;
                self.domain_status = cluster.domain_status;
                self.domain_now_ns = cluster.domain_now_ns;
                self.domain_offset_ns = cluster.domain_offset_ns;
                self.domain_uncertainty_ns = cluster.domain_uncertainty_ns;
                self.domain_mode_available = self.domain_now_ns.is_some();
                if was_unclustered || was_joining {
                    self.events.push("cluster joined".into());
                }
            }
            RuntimeCommand::ClusterJoinFailed(error) => {
                self.join_in_flight = false;
                self.events.push(format!("cluster join failed: {error}"));
            }
            RuntimeCommand::ClusterLeft => {
                let had_cluster_state = self.join_in_flight
                    || self.local_peer_id.is_some()
                    || self.role != Role::Unclustered
                    || self.peer_count != 0
                    || self.manager_suffix.is_some()
                    || !self.peers.is_empty()
                    || self.domain_mode_available
                    || self.domain_now_ns.is_some();
                self.join_in_flight = false;
                self.local_peer_id = None;
                self.local_peer_suffix = None;
                self.role = Role::Unclustered;
                self.peer_count = 0;
                self.manager_suffix = None;
                self.peers.clear();
                self.domain_mode_available = false;
                self.domain_status = "not clustered".into();
                self.domain_now_ns = None;
                self.domain_offset_ns = None;
                self.domain_uncertainty_ns = None;
                self.peer_tick_stats.clear();
                if had_cluster_state {
                    self.events.push("cluster left".into());
                }
            }
        }
    }
}

pub struct SdkRuntime {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
    commands: mpsc::UnboundedSender<WorkerCommand>,
    join_generation: JoinGeneration,
    runtime: tokio::runtime::Runtime,
    worker: Option<JoinHandle<()>>,
}

struct RuntimeWorker {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
    commands: mpsc::UnboundedReceiver<WorkerCommand>,
    join_generation: JoinGeneration,
    manager: Option<ClusterManager>,
    tick_reports: TickReportStore,
}

enum WorkerCommand {
    JoinOrCreate { token: u64 },
    PublishTickReport(TickReport),
    LeaveCluster,
    Shutdown,
}

#[derive(Clone, Default)]
struct JoinGeneration {
    value: Arc<AtomicU64>,
}

impl JoinGeneration {
    fn current(&self) -> u64 {
        self.value.load(Ordering::SeqCst)
    }

    fn invalidate(&self) -> u64 {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_current(&self, token: u64) -> bool {
        self.current() == token
    }
}

impl SdkRuntime {
    pub fn spawn() -> Self {
        let snapshot = Arc::new(Mutex::new(RuntimeSnapshot::initial()));
        let (commands, worker_commands) = mpsc::unbounded_channel();
        let join_generation = JoinGeneration::default();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("auki-diagnostic-runtime")
            .build()
            .expect("diagnostic tokio runtime");
        let worker = runtime.spawn(
            RuntimeWorker {
                snapshot: Arc::clone(&snapshot),
                commands: worker_commands,
                join_generation: join_generation.clone(),
                manager: None,
                tick_reports: TickReportStore::default(),
            }
            .run(),
        );

        Self {
            snapshot,
            commands,
            join_generation,
            runtime,
            worker: Some(worker),
        }
    }

    pub fn send(&self, command: RuntimeCommand) {
        match command {
            RuntimeCommand::JoinOrCreate => {
                let (should_send, token) = {
                    let mut snapshot = self.snapshot.lock().expect("runtime snapshot lock");
                    let should_send = !snapshot.join_in_flight;
                    let token = self.join_generation.current();
                    snapshot.apply_command(RuntimeCommand::JoinOrCreate);
                    (should_send, token)
                };
                if should_send
                    && self
                        .commands
                        .send(WorkerCommand::JoinOrCreate { token })
                        .is_err()
                {
                    self.apply(RuntimeCommand::ClusterJoinFailed(
                        "runtime worker stopped".into(),
                    ));
                }
            }
            RuntimeCommand::LeaveCluster => {
                self.join_generation.invalidate();
                self.apply(RuntimeCommand::ClusterLeft);
                if self.commands.send(WorkerCommand::LeaveCluster).is_err() {
                    self.apply(RuntimeCommand::ClusterLeft);
                }
            }
            RuntimeCommand::PublishTickReport(report) => {
                let _ = self.commands.send(WorkerCommand::PublishTickReport(report));
            }
            RuntimeCommand::SetDiscoveryUrl(_)
            | RuntimeCommand::SetClusterName(_)
            | RuntimeCommand::SetDisplayName(_)
            | RuntimeCommand::SetFlashMode(_)
            | RuntimeCommand::ClusterJoined(_)
            | RuntimeCommand::ClusterJoinFailed(_)
            | RuntimeCommand::ClusterLeft => self.apply(command),
        }
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.lock().expect("runtime snapshot lock").clone()
    }

    fn apply(&self, command: RuntimeCommand) {
        self.snapshot
            .lock()
            .expect("runtime snapshot lock")
            .apply_command(command);
    }
}

impl Drop for SdkRuntime {
    fn drop(&mut self) {
        self.join_generation.invalidate();
        let _ = self.commands.send(WorkerCommand::Shutdown);

        if let Some(worker) = self.worker.take() {
            let _ = self
                .runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(5), worker).await });
        }
    }
}

impl RuntimeWorker {
    async fn run(mut self) {
        let mut refresh = tokio::time::interval(Duration::from_millis(50));

        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        self.shutdown_manager().await;
                        break;
                    };
                    if matches!(command, WorkerCommand::Shutdown) {
                        self.handle_command(command).await;
                        break;
                    }
                    self.handle_command(command).await;
                }
                _ = refresh.tick() => {
                    self.refresh_cluster_snapshot();
                    self.ingest_diagnostic_messages();
                }
            }
        }
    }

    async fn handle_command(&mut self, command: WorkerCommand) {
        match command {
            WorkerCommand::JoinOrCreate { token } => self.join_or_create(token).await,
            WorkerCommand::PublishTickReport(report) => self.publish_tick_report(report),
            WorkerCommand::LeaveCluster => {
                self.shutdown_manager().await;
                self.apply(RuntimeCommand::ClusterLeft);
                self.tick_reports = TickReportStore::default();
                self.publish_tick_stats();
            }
            WorkerCommand::Shutdown => {
                self.join_generation.invalidate();
                self.shutdown_manager().await;
            }
        }
    }

    async fn join_or_create(&mut self, token: u64) {
        if !self.join_generation.is_current(token) {
            return;
        }

        if self.manager.is_some() {
            if self.join_generation.is_current(token) {
                self.refresh_cluster_snapshot();
            }
            return;
        }

        let snapshot = self.snapshot.lock().expect("runtime snapshot lock").clone();
        let seed = match load_or_mint_seed(&identity_seed_path()) {
            Ok(seed) => seed,
            Err(error) => {
                self.apply_join_failed_if_current(token, format!("identity seed: {error}"));
                return;
            }
        };
        let wallet =
            Wallet::from_seed(seed.to_vec()).expect("load_or_mint_seed produces a 32-byte seed");
        let identity = PeerIdentity::from_wallet(wallet);

        let mut swarm = match build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![
                    "/ip4/0.0.0.0/tcp/0".parse().expect("valid tcp listen addr"),
                    "/ip4/0.0.0.0/udp/0/quic-v1"
                        .parse()
                        .expect("valid quic listen addr"),
                ],
                agent_version: "auki-diagnostic-app/0.0.0".into(),
                enable_relay_server: false,
            },
        ) {
            Ok(swarm) => swarm,
            Err(error) => {
                self.apply_join_failed_if_current(token, format!("build swarm: {error}"));
                return;
            }
        };

        let mut local_multiaddrs =
            collect_routable_listen_addrs(&mut swarm, Duration::from_secs(3)).await;
        if local_multiaddrs.is_empty() {
            local_multiaddrs = swarm.listeners().cloned().collect();
        }

        let app_instance = app_instance::derive().unwrap_or_else(|_| "unknown".into());
        let daemon_info = DaemonInfo {
            app: "auki-diagnostic-app".into(),
            name: snapshot.display_name,
            session_id: uuid::Uuid::new_v4().to_string(),
            session_clock_id: "compat".into(),
            session_clock_hash: "compat".into(),
            app_instance,
        };

        let manager = match ClusterManager::bootstrap(
            ClusterTarget::join_or_create(snapshot.cluster_name),
            identity,
            local_multiaddrs,
            snapshot.discovery_url,
            swarm,
            decline_all_streams(),
            daemon_info,
        )
        .await
        {
            Ok(manager) => manager,
            Err(error) => {
                self.apply_join_failed_if_current(token, format!("cluster bootstrap: {error}"));
                return;
            }
        };

        if !self.join_generation.is_current(token) {
            let _ = manager.shutdown().await;
            return;
        }

        let cluster_snapshot = cluster_snapshot(&manager);
        self.manager = Some(manager);
        self.apply(RuntimeCommand::ClusterJoined(Box::new(cluster_snapshot)));
    }

    fn refresh_cluster_snapshot(&self) {
        if let Some(manager) = &self.manager {
            self.apply(RuntimeCommand::ClusterJoined(Box::new(cluster_snapshot(
                manager,
            ))));
        }
    }

    fn ingest_diagnostic_messages(&mut self) {
        let Some(manager) = &self.manager else {
            return;
        };
        for inbound in manager.drain_diagnostic_messages() {
            if inbound.message.topic != TICK_REPORT_TOPIC {
                continue;
            }
            match serde_json::from_str::<TickReport>(&inbound.message.payload_json) {
                Ok(report) => self.tick_reports.record_remote(report),
                Err(error) => {
                    eprintln!(
                        "tick report decode failed from {}: {error}",
                        inbound.peer_id
                    );
                }
            }
        }
        self.publish_tick_stats();
    }

    fn publish_tick_report(&mut self, report: TickReport) {
        self.tick_reports.record_local(report.clone());
        if let Some(manager) = &self.manager {
            match serde_json::to_string(&report) {
                Ok(payload_json) => {
                    let _ = manager.broadcast_diagnostic_message(DiagnosticMessage {
                        topic: TICK_REPORT_TOPIC.into(),
                        payload_json,
                    });
                }
                Err(error) => {
                    eprintln!("tick report encode failed: {error}");
                }
            }
        }
        self.publish_tick_stats();
    }

    fn publish_tick_stats(&self) {
        self.snapshot
            .lock()
            .expect("runtime snapshot lock")
            .peer_tick_stats = self.tick_reports.peer_stats();
    }

    fn apply(&self, command: RuntimeCommand) {
        self.snapshot
            .lock()
            .expect("runtime snapshot lock")
            .apply_command(command);
    }

    fn apply_join_failed_if_current(&self, token: u64, error: String) {
        if self.join_generation.is_current(token) {
            self.apply(RuntimeCommand::ClusterJoinFailed(error));
        }
    }

    async fn shutdown_manager(&mut self) {
        if let Some(manager) = self.manager.take() {
            let _ = manager.shutdown().await;
        }
    }
}

fn identity_seed_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".auki")
        .join("diagnostic-app")
        .join("identity.seed")
}

fn cluster_snapshot(manager: &ClusterManager) -> ClusterSnapshot {
    let local_peer_id = manager.local_peer_id();
    let manager_peer_id = manager.manager_peer_id();
    let membership = manager.membership();
    let (domain_status, domain_now_ns, domain_offset_ns, domain_uncertainty_ns) =
        match manager.domain_clock_estimate() {
            Ok(estimate) => match manager.domain_time_now() {
                Ok(now_ns) => (
                    "synced".to_string(),
                    Some(now_ns),
                    Some(estimate.total_offset_ns),
                    Some(estimate.uncertainty_ns),
                ),
                Err(error) => (
                    error.to_string(),
                    None,
                    Some(estimate.total_offset_ns),
                    Some(estimate.uncertainty_ns),
                ),
            },
            Err(error) => (error.to_string(), None, None, None),
        };
    let peers = membership
        .peers
        .iter()
        .map(|member| PeerRow {
            suffix: crate::flash::peer_suffix(&member.peer_id.to_string()),
            role: if member.peer_id == manager_peer_id {
                Role::Manager
            } else {
                Role::Member
            },
        })
        .collect();

    ClusterSnapshot {
        local_peer_id: local_peer_id.to_string(),
        role: if manager.is_manager() {
            Role::Manager
        } else {
            Role::Member
        },
        peer_count: manager.peer_count(),
        manager_suffix: Some(crate::flash::peer_suffix(&manager_peer_id.to_string())),
        peers,
        domain_status,
        domain_now_ns,
        domain_offset_ns,
        domain_uncertainty_ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_applies_local_config_commands() {
        let runtime = SdkRuntime::spawn();

        runtime.send(RuntimeCommand::SetDiscoveryUrl(
            "http://discovery.local:8080".into(),
        ));
        runtime.send(RuntimeCommand::SetClusterName("lab".into()));
        runtime.send(RuntimeCommand::SetDisplayName("bench-peer".into()));

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.discovery_url, "http://discovery.local:8080");
        assert_eq!(snapshot.cluster_name, "lab");
        assert_eq!(snapshot.display_name, "bench-peer");
        assert!(!snapshot.join_in_flight);
    }

    #[test]
    fn runtime_keeps_domain_mode_disabled_without_domain_time() {
        let runtime = SdkRuntime::spawn();

        runtime.send(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.flash_mode, FlashMode::Utc);
        assert!(!snapshot.domain_mode_available);
        assert_eq!(snapshot.events.last().unwrap(), "domain flash unavailable");
    }

    #[test]
    fn cluster_joined_updates_domain_time_snapshot() {
        let mut snapshot = RuntimeSnapshot::default_for_tests();

        snapshot.apply_command(RuntimeCommand::ClusterJoined(Box::new(ClusterSnapshot {
            local_peer_id: "12D3KooWabcdef".into(),
            role: Role::Member,
            peer_count: 2,
            manager_suffix: Some("...manager".into()),
            peers: Vec::new(),
            domain_status: "synced".into(),
            domain_now_ns: Some(42_000),
            domain_offset_ns: Some(1_500),
            domain_uncertainty_ns: Some(25),
        })));

        assert!(snapshot.domain_mode_available);
        assert_eq!(snapshot.domain_status, "synced");
        assert_eq!(snapshot.domain_now_ns, Some(42_000));
        assert_eq!(snapshot.domain_offset_ns, Some(1_500));
        assert_eq!(snapshot.domain_uncertainty_ns, Some(25));
    }

    #[test]
    fn cluster_left_clears_domain_time_snapshot() {
        let mut snapshot = RuntimeSnapshot::default_for_tests();
        snapshot.domain_mode_available = true;
        snapshot.domain_status = "synced".into();
        snapshot.domain_now_ns = Some(42_000);
        snapshot.domain_offset_ns = Some(1_500);
        snapshot.domain_uncertainty_ns = Some(25);

        snapshot.apply_command(RuntimeCommand::ClusterLeft);

        assert!(!snapshot.domain_mode_available);
        assert_eq!(snapshot.domain_status, "not clustered");
        assert_eq!(snapshot.domain_now_ns, None);
        assert_eq!(snapshot.domain_offset_ns, None);
        assert_eq!(snapshot.domain_uncertainty_ns, None);
    }

    #[test]
    fn cluster_commands_update_join_state_and_snapshot() {
        let mut snapshot = RuntimeSnapshot::default_for_tests();
        assert!(!snapshot.join_in_flight);

        snapshot.apply_command(RuntimeCommand::JoinOrCreate);
        assert!(snapshot.join_in_flight);

        snapshot.apply_command(RuntimeCommand::ClusterJoined(Box::new(ClusterSnapshot {
            local_peer_id: "12D3KooWabcdef".into(),
            role: Role::Manager,
            peer_count: 1,
            manager_suffix: Some("...abcdef".into()),
            peers: vec![PeerRow {
                suffix: "...abcdef".into(),
                role: Role::Manager,
            }],
            domain_status: "synced".into(),
            domain_now_ns: Some(12_345),
            domain_offset_ns: Some(0),
            domain_uncertainty_ns: Some(0),
        })));

        assert!(!snapshot.join_in_flight);
        assert_eq!(snapshot.local_peer_id.as_deref(), Some("12D3KooWabcdef"));
        assert_eq!(snapshot.local_peer_suffix.as_deref(), Some("...abcdef"));
        assert_eq!(snapshot.role, Role::Manager);
        assert_eq!(snapshot.peer_count, 1);
        assert_eq!(snapshot.manager_suffix.as_deref(), Some("...abcdef"));
        assert_eq!(snapshot.peers.len(), 1);
        assert!(snapshot.domain_mode_available);
        assert_eq!(snapshot.domain_now_ns, Some(12_345));

        snapshot.apply_command(RuntimeCommand::ClusterLeft);
        assert!(!snapshot.join_in_flight);
        assert_eq!(snapshot.local_peer_id, None);
        assert_eq!(snapshot.local_peer_suffix, None);
        assert_eq!(snapshot.role, Role::Unclustered);
        assert_eq!(snapshot.peer_count, 0);
        assert_eq!(snapshot.manager_suffix, None);
        assert!(snapshot.peers.is_empty());
        assert!(!snapshot.domain_mode_available);
        assert_eq!(snapshot.domain_now_ns, None);
    }

    #[test]
    fn join_generation_invalidates_stale_bootstrap_results() {
        let generation = JoinGeneration::default();
        let token = generation.current();

        assert!(generation.is_current(token));

        generation.invalidate();

        assert!(!generation.is_current(token));
        assert!(generation.is_current(generation.current()));
    }
}
