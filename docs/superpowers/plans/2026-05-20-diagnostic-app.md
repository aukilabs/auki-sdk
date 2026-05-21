# Auki Diagnostic App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone native diagnostic example app for macOS/Linux that can join an Auki cluster, show peer/cluster diagnostics, flash every three seconds on UTC time, and expose a Domain timing mode that is disabled until heartbeat domain-clock sync lands.

**Architecture:** Add a top-level `examples/diagnostic-app/` Cargo package using `eframe/egui`. Keep deterministic logic in small testable modules (`flash`, `app_state`), isolate SDK/tokio cluster work in `sdk_runtime`, and keep `ui` as rendering over snapshots. Current SDK state does not expose a `session clock -> domain clock` sync snapshot yet, so this first implementation shows Domain mode as unavailable instead of faking corrected timing.

**Tech Stack:** Rust 2024, `eframe/egui`, `tokio`, existing SDK crates (`auki-identity`, `auki-network`, `auki-domain`), standard library UTC/wall-clock timing.

---

## File Structure

- Modify `Cargo.toml`: add `examples/diagnostic-app` as a workspace member.
- Create `examples/diagnostic-app/Cargo.toml`: package dependencies and metadata.
- Create `examples/diagnostic-app/README.md`: run instructions and two-laptop manual test.
- Create `examples/diagnostic-app/src/main.rs`: eframe entrypoint.
- Create `examples/diagnostic-app/src/app_state.rs`: UI-owned config, snapshots, event log, status derivation.
- Create `examples/diagnostic-app/src/flash.rs`: UTC/domain flash scheduling and peer-id display helpers.
- Create `examples/diagnostic-app/src/sdk_runtime.rs`: background tokio runtime, cluster commands, SDK snapshots.
- Create `examples/diagnostic-app/src/ui.rs`: egui rendering.

Do not update changelogs for this work.

---

### Task 1: Scaffold the Example Package

**Files:**
- Modify: `Cargo.toml`
- Create: `examples/diagnostic-app/Cargo.toml`
- Create: `examples/diagnostic-app/src/main.rs`
- Create: `examples/diagnostic-app/src/app_state.rs`
- Create: `examples/diagnostic-app/src/flash.rs`
- Create: `examples/diagnostic-app/src/sdk_runtime.rs`
- Create: `examples/diagnostic-app/src/ui.rs`

- [ ] **Step 1: Add the workspace member**

Add the example package to the root `Cargo.toml` members list:

```toml
    "examples/diagnostic-app",
```

- [ ] **Step 2: Create package manifest**

Create `examples/diagnostic-app/Cargo.toml`:

```toml
[package]
name = "auki-diagnostic-app"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Native diagnostic example app for Auki networking, clustering, and time-sync visibility."

[dependencies]
auki-domain = { path = "../../crates/auki-domain" }
auki-identity = { path = "../../crates/auki-identity" }
auki-network = { path = "../../crates/auki-network", features = ["swarm", "discovery_client", "app_instance"] }
eframe = "0.30"
egui = "0.30"
libp2p = { version = "0.56", default-features = false, features = ["tokio", "tcp", "quic", "noise", "yamux", "macros"] }
multiaddr = "0.18"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 3: Create module stubs**

Create `examples/diagnostic-app/src/main.rs`:

```rust
mod app_state;
mod flash;
mod sdk_runtime;
mod ui;

use app_state::DiagnosticApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Auki Diagnostics")
            .with_inner_size([1120.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Auki Diagnostics",
        options,
        Box::new(|cc| Ok(Box::new(DiagnosticApp::new(cc)))),
    )
}
```

Create `examples/diagnostic-app/src/app_state.rs`:

```rust
use crate::sdk_runtime::{RuntimeCommand, RuntimeSnapshot, SdkRuntime};

pub struct DiagnosticApp {
    runtime: SdkRuntime,
    snapshot: RuntimeSnapshot,
}

impl DiagnosticApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let runtime = SdkRuntime::spawn();
        let snapshot = runtime.snapshot();
        Self { runtime, snapshot }
    }

    pub fn send(&self, command: RuntimeCommand) {
        self.runtime.send(command);
    }

    pub fn refresh_snapshot(&mut self) {
        self.snapshot = self.runtime.snapshot();
    }

    pub fn snapshot(&self) -> &RuntimeSnapshot {
        &self.snapshot
    }
}

