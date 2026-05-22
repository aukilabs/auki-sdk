# Game-Grade Domain Time Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden heartbeat-derived domain time so every peer can expose game-networking style sync quality, reject stale transforms explicitly, avoid avoidable Manager split-brain, and make the diagnostic app show why domain time is or is not usable.

**Architecture:** Keep heartbeat as the carrier for clock observations and domain-clock source metadata. Keep NTP/sample-window math in `auki-time`. Keep `ClusterManager` responsible for cluster domain-clock source selection, stale-domain policy, and Manager election safety. Keep `auki-network` as a transport/runtime layer that emits raw observations and never chooses a domain clock. The diagnostic app reads the SDK APIs and displays sync/election state without inventing fallback time.

**Tech Stack:** Rust, Tokio, libp2p, serde, `auki-time`, `auki-network`, `auki-domain`, and the egui diagnostic app under `examples/diagnostic-app`.

---

## File Structure

- `crates/auki-time/src/lib.rs` - clock-sync quality snapshots, stale checks, jitter statistics, domain-source uncertainty composition.
- `crates/auki-time/src/readme.md` - implemented API notes for the new snapshot/stale policy.
- `crates/auki-network/src/heartbeat_protocol.rs` - heartbeat domain-clock metadata wire shape if source uncertainty is added.
- `crates/auki-network/src/network_runtime.rs` - raw heartbeat observation pass-through only; no domain decisions.
- `crates/auki-network/src/readme.md` - heartbeat payload/runtime behavior update.
- `crates/auki-domain/src/cluster_manager.rs` - domain-clock stale policy, election safety, peer-id collision guard, diagnostic events.
- `crates/auki-domain/src/readme.md` - current behavior update.
- `crates/auki-domain/src/sprint.md` - next-step status after implementation.
- `examples/diagnostic-app/src/sdk_runtime.rs` - snapshot fields and event ingestion.
- `examples/diagnostic-app/src/ui.rs` - sync quality, stale state, and election diagnostics.
- `examples/diagnostic-app/src/flash.rs` - only if Domain flash needs stale-state gating text or mode handling.
- `crates/*/changelog.md`, `examples/*/changelog.md`, `docs/*/changelog.md`, root `changelog.md` - append-only propagation for touched areas.

## Setup

- [ ] Start from current `develop`.

```bash
git fetch origin
git switch develop
git pull --ff-only origin develop
git switch -c codex/game-grade-domain-time-hardening
```

- [ ] Confirm the existing domain-clock baseline is green before changes.

```bash
cargo test -p auki-time
cargo test -p auki-domain
cargo test -p auki-diagnostic-app
```

Expected result: all non-ignored tests pass.

## Task 1: Add Clock-Sync Quality Snapshots In `auki-time`

Purpose: retained NTP windows already exist, but callers need the full quality picture, not only the selected best offset.

- [ ] Add failing tests in `crates/auki-time/src/lib.rs`.

Test names:

```rust
clock_sync_snapshot_reports_window_quality
clock_sync_snapshot_reports_stale_age_without_fresh_estimate
clock_sync_snapshot_resets_quality_on_clock_hash_change
clock_sync_snapshot_rejects_samples_above_uncertainty_limit
```

The first test should insert at least three accepted samples for one local/remote clock pair and assert:

- best estimate is the lowest-uncertainty sample.
- `retained_sample_count == 3`.
- `offset_jitter_ns == max(offset_ns) - min(offset_ns)` across retained samples.
- `uncertainty_jitter_ns == max(uncertainty_ns) - min(uncertainty_ns)`.
- `min_uncertainty_ns`, `median_uncertainty_ns`, and `max_uncertainty_ns` are populated.
- `newest_observed_at_clock_ns` is the newest sample completion time.
- `age_ns` is computed against the caller-supplied local clock reading.
- `is_stale == false` while `age_ns <= max_sample_age_ns`.