impl eframe::App for DiagnosticApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.refresh_snapshot();
        crate::ui::render(ctx, self);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
```

Create `examples/diagnostic-app/src/flash.rs`:

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FLASH_PERIOD: Duration = Duration::from_secs(3);
pub const FLASH_ON: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashMode {
    Utc,
    Domain,
}

pub fn peer_suffix(peer_id: &str) -> String {
    let suffix: String = peer_id
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{suffix}")
}

pub fn utc_now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn next_period_boundary_ns(now_ns: u128, period_ns: u128) -> u128 {
    ((now_ns / period_ns) + 1) * period_ns
}

pub fn elapsed_in_period_ns(now_ns: u128, period_ns: u128) -> u128 {
    now_ns % period_ns
}

pub fn flash_is_on(now_ns: u128, period_ns: u128, flash_on_ns: u128) -> bool {
    elapsed_in_period_ns(now_ns, period_ns) < flash_on_ns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_suffix_uses_last_six_characters() {
        assert_eq!(peer_suffix("12D3KooWabcdefgh"), "...cdefgh");
    }

    #[test]
    fn peer_suffix_handles_short_ids() {
        assert_eq!(peer_suffix("abc"), "...abc");
    }

    #[test]
    fn next_boundary_uses_strictly_future_period() {
        assert_eq!(next_period_boundary_ns(0, 3_000), 3_000);
        assert_eq!(next_period_boundary_ns(2_999, 3_000), 3_000);
        assert_eq!(next_period_boundary_ns(3_000, 3_000), 6_000);
    }

    #[test]
    fn flash_is_on_inside_opening_window() {
        assert!(flash_is_on(6_050, 3_000, 180));
        assert!(!flash_is_on(6_250, 3_000, 180));
    }
}
```

Create `examples/diagnostic-app/src/sdk_runtime.rs`:

```rust
use crate::flash::FlashMode;

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

#[derive(Debug)]
pub enum RuntimeCommand {
    SetDiscoveryUrl(String),
    SetClusterName(String),
    SetDisplayName(String),
    JoinOrCreate,
    LeaveCluster,
    SetFlashMode(FlashMode),
}

pub struct SdkRuntime;

impl SdkRuntime {
    pub fn spawn() -> Self {
        Self
    }

    pub fn send(&self, _command: RuntimeCommand) {}

    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
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
        }
    }
}
```

Create `examples/diagnostic-app/src/ui.rs`:

```rust
use crate::app_state::DiagnosticApp;
use crate::flash::{FLASH_ON, FLASH_PERIOD, FlashMode, flash_is_on, utc_now_ns};
use crate::sdk_runtime::{Role, RuntimeCommand};

pub fn render(ctx: &egui::Context, app: &mut DiagnosticApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Auki Diagnostics");
        ui.label("Scaffold complete. UI implementation follows in later tasks.");
        let snapshot = app.snapshot();
        ui.label(format!("Cluster: {}", snapshot.cluster_name));
        ui.label(format!("Flash mode: {:?}", snapshot.flash_mode));
        let on = flash_is_on(
            utc_now_ns(),
            FLASH_PERIOD.as_nanos(),
            FLASH_ON.as_nanos(),
        );
        ui.label(if on { "FLASH" } else { "waiting" });

        if ui.button("UTC").clicked() {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Utc));
        }
        if ui
            .add_enabled(snapshot.domain_mode_available, egui::Button::new("Domain"))
            .clicked()
        {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Domain));
        }

        match snapshot.role {
            Role::Unclustered => {
                ui.label("Not clustered");
            }
            Role::Manager => {
                ui.label("Manager");
            }
            Role::Member => {
                ui.label("Member");
            }
        }
    });
}
```

- [ ] **Step 4: Run initial build/test**

Run:

```bash
cargo test -p auki-diagnostic-app
cargo check -p auki-diagnostic-app
```

Expected: both commands pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml examples/diagnostic-app
git commit -m "feat: scaffold diagnostic example app"
```

---

### Task 2: Implement Deterministic App State

**Files:**
- Modify: `examples/diagnostic-app/src/app_state.rs`
- Modify: `examples/diagnostic-app/src/sdk_runtime.rs`

- [ ] **Step 1: Add app-state tests**

Append these tests to `examples/diagnostic-app/src/app_state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::flash::FlashMode;
    use crate::sdk_runtime::{PeerRow, Role};

    #[test]
    fn domain_mode_is_disabled_when_unavailable() {
        let mut state = RuntimeSnapshot::default_for_tests();
        state.flash_mode = FlashMode::Utc;
        state.domain_mode_available = false;

        state.apply_command(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        assert_eq!(state.flash_mode, FlashMode::Utc);
        assert_eq!(state.events.last().unwrap(), "domain flash unavailable");
    }

    #[test]
    fn domain_mode_can_be_selected_when_available() {
        let mut state = RuntimeSnapshot::default_for_tests();
        state.domain_mode_available = true;

        state.apply_command(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        assert_eq!(state.flash_mode, FlashMode::Domain);
    }

    #[test]
    fn manager_row_uses_role_from_snapshot() {
        let row = PeerRow {
            suffix: "...Q2La9F".into(),
            role: Role::Manager,
        };

        assert_eq!(row.role, Role::Manager);
        assert_eq!(row.suffix, "...Q2La9F");
    }
}
```

- [ ] **Step 2: Run red tests**

Run:

```bash
cargo test -p auki-diagnostic-app app_state
```

Expected: FAIL because `RuntimeSnapshot::default_for_tests` and `apply_command` do not exist.

- [ ] **Step 3: Implement snapshot mutation helpers**

Add this impl to `examples/diagnostic-app/src/sdk_runtime.rs`:

```rust
impl RuntimeSnapshot {
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
            RuntimeCommand::JoinOrCreate | RuntimeCommand::LeaveCluster => {}
        }
    }
}
```

- [ ] **Step 4: Run green tests**

Run:

```bash
cargo test -p auki-diagnostic-app app_state
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add examples/diagnostic-app/src/app_state.rs examples/diagnostic-app/src/sdk_runtime.rs
git commit -m "test: cover diagnostic app state"
```

---

### Task 3: Implement the Runtime Command Loop

**Files:**
- Modify: `examples/diagnostic-app/src/sdk_runtime.rs`

- [ ] **Step 1: Add runtime command-loop tests**

Append these tests to `examples/diagnostic-app/src/sdk_runtime.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::flash::FlashMode;

    #[test]
    fn runtime_applies_local_config_commands() {
        let runtime = SdkRuntime::spawn();

        runtime.send(RuntimeCommand::SetDiscoveryUrl("http://discovery.local:8080".into()));
        runtime.send(RuntimeCommand::SetClusterName("lab".into()));
        runtime.send(RuntimeCommand::SetDisplayName("macbook".into()));

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.discovery_url, "http://discovery.local:8080");
        assert_eq!(snapshot.cluster_name, "lab");
        assert_eq!(snapshot.display_name, "macbook");
    }

    #[test]
    fn runtime_keeps_domain_mode_disabled_without_sync_api() {
        let runtime = SdkRuntime::spawn();

        runtime.send(RuntimeCommand::SetFlashMode(FlashMode::Domain));

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.flash_mode, FlashMode::Utc);
        assert!(!snapshot.domain_mode_available);
    }
}
```

- [ ] **Step 2: Run red tests**

Run:

```bash
cargo test -p auki-diagnostic-app sdk_runtime
```

Expected: FAIL because `SdkRuntime::send` currently ignores commands.

- [ ] **Step 3: Implement shared snapshot storage**

Replace `SdkRuntime` in `examples/diagnostic-app/src/sdk_runtime.rs` with:

```rust
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SdkRuntime {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
}

impl SdkRuntime {
    pub fn spawn() -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(RuntimeSnapshot {
                discovery_url: "http://127.0.0.1:8080".into(),
                cluster_name: "hagall-test".into(),
                display_name: default_display_name(),
                local_peer_id: None,
                local_peer_suffix: None,
                role: Role::Unclustered,
                peer_count: 0,
                manager_suffix: None,
                peers: Vec::new(),
                events: vec!["app started".into()],
                flash_mode: FlashMode::Utc,
                domain_mode_available: false,
            })),
        }
    }

    pub fn send(&self, command: RuntimeCommand) {
        self.snapshot
            .lock()
            .expect("snapshot lock")
            .apply_command(command);
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.lock().expect("snapshot lock").clone()
    }
}