The stale test should call the new snapshot API with `now_local_clock_ns` beyond `max_sample_age_ns` and assert `is_stale == true`. It should also assert that the old `estimate(...)` API still returns the best retained estimate until domain consumers migrate to the explicit stale-aware API.

- [ ] Add public snapshot structs.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSyncSnapshot {
    pub estimate: ClockTransformEstimate,
    pub retained_sample_count: usize,
    pub min_uncertainty_ns: u64,
    pub median_uncertainty_ns: u64,
    pub max_uncertainty_ns: u64,
    pub uncertainty_jitter_ns: u64,
    pub offset_jitter_ns: u64,
    pub min_round_trip_ns: u64,
    pub median_round_trip_ns: u64,
    pub max_round_trip_ns: u64,
    pub newest_observed_at_clock_ns: i64,
    pub oldest_observed_at_clock_ns: i64,
    pub age_ns: Option<u64>,
    pub max_sample_age_ns: u64,
    pub is_stale: bool,
}
```

- [ ] Add stale-aware APIs without breaking existing callers.

```rust
impl ClockSyncState {
    pub fn snapshot_at(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
        now_local_clock_ns: i64,
    ) -> Option<ClockSyncSnapshot>;

    pub fn fresh_estimate_at(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
        now_local_clock_ns: i64,
    ) -> Option<ClockTransformEstimate>;
}

impl ClockSyncHandle {
    pub fn snapshot_at(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
        now_local_clock_ns: i64,
    ) -> Option<ClockSyncSnapshot>;