fn default_display_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "diagnostic-peer".into())
}
```

- [ ] **Step 4: Run green tests**

Run:

```bash
cargo test -p auki-diagnostic-app sdk_runtime
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add examples/diagnostic-app/src/sdk_runtime.rs
git commit -m "feat: add diagnostic runtime state loop"
```

---

### Task 4: Build the Native UI

**Files:**
- Modify: `examples/diagnostic-app/src/ui.rs`

- [ ] **Step 1: Replace scaffold UI with the dashboard**

Replace `examples/diagnostic-app/src/ui.rs` with:

```rust
use crate::app_state::DiagnosticApp;
use crate::flash::{FLASH_ON, FLASH_PERIOD, FlashMode, flash_is_on, utc_now_ns};
use crate::sdk_runtime::{Role, RuntimeCommand};

pub fn render(ctx: &egui::Context, app: &mut DiagnosticApp) {
    egui::SidePanel::left("sidebar")
        .resizable(false)
        .default_width(300.0)
        .show(ctx, |ui| render_sidebar(ui, app));

    egui::CentralPanel::default().show(ctx, |ui| {
        render_status_strip(ui, app);
        ui.add_space(12.0);
        render_flash_panel(ui, app);
        ui.add_space(12.0);
        render_bottom_diagnostics(ui, app);
    });
}

fn render_sidebar(ui: &mut egui::Ui, app: &mut DiagnosticApp) {
    let snapshot = app.snapshot().clone();
    ui.heading("Auki Diagnostics");
    ui.separator();

    ui.label("Peer self");
    ui.monospace(snapshot.local_peer_suffix.as_deref().unwrap_or("not started"));
    ui.label(format!("name: {}", snapshot.display_name));
    ui.label(format!("role: {}", role_label(snapshot.role)));

    ui.separator();
    ui.label("Cluster");
    ui.text_edit_singleline(&mut snapshot.cluster_name.clone());
    ui.label(format!("peers: {}", snapshot.peer_count));
    ui.label(format!(
        "manager: {}",
        snapshot.manager_suffix.as_deref().unwrap_or("none")
    ));

    if ui.button("Join / Create").clicked() {
        app.send(RuntimeCommand::JoinOrCreate);
    }
    if ui.button("Leave Cluster").clicked() {
        app.send(RuntimeCommand::LeaveCluster);
    }

    ui.separator();
    ui.label("Discovery URL");
    ui.monospace(&snapshot.discovery_url);
}

fn render_status_strip(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let snapshot = app.snapshot();
    ui.horizontal(|ui| {
        status_box(ui, "Networking", networking_label(snapshot.role));
        status_box(ui, "Heartbeat", "unavailable");
        status_box(ui, "Session -> domain", "unavailable");
    });
}

fn render_flash_panel(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let snapshot = app.snapshot();
    ui.horizontal(|ui| {
        ui.label("Flash timing");
        let utc_selected = snapshot.flash_mode == FlashMode::Utc;
        if ui.selectable_label(utc_selected, "UTC").clicked() {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Utc));
        }
        let domain_selected = snapshot.flash_mode == FlashMode::Domain;
        if ui
            .add_enabled(
                snapshot.domain_mode_available,
                egui::SelectableLabel::new(domain_selected, "Domain"),
            )
            .clicked()
        {
            app.send(RuntimeCommand::SetFlashMode(FlashMode::Domain));
        }
    });

    let now_ns = utc_now_ns();
    let on = match snapshot.flash_mode {
        FlashMode::Utc => flash_is_on(now_ns, FLASH_PERIOD.as_nanos(), FLASH_ON.as_nanos()),
        FlashMode::Domain => false,
    };

    let desired = egui::vec2(ui.available_width(), 380.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let bg = if on {
        egui::Color32::from_rgb(250, 204, 21)
    } else {
        egui::Color32::from_rgb(15, 23, 42)
    };
    painter.rect_filled(rect, 8.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if on { "FLASH" } else { "waiting" },
        egui::FontId::proportional(56.0),
        if on {
            egui::Color32::from_rgb(17, 24, 39)
        } else {
            egui::Color32::from_rgb(229, 231, 235)
        },
    );
}

fn render_bottom_diagnostics(ui: &mut egui::Ui, app: &DiagnosticApp) {
    let snapshot = app.snapshot();
    ui.columns(2, |columns| {
        columns[0].heading("Peers");
        for peer in &snapshot.peers {
            columns[0].monospace(format!("{} {}", peer.suffix, role_label(peer.role)));
        }

        columns[1].heading("Events");
        for event in snapshot.events.iter().rev().take(8) {
            columns[1].monospace(event);
        }
    });
}

fn status_box(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(180.0);
        ui.label(label);
        ui.heading(value);
    });
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Unclustered => "Unclustered",
        Role::Manager => "Manager",
        Role::Member => "Member",
    }
}

fn networking_label(role: Role) -> &'static str {
    match role {
        Role::Unclustered => "Not clustered",
        Role::Manager | Role::Member => "Clustered",
    }
}
```

- [ ] **Step 2: Run UI compile check**

Run:

```bash
cargo check -p auki-diagnostic-app
```

Expected: PASS.

- [ ] **Step 3: Run the app locally**

Run:

```bash
cargo run -p auki-diagnostic-app
```

Expected: A native window opens. The flash panel alternates to `FLASH` for roughly 180 ms every three seconds in UTC mode. Domain mode is disabled.

- [ ] **Step 4: Commit**

```bash
git add examples/diagnostic-app/src/ui.rs
git commit -m "feat: render diagnostic native UI"
```

---

### Task 5: Wire Real SDK Cluster Join/Create

**Files:**
- Modify: `examples/diagnostic-app/src/sdk_runtime.rs`
- Modify: `examples/diagnostic-app/src/app_state.rs`

- [ ] **Step 1: Add SDK runtime command variants**

Extend `RuntimeSnapshot` and `RuntimeCommand` in `examples/diagnostic-app/src/sdk_runtime.rs`:

```rust
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
    pub join_in_flight: bool,
}

#[derive(Debug)]
pub enum RuntimeCommand {
    SetDiscoveryUrl(String),
    SetClusterName(String),
    SetDisplayName(String),
    JoinOrCreate,
    LeaveCluster,
    SetFlashMode(FlashMode),
    ClusterJoined(Box<ClusterSnapshot>),
    ClusterJoinFailed(String),
    ClusterLeft,
}

#[derive(Debug, Clone)]
pub struct ClusterSnapshot {
    pub local_peer_id: String,
    pub role: Role,
    pub peer_count: usize,
    pub manager_suffix: Option<String>,
    pub peers: Vec<PeerRow>,
}
```

Update `apply_command` so `JoinOrCreate` sets `join_in_flight = true`, `ClusterJoinFailed` clears it and pushes the error, `ClusterJoined` copies the snapshot, and `ClusterLeft` resets cluster fields.

- [ ] **Step 2: Run check**

Run:

```bash
cargo check -p auki-diagnostic-app
```

Expected: FAIL until `join_in_flight` is initialized and new command variants are handled.

- [ ] **Step 3: Implement background tokio runtime**

Extend `SdkRuntime` with command channel and a stored cluster handle:

```rust
use auki_domain::{ClusterManager, ClusterTarget, DaemonInfo};
use auki_identity::{Wallet, load_or_mint_seed};
use auki_network::PeerIdentity;
use auki_network::app_instance;
use auki_network::stream_runtime::decline_all_streams;
use auki_network::swarm::{SwarmConfig, build_swarm, collect_routable_listen_addrs};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct SdkRuntime {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
    commands: mpsc::UnboundedSender<RuntimeCommand>,
}