    pub fn fresh_estimate_at(
        &self,
        local_clock_id: &str,
        remote_clock_id: &str,
        now_local_clock_ns: i64,
    ) -> Option<ClockTransformEstimate>;
}
```

- [ ] Compute medians deterministically by copying values into sorted `Vec<u64>` buffers. For even counts, use the upper median to avoid fractional nanoseconds.
- [ ] Keep `ClockSyncState::estimate(...)` and `ClockSyncHandle::estimate(...)` unchanged for compatibility. New domain-time code must use `snapshot_at(...)` or `fresh_estimate_at(...)`.
- [ ] Run focused tests.

```bash
cargo test -p auki-time clock_sync
```

Expected result: all clock-sync and domain-clock unit tests pass.

## Task 2: Preserve Domain-Source Quality Through Manager Handoff

Purpose: after Manager2 inherits domain time, its local monotonic clock can back the domain clock, but the inherited offset still has uncertainty. Do not accidentally advertise that inherited source as uncertainty zero.

- [ ] Add failing tests in `crates/auki-time/src/lib.rs`.

Test names:

```rust
domain_clock_estimate_adds_descriptor_uncertainty
domain_clock_estimate_rejects_total_uncertainty_overflow
```

The first test should compose:

- `local_to_backing.uncertainty_ns = 25`.
- `descriptor.backing_to_domain_uncertainty_ns = 40`.
- expected `DomainClockEstimate.uncertainty_ns == 65`.

- [ ] Extend `DomainClockDescriptor`.

```rust
pub struct DomainClockDescriptor {
    pub cluster_name: String,
    pub domain_clock_id: String,
    pub domain_clock_hash: String,
    pub backing_peer_id: String,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub backing_to_domain_offset_ns: i64,
    pub backing_to_domain_uncertainty_ns: u64,
    pub observed_at_backing_clock_ns: i64,
}
```

- [ ] Extend `DomainClockEstimate`.

```rust
pub struct DomainClockEstimate {
    pub backing_to_domain_uncertainty_ns: u64,
    pub local_to_backing_uncertainty_ns: u64,
    pub source_observed_at_backing_clock_ns: i64,
    // existing fields remain
}
```

- [ ] Add `DomainClockEstimateError::TotalUncertaintyOutOfRange` and use checked `u64` addition for total uncertainty.
- [ ] Extend `HeartbeatDomainClock` in `crates/auki-network/src/heartbeat_protocol.rs` with:

```rust
pub backing_to_domain_uncertainty_ns: u64,
pub observed_at_backing_clock_ns: i64,
```

- [ ] Update heartbeat wire-shape tests to assert the new fields are present.
- [ ] Update initial Manager source creation in `auki-domain` to advertise offset `0`, uncertainty `0`, observed-at current session clock.
- [ ] Update promoted Manager advertisement so it uses the inherited `DomainClockEstimate.total_offset_ns`, `DomainClockEstimate.uncertainty_ns`, and current local session clock as `observed_at_backing_clock_ns`.
- [ ] Run focused tests.

```bash
cargo test -p auki-time domain_clock_estimate
cargo test -p auki-network heartbeat
cargo test -p auki-domain domain_clock
```

Expected result: domain estimates preserve uncertainty across handoff and heartbeat serialization remains stable except for the intentional new fields.

## Task 3: Make Stale Domain Time Explicit

Purpose: callers should know when domain time is unavailable. There should be no fallback to UTC or stale transforms.

- [ ] Add failing `auki-domain` tests in `crates/auki-domain/src/cluster_manager.rs`.

Test names:

```rust
domain_clock_estimate_rejects_stale_peer_clock_snapshot
domain_time_now_reports_stale_peer_clock_snapshot
initial_manager_domain_time_does_not_need_peer_sample
promoted_manager_domain_time_uses_inherited_source_quality
```

- [ ] Extend `DomainClockEstimateUnavailable`.

```rust
StaleBackingEstimate {
    local_clock_id: String,
    backing_clock_id: String,
    age_ns: u64,
    max_sample_age_ns: u64,
}
```

Display text should be short and diagnostic-app friendly, for example:

```text
domain backing estimate stale: local <local> -> backing <backing>, age <age_ns>ns > <max_sample_age_ns>ns
```

- [ ] Change `estimate_cluster_domain_clock(...)` to call `clock_sync.snapshot_at(...)` for non-local backing clocks. If `snapshot.is_stale`, return `StaleBackingEstimate`. If no snapshot exists, keep returning `BackingEstimateUnavailable`.
- [ ] Keep local backing clocks available without peer samples when clock id/hash match. This covers the initial Manager and promoted Managers whose domain source is now backed by their own monotonic clock.
- [ ] Add an optional SDK accessor for diagnostics:

```rust
pub fn domain_clock_snapshot(&self) -> Result<DomainClockSnapshot, DomainClockEstimateUnavailable>;
```

Where `DomainClockSnapshot` includes:

```rust
pub estimate: DomainClockEstimate,
pub backing_snapshot: Option<ClockSyncSnapshot>,
pub is_stale: bool,
pub backing_peer_id: String,
```

For local backing clocks, `backing_snapshot` is `None` and `is_stale` is `false`.

- [ ] Run focused tests.

```bash
cargo test -p auki-domain domain_clock_estimate
cargo test -p auki-domain domain_time_now
```

Expected result: stale backing estimates make domain time unavailable, while self-backed domain clocks remain available.

## Task 4: Add Peer-ID Collision Guard

Purpose: cloned machines with the same peer id should fail loudly before they produce confusing join timeouts or split-brain diagnostics.

- [ ] Add failing tests in `crates/auki-domain/src/cluster_manager.rs`.

Test names:

```rust
join_cluster_rejects_discovery_manager_with_same_peer_id
join_cluster_error_message_names_peer_id_collision
```

- [ ] Add a new error variant.

```rust
#[error("peer-id collision: local peer {peer_id} is already the Discovery Manager for cluster {cluster:?}")]
PeerIdCollision {
    cluster: String,
    peer_id: PeerId,
}
```

- [ ] In `ClusterManager::join_cluster(...)`, after the Discovery lookup and before spawning the runtime, return `JoinClusterError::PeerIdCollision` when `entry.manager_peer_id == local_peer_id`.
- [ ] Keep this scoped to join. `create_cluster(...)` remains valid when the local peer is intentionally becoming the first Manager.
- [ ] Surface the exact error text in diagnostic app event log through the existing `ClusterJoinFailed(String)` path.
- [ ] Run focused tests.

```bash
cargo test -p auki-domain peer_id_collision
cargo test -p auki-diagnostic-app cluster_join_failed
```

Expected result: a cloned peer id produces a specific join failure instead of a timeout.

## Task 5: Gate Manager Election With Discovery State

Purpose: followers should not self-promote merely because their local heartbeat carrier went quiet if Discovery still has a fresh Manager hint.

- [ ] Extract pure decision helpers in `crates/auki-domain/src/cluster_manager.rs`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum ManagerLossDecision {
    DeferToDiscoveryManager(PeerId),
    FollowDiscoveryManager(PeerId),
    ElectLocally,
}

fn decide_manager_loss(
    cluster_name: &str,
    local_peer_id: PeerId,
    lost_manager: PeerId,
    discovery_entries: &[ClusterEntry],
) -> ManagerLossDecision;
```