struct RuntimeWorker {
    snapshot: Arc<Mutex<RuntimeSnapshot>>,
    commands: mpsc::UnboundedReceiver<RuntimeCommand>,
    manager: Option<ClusterManager>,
}
```

In `SdkRuntime::spawn`, create a `tokio::runtime::Runtime`, create the channel, spawn `RuntimeWorker::run`, and keep the sender. `send` should both update immediate UI state for simple commands and forward async commands to the worker.

- [ ] **Step 4: Implement join/create worker path**

Add this worker method:

```rust
impl RuntimeWorker {
    async fn join_or_create(&mut self) {
        let snapshot = self.snapshot.lock().expect("snapshot lock").clone();
        let seed_path = identity_seed_path();
        let seed = match load_or_mint_seed(&seed_path) {
            Ok(seed) => seed,
            Err(e) => {
                self.apply(RuntimeCommand::ClusterJoinFailed(format!("identity seed: {e}")));
                return;
            }
        };
        let wallet = Wallet::from_seed(&seed);
        let identity = PeerIdentity::from_wallet(&wallet);
        let local_peer_id = identity.peer_id();

        let mut swarm = match build_swarm(
            &identity,
            SwarmConfig {
                listen_addresses: vec![
                    "/ip4/0.0.0.0/tcp/0".parse().expect("valid tcp listen addr"),
                    "/ip4/0.0.0.0/udp/0/quic-v1".parse().expect("valid quic listen addr"),
                ],
                agent_version: "auki-diagnostic-app/0.0.0".into(),
                enable_relay_server: false,
            },
        ) {
            Ok(swarm) => swarm,
            Err(e) => {
                self.apply(RuntimeCommand::ClusterJoinFailed(format!("build swarm: {e}")));
                return;
            }
        };

        let addrs = collect_routable_listen_addrs(&mut swarm, std::time::Duration::from_secs(3)).await;
        let app_instance = app_instance::derive().unwrap_or_else(|_| "unknown".into());
        let daemon_info = DaemonInfo {
            app: "auki-diagnostic-app".into(),
            name: snapshot.display_name.clone(),
            session_id: uuid::Uuid::new_v4().to_string(),
            session_clock_id: "compat".into(),
            session_clock_hash: "compat".into(),
            app_instance,
        };

        let manager = match ClusterManager::bootstrap(
            ClusterTarget::join_or_create(snapshot.cluster_name.clone()),
            identity,
            addrs,
            snapshot.discovery_url.clone(),
            swarm,
            decline_all_streams(),
            daemon_info,
        )
        .await
        {
            Ok(manager) => manager,
            Err(e) => {
                self.apply(RuntimeCommand::ClusterJoinFailed(format!("cluster bootstrap: {e}")));
                return;
            }
        };

        let cluster_snapshot = cluster_snapshot(&manager, local_peer_id.to_string());
        self.manager = Some(manager);
        self.apply(RuntimeCommand::ClusterJoined(Box::new(cluster_snapshot)));
    }

    fn apply(&self, command: RuntimeCommand) {
        self.snapshot.lock().expect("snapshot lock").apply_command(command);
    }
}
```

Add helpers:

```rust
fn identity_seed_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".auki")
        .join("diagnostic-app")
        .join("identity.seed")
}