Rules:

- If Discovery still lists `cluster_name` with `manager_peer_id == lost_manager`, return `DeferToDiscoveryManager(lost_manager)`.
- If Discovery lists `cluster_name` with a different `manager_peer_id`, return `FollowDiscoveryManager(manager_peer_id)`.
- If Discovery does not list `cluster_name`, return `ElectLocally`.

- [ ] Add unit tests.

Test names:

```rust
manager_loss_defers_when_discovery_still_lists_lost_manager
manager_loss_follows_when_discovery_lists_new_manager
manager_loss_elects_when_discovery_cluster_is_absent
```

- [ ] Change `handle_domain_peer_lost(...)` so a follower that loses the current Manager calls `discovery.list_clusters().await` before local election.
- [ ] On `DeferToDiscoveryManager`, do not mutate `manager_peer_id`, do not advertise a promoted domain source, and do not call `rotate_manager`. Clear or age the heartbeat watch entry so the peer retries after another timeout interval.
- [ ] On `FollowDiscoveryManager`, set `manager_peer_id` to the Discovery Manager, call `sync_heartbeat_targets(...)`, and wait for membership gossip or future Discovery refresh before considering promotion.
- [ ] On `ElectLocally`, continue with the existing membership-order election.
- [ ] On `discovery.list_clusters()` transport error, keep the current local election behavior but emit a diagnostic event string so the app can show that election was based on local state because Discovery was unreachable.
- [ ] Run focused tests.

```bash
cargo test -p auki-domain manager_loss
```

Expected result: split-brain-prone local promotions are deferred when Discovery still says the old Manager is alive.

## Task 6: Recover Promotion When Discovery Swept The Row

Purpose: if Discovery has already swept the cluster row, a legitimate successor should recreate or re-register the Manager hint instead of logging a failed `rotate_manager` and leaving Discovery empty.

- [ ] Add a helper around Manager registration.

```rust
async fn publish_manager_handoff(
    discovery: &DiscoveryClient,
    cluster_name: &str,
    local_peer_id: &PeerId,
    local_multiaddrs: &[Multiaddr],
) -> Result<ClusterEntry, DiscoveryError>;
```

Behavior:

- First call `rotate_manager(...)`.
- If it succeeds, return the entry.
- If it returns HTTP 404, call `create_cluster(...)`.
- If `create_cluster(...)` returns `Created(entry)`, return it.
- If `create_cluster(...)` returns `AlreadyExists`, call `list_clusters(...)` and return the matching entry if present.
- For other errors, return the original error.

- [ ] Add unit tests with a small fake or test-only decision helper if mocking `DiscoveryClient` directly is awkward. Keep the HTTP-backed integration for final proof.

Test names:

```rust
publish_manager_handoff_recreates_missing_discovery_row
publish_manager_handoff_handles_create_conflict_by_relisting
publish_manager_handoff_returns_original_non_404_error
```

- [ ] Replace direct `discovery.rotate_manager(...)` in promotion code with `publish_manager_handoff(...)`.
- [ ] Emit diagnostic events for:

```text
manager handoff published
manager handoff recreated discovery row
manager handoff failed: <error>
```

- [ ] Run focused tests.

```bash
cargo test -p auki-domain publish_manager_handoff
cargo test -p auki-domain manager_loss
```

Expected result: elected successors keep Discovery usable even when the old row disappeared first.

## Task 7: Add SDK Diagnostic Events

Purpose: the diagnostic app needs structured-ish strings today, and the SDK needs enough internal events to explain time-sync and election behavior.

- [ ] Add a small domain diagnostic event channel in `ClusterManager`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterDiagnosticEvent {
    DomainClockSourceObserved { backing_peer_id: String, backing_clock_id: String },
    DomainClockEstimateUpdated { backing_peer_id: String, offset_ns: i64, uncertainty_ns: u64 },
    DomainClockEstimateStale { backing_peer_id: String, age_ns: u64, max_sample_age_ns: u64 },
    ManagerHeartbeatLost { manager_peer_id: PeerId },
    ManagerElectionDeferred { manager_peer_id: PeerId },
    ManagerElectionWon { previous_manager_peer_id: PeerId, new_manager_peer_id: PeerId },
    ManagerElectionFollowed { new_manager_peer_id: PeerId },
    PeerIdCollisionDetected { peer_id: PeerId, cluster_name: String },
}
```

- [ ] Add an accessor:

```rust
pub fn take_diagnostic_events(&self) -> Vec<ClusterDiagnosticEvent>;
```

Use a bounded `VecDeque` internally with a small cap such as 128 events.

- [ ] Emit events from:

- heartbeat domain-clock source observation.
- heartbeat NTP sample observation when it changes the best estimate.
- stale domain-clock reads.
- Manager heartbeat timeout.
- Discovery-deferred election.
- successful local promotion.
- follow-Discovery-manager transition.
- peer-id collision guard.

- [ ] Add unit tests that exercise the queue cap and drain behavior.

```bash
cargo test -p auki-domain diagnostic_events
```

Expected result: SDK callers can drain recent domain/election events without parsing stderr.

## Task 8: Upgrade The Diagnostic App

Purpose: make the demo answer the questions a human will ask while looking at two laptops: "Who is Manager?", "What clock backs domain time?", "How good is the estimate?", "Is it stale?", and "Why did role change?"

- [ ] Extend `RuntimeSnapshot` and `ClusterSnapshot` in `examples/diagnostic-app/src/sdk_runtime.rs`.

Add fields:

```rust
pub domain_backing_peer_suffix: Option<String>,
pub domain_backing_clock_id: Option<String>,
pub domain_sample_count: Option<usize>,
pub domain_age_ns: Option<u64>,
pub domain_offset_jitter_ns: Option<u64>,
pub domain_uncertainty_jitter_ns: Option<u64>,
pub domain_min_uncertainty_ns: Option<u64>,
pub domain_median_uncertainty_ns: Option<u64>,
pub domain_max_uncertainty_ns: Option<u64>,
pub domain_stale: bool,
```

- [ ] Change diagnostic refresh to use `manager.domain_clock_snapshot()` for the app's Domain flash availability and quality display. Do not infer "synced" from `domain_clock_estimate()` alone once the snapshot accessor exists.
- [ ] Drain `manager.take_diagnostic_events()` during snapshot refresh and append concise event strings to the existing event log.
- [ ] Gate Domain flash mode on `domain_now_ns.is_some()` and `domain_stale == false`.
- [ ] Update `examples/diagnostic-app/src/ui.rs` to show:

- Domain status: synced, unavailable, stale.
- Backing peer suffix.
- Offset.
- Uncertainty.
- Sample count.
- Age.
- Offset jitter.
- Uncertainty jitter.
- Min/median/max uncertainty.
- Recent election/time-sync events.

- [ ] Add snapshot/app-state tests.

Test names:

```rust
cluster_snapshot_carries_domain_quality_fields
domain_flash_mode_disables_when_snapshot_is_stale
diagnostic_events_are_appended_to_event_log
```

- [ ] Run tests.

```bash
cargo test -p auki-diagnostic-app
```

Expected result: the diagnostic app clearly distinguishes unavailable, stale, and synced domain time.

## Task 9: Add Live Regression Coverage

Purpose: protect the behavior that was observed during the two-laptop demo: peers can cluster, lose contact, avoid split-brain when Discovery still points at the Manager, and recover when Discovery legitimately changes.

- [ ] Add or update ignored integration tests in `crates/auki-domain/tests/cluster_manager_integration.rs`.

Test names:

```rust
two_peer_domain_time_reports_quality_after_heartbeats
follower_defers_promotion_when_discovery_still_lists_manager
successor_recreates_discovery_row_after_sweep
domain_quality_survives_manager_handoff
```

- [ ] Each ignored test should accept the same Discovery URL convention already used by existing ignored tests.
- [ ] The two-peer quality test should wait until the follower has:

- `domain_clock_snapshot().estimate.total_offset_ns`.
- `uncertainty_ns > 0 || sample_count > 0`.
- `domain_stale == false`.

- [ ] The deferral test should simulate heartbeat loss while Discovery still lists the old Manager and assert the follower does not become Manager.
- [ ] The recreate-row test should remove or allow Discovery to sweep the old row, then assert the successor publishes a row again.
- [ ] Run the ignored tests manually against local or LAN Discovery.

```bash
cargo test -p auki-domain --test cluster_manager_integration two_peer_domain_time_reports_quality_after_heartbeats -- --ignored --nocapture --test-threads=1
cargo test -p auki-domain --test cluster_manager_integration follower_defers_promotion_when_discovery_still_lists_manager -- --ignored --nocapture --test-threads=1
cargo test -p auki-domain --test cluster_manager_integration successor_recreates_discovery_row_after_sweep -- --ignored --nocapture --test-threads=1
cargo test -p auki-domain --test cluster_manager_integration domain_quality_survives_manager_handoff -- --ignored --nocapture --test-threads=1
```

Expected result: ignored live tests pass when a Discovery service is reachable.

## Task 10: Documentation, Changelogs, And Full Verification

- [ ] Update implemented-status docs:

- `crates/auki-time/src/readme.md`.
- `crates/auki-network/src/readme.md`.
- `crates/auki-domain/src/readme.md`.
- `examples/diagnostic-app/src/readme.md` if present, otherwise `examples/diagnostic-app/README.md` if present.

- [ ] Update sprint files that mention next work:

- `crates/auki-time/src/sprint.md` if present.
- `crates/auki-domain/src/sprint.md`.
- `examples/diagnostic-app/src/sprint.md` if present.

- [ ] Append changelog entries at the most-specific touched folders and propagate upward immediately, following `AGENTS.md`.
- [ ] Run formatting and checks.

```bash
cargo fmt --check
cargo test -p auki-time
cargo test -p auki-network
cargo test -p auki-domain
cargo test -p auki-diagnostic-app
cargo check --workspace
git diff --check
```

Expected result: format, focused tests, workspace check, and diff whitespace checks all pass.

## Self-Review Checklist

- [ ] No code path falls back to UTC or wall clock when domain time is stale or unavailable.
- [ ] `auki-time` owns NTP sample selection, sample-window stats, jitter, and stale checks.
- [ ] `ClusterManager` does not compute NTP offsets by hand.
- [ ] Heartbeat remains the single carrier for observations and domain-clock source metadata.
- [ ] Promoted Managers preserve inherited domain uncertainty.
- [ ] The diagnostic app can tell apart "not enough samples", "stale samples", "Discovery says another Manager", and "synced".
- [ ] Peer-id collisions produce a specific error visible in the diagnostic app.
- [ ] Election code consults Discovery before self-promotion on Manager heartbeat loss.
- [ ] Successor promotion leaves Discovery with a usable Manager row.
- [ ] Live/ignored tests cover both domain quality and election safety.