fn cluster_snapshot(manager: &ClusterManager, local_peer_id: String) -> ClusterSnapshot {
    let local = manager.local_peer_id();
    let manager_peer = manager.manager_peer_id();
    let membership = manager.membership();
    let peers = membership
        .peers
        .iter()
        .map(|member| PeerRow {
            suffix: crate::flash::peer_suffix(&member.peer_id.to_string()),
            role: if member.peer_id == manager_peer {
                Role::Manager
            } else {
                Role::Member
            },
        })
        .collect();

    ClusterSnapshot {
        local_peer_id,
        role: if local == manager_peer {
            Role::Manager
        } else {
            Role::Member
        },
        peer_count: membership.peers.len(),
        manager_suffix: Some(crate::flash::peer_suffix(&manager_peer.to_string())),
        peers,
    }
}
```

- [ ] **Step 5: Poll cluster snapshot while joined**

In `RuntimeWorker::run`, use `tokio::time::interval(Duration::from_millis(250))`. On each tick, if `self.manager.is_some()`, rebuild and apply `ClusterJoined(Box::new(...))`. This keeps peer count, Manager role, and peer rows fresh after membership gossip and handoff.

- [ ] **Step 6: Implement leave**

When `RuntimeCommand::LeaveCluster` reaches the worker, call `manager.shutdown().await` if a manager exists, then set `self.manager = None` and apply `ClusterLeft`.

- [ ] **Step 7: Run check**

Run:

```bash
cargo check -p auki-diagnostic-app
```

Expected: PASS after imports, snapshot initialization, and command handling are consistent.

- [ ] **Step 8: Run against Discovery manually**

Run:

```bash
cargo run -p auki-diagnostic-app
```

Expected: clicking `Join / Create` creates or joins the configured cluster. The UI shows local peer suffix, role, peer count, manager suffix, and an event indicating success. Domain mode remains unavailable.

- [ ] **Step 9: Commit**

```bash
git add examples/diagnostic-app/src/sdk_runtime.rs examples/diagnostic-app/src/app_state.rs
git commit -m "feat: wire diagnostic app cluster runtime"
```

---

### Task 6: Finish UI Inputs and Runtime Feedback

**Files:**
- Modify: `examples/diagnostic-app/src/ui.rs`
- Modify: `examples/diagnostic-app/src/sdk_runtime.rs`

- [ ] **Step 1: Add editable config fields**

In `render_sidebar`, replace read-only labels with local editable buffers stored in `DiagnosticApp`. Add fields to `DiagnosticApp`:

```rust
discovery_url_input: String,
cluster_name_input: String,
display_name_input: String,
```

Initialize them from the first snapshot in `DiagnosticApp::new`. On text edit change, send `SetDiscoveryUrl`, `SetClusterName`, or `SetDisplayName`.

- [ ] **Step 2: Disable buttons during transitions**

Use `snapshot.join_in_flight` to disable `Join / Create` while a join is running. Enable `Leave Cluster` only when `snapshot.role != Role::Unclustered`.

- [ ] **Step 3: Improve flash panel labels**

Show these labels in the flash panel footer:

```text
mode: UTC
baseline: no Auki correction
period: 3.000s
```

When Domain mode is unavailable, render:

```text
Domain unavailable: heartbeat sync API not implemented in this SDK build
```

- [ ] **Step 4: Run app smoke**

Run:

```bash
cargo run -p auki-diagnostic-app
```

Expected: user can edit Discovery URL, cluster name, and display name; UTC flash continues smoothly; Domain button is disabled with explanatory text.

- [ ] **Step 5: Commit**

```bash
git add examples/diagnostic-app/src/app_state.rs examples/diagnostic-app/src/ui.rs examples/diagnostic-app/src/sdk_runtime.rs
git commit -m "feat: polish diagnostic app controls"
```

---

### Task 7: Add README and Verification Notes

**Files:**
- Create: `examples/diagnostic-app/README.md`

- [ ] **Step 1: Write README**

Create `examples/diagnostic-app/README.md`:

````markdown
# Auki Diagnostic App

Native macOS/Linux diagnostic example for Auki networking, clustering, and time-sync visibility.

## Run

```bash
cargo run -p auki-diagnostic-app
```

The app defaults to:

- Discovery URL: `http://127.0.0.1:8080`
- Cluster name: `hagall-test`
- Flash mode: `UTC`

## Timing Modes

`UTC` mode flashes every three seconds on host UTC wall-clock boundaries and applies no Auki correction. Use this first to eyeball whether two machines have visibly different UTC time.

`Domain` mode is reserved for heartbeat domain-clock sync. In this SDK build, the domain sync snapshot API is not implemented yet, so the app shows Domain mode as unavailable rather than faking corrected timing.

## Two-Laptop Test

1. Start Discovery.
2. Run this app on the macOS laptop.
3. Run this app on the Linux laptop.
4. Set the same Discovery URL and cluster name on both.
5. Click `Join / Create` on both.
6. Confirm both apps show two peers and peer-id suffixes.
7. Put the laptops side by side and compare UTC flashes.
8. After heartbeat domain-clock sync lands, switch both apps to Domain mode and compare the corrected flashes.
````

- [ ] **Step 2: Run verification**

Run:

```bash
cargo test -p auki-diagnostic-app
cargo check -p auki-diagnostic-app
```

Expected: both commands pass.

- [ ] **Step 3: Commit**

```bash
git add examples/diagnostic-app/README.md
git commit -m "docs: document diagnostic example app"
```

---

## Plan Self-Review

- Spec coverage: covers `examples/diagnostic-app/`, native macOS/Linux, peer suffixes, UTC-first flash, Domain switch, peer-self diagnostics, cluster diagnostics, no changelog updates.
- Current SDK reality: heartbeat domain-clock sync is not implemented in `auki-domain`; this plan keeps Domain mode unavailable rather than inventing fake sync.
- Placeholder scan: no placeholder steps remain; every task has concrete files, commands, and expected outcomes.
- Type consistency: `FlashMode`, `RuntimeSnapshot`, `RuntimeCommand`, `Role`, and `PeerRow` are introduced before use by later tasks.
