# SDK Timekeeping Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make SDK timekeeping explicit, reusable, and registry-bound before adding heartbeat-based time sync.

**Architecture:** `auki-time` grows from "TimeTransform Log sampler" into the shared home for clock-reading primitives. It provides a `SessionClock` value that owns a `ClockRegistryEntry` identity plus a monotonic `now_ns()` reader. Registry identities are anchored to the daemon's libp2p `peer_id`, not to the stale MAC-derived machine-id convention. `auki-domain::ClusterManager` stops hand-rolling `Instant::elapsed()` and uses `SessionClock` for `ParticipantInfo.session_now_ns`. Later heartbeat time sync can consume the same `SessionClock` instead of inventing a heartbeat-specific clock abstraction.

**Tech Stack:** Rust, `std::time::Instant`, `auki-registry::ClockRegistryEntry`, `auki-time`, `auki-domain`, PyO3 follow-up only if Python needs to construct clocks directly.

---

## File Structure

- Modify `crates/auki-time/Cargo.toml`: depend on `auki-registry`.
- Modify `crates/auki-time/src/lib.rs`: add `SessionClock`, `SessionClockId`, and helpers for peer-id anchored monotonic epoch identity.
- Modify `crates/auki-time/README.md` and `crates/auki-time/src/readme.md`: document the crate as shared clock primitives plus TimeTransform sampling.
- Modify `crates/auki-domain/Cargo.toml`: depend on `auki-time`.
- Modify `crates/auki-domain/src/cluster_manager.rs`: replace `session_started: Instant` with `session_clock: SessionClock`.
- Modify `crates/auki-domain/README.md`, `crates/auki-domain/src/README.md`, and `crates/auki-domain/src/sprint.md`: document that ClusterManager reads session time through the shared SDK clock primitive.
- Modify `docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md`: mark heartbeat sync as dependent on this foundation plan.
- Modify changelogs at the touched leaves and propagate one-liners upward: `crates/auki-time/changelog.md`, `crates/auki-domain/changelog.md`, `crates/changelog.md`, `docs/superpowers/changelog.md`, `docs/changelog.md`, and `changelog.md`.

---

### Task 1: Add SessionClock To auki-time

**Files:**
- Modify: `crates/auki-time/Cargo.toml`
- Modify: `crates/auki-time/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add tests proving a `SessionClock` has a peer-id anchored registry identity, a hash, monotonic `now_ns()`, and a unique epoch marker in the registry entry:

```rust
#[test]
fn session_clock_builds_peer_anchored_registry_entry_with_epoch_marker() {
    let peer_id = "12D3KooWPeerExample";
    let clock = SessionClock::new(
        peer_id,
        "session-123",
        "monotonic",
    );

    let entry = clock.registry_entry();
    assert_eq!(entry.clock_id, "12D3KooWPeerExample/session-123/monotonic");
    match entry.body {
        ClockBody::MonotonicClock(meta) => {
            assert_eq!(meta.unit, "ns");
            assert!(meta.monotonic);
            assert_eq!(meta.scope, Scope::DeviceLocal);
            assert_eq!(meta.epoch.as_deref(), Some("session-123"));
        }
        ClockBody::UtcClock(_) => panic!("session clock must be monotonic"),
    }
    assert_eq!(clock.clock_hash(), entry.hash());
}

#[test]
fn session_clock_now_is_monotonic() {
    let clock = SessionClock::new("12D3KooWPeerExample", "session-123", "monotonic");
    let a = clock.now_ns();
    let b = clock.now_ns();
    assert!(b >= a);
}
```

- [ ] **Step 2: Run red test**

Run: `cargo test -p auki-time session_clock`

Expected: FAIL because `SessionClock` does not exist.

- [ ] **Step 3: Implement SessionClock**

Add `auki-registry = { path = "../auki-registry" }` to `crates/auki-time/Cargo.toml`.

Add:

```rust
#[derive(Debug, Clone)]
pub struct SessionClock {
    registry_entry: ClockRegistryEntry,
    started: Instant,
}

impl SessionClock {
    pub fn new(
        peer_id: impl Into<String>,
        session_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let peer_id = peer_id.into();
        let session_id = session_id.into();
        let name = name.into();
        let registry_entry = ClockRegistryEntry {
            clock_id: format!("{peer_id}/{session_id}/{name}"),
            body: ClockBody::MonotonicClock(ClockMeta {
                unit: "ns".into(),
                monotonic: true,
                epoch: Some(session_id),
                scope: Scope::DeviceLocal,
            }),
        };
        Self {
            registry_entry,
            started: Instant::now(),
        }
    }

    pub fn now_ns(&self) -> u64 {
        self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    pub fn now_i64_ns(&self) -> i64 {
        self.started.elapsed().as_nanos().min(i64::MAX as u128) as i64
    }

    pub fn clock_id(&self) -> &str {
        &self.registry_entry.clock_id
    }

    pub fn clock_hash(&self) -> String {
        self.registry_entry.hash()
    }

    pub fn registry_entry(&self) -> ClockRegistryEntry {
        self.registry_entry.clone()
    }
}
```

Use `epoch = Some(session_id)` for monotonic session clocks to encode the monotonic epoch/lifetime. The `clock_id` MUST be anchored to the daemon's `peer_id` so peers can identify the source without parsing stale MAC-derived naming conventions. This is the key invariant needed before heartbeat sync.

- [ ] **Step 4: Run green tests**

Run: `cargo test -p auki-time session_clock`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-time/Cargo.toml crates/auki-time/src/lib.rs
git commit -m "feat: add session clock primitive"
```

---

### Task 2: Move ClusterManager Session Time Onto SessionClock

**Files:**
- Modify: `crates/auki-domain/Cargo.toml`
- Modify: `crates/auki-domain/src/cluster_manager.rs`

- [ ] **Step 1: Write failing tests**

Update existing `participant_info` tests to assert `ClusterManager` publishes the SDK-minted peer-id anchored session clock id/hash, while `session_now_ns` advances from the shared clock primitive:

```rust
#[test]
fn participant_info_uses_session_clock_primitive() {
    let daemon = sample_daemon_info();
    let clock = SessionClock::new(
        fixed_peer_id(1).to_string(),
        daemon.session_id.clone(),
        "monotonic",
    );

    let info = build_participant_info_with_clock(
        &daemon,
        fixed_peer_id(1),
        &manager_peer_id,
        &membership,
        &clock,
        &cluster_joined_at_ns,
    );

    assert_eq!(
        info.session_clock_id,
        format!("{}/session-123/monotonic", fixed_peer_id(1)),
    );
    assert_eq!(info.session_clock_hash, clock.clock_hash());
    assert!(info.session_now_ns <= clock.now_ns());
}
```

- [ ] **Step 2: Run red test**

Run: `cargo test -p auki-domain participant_info_uses_session_clock_primitive`

Expected: FAIL because `ClusterManager` still stores `session_started: Instant`.

- [ ] **Step 3: Add dependency and replace field**

Add `auki-time = { path = "../auki-time" }` to `crates/auki-domain/Cargo.toml`.

Change the `ClusterManager` field:

```rust
session_clock: SessionClock,
```

Replace construction sites:

```rust
let session_clock = SessionClock::new(
    local_peer_id.to_string(),
    daemon_info.session_id.clone(),
    "monotonic",
);
```

The SDK-minted `SessionClock` becomes the source of truth for `DaemonInfo.session_clock_id/hash` inside `ClusterManager`. Keep the input fields temporarily for compatibility, but construction should overwrite or normalize them in the SDK-owned `ParticipantInfo` path rather than trusting stale MAC-based caller strings. If current live callers pass placeholder hashes, do not fail construction yet; log or document the mismatch as migration debt.

- [ ] **Step 4: Update participant_info builder**

Change `build_participant_info(...)` to take `&SessionClock` instead of `session_started: Instant`:

```rust
let session_now_ns = session_clock.now_ns();
```

Keep `cluster_joined_at_ns` semantics unchanged: set it lazily when membership first contains a non-self peer, using the same `session_now_ns`.

- [ ] **Step 5: Run green tests**

Run:

```bash
cargo test -p auki-domain participant_info
cargo test -p auki-domain cluster_manager::tests
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-domain/Cargo.toml crates/auki-domain/src/cluster_manager.rs
git commit -m "refactor: use shared session clock in cluster manager"
```

---

### Task 3: Document Clock Ownership And Epoch Rules

**Files:**
- Modify: `crates/auki-time/README.md`
- Modify: `crates/auki-time/src/readme.md`
- Modify: `crates/auki-domain/README.md`
- Modify: `crates/auki-domain/src/README.md`
- Modify: `crates/auki-domain/src/sprint.md`

- [ ] **Step 1: Update auki-time docs**

Document:

- `SessionClock` is the SDK's shared session-monotonic clock primitive.
- `SessionClock` owns a `ClockRegistryEntry`.
- `SessionClock` ids are anchored to `peer_id`, e.g. `<peer_id>/<session_id>/monotonic`; the first path segment of SDK-minted registry IDs is expected to be the authoring peer id.
- The old `<platform-tag>-<machine-id>/...` convention is stale for new SDK-minted session clocks.
- Monotonic session clocks must encode one epoch/lifetime in the registry entry.
- `SystemClock` remains the local monotonic/UTC pair used by `local_clock_read`.
- Time sync should consume `SessionClock` rather than creating heartbeat-specific clock abstractions.

- [ ] **Step 2: Update auki-domain docs**

Document:

- `ClusterManager` reads `ParticipantInfo.session_now_ns` from `SessionClock`.
- `ClusterManager` should publish SDK-minted `SessionClock` id/hash in `ParticipantInfo`.
- `DaemonInfo.session_clock_id/hash` are compatibility inputs until the constructor can stop accepting caller-built session clock ids.
- Heartbeat time sync is deferred until `SessionClock` is the source for heartbeat timestamps.

- [ ] **Step 3: Run docs sanity checks**

Run:

```bash
rg -n "session_started|Instant::elapsed|HeartbeatClock" crates/auki-domain crates/auki-time docs/superpowers/plans
git diff --check
```

Expected: no `HeartbeatClock` references; remaining `Instant::elapsed` references are only internal to `SessionClock` or unrelated liveness timers.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-time/README.md crates/auki-time/src/readme.md
git add crates/auki-domain/README.md crates/auki-domain/src/README.md crates/auki-domain/src/sprint.md
git commit -m "docs: clarify sdk session clock ownership"
```

---

### Task 4: Defer Heartbeat Time Sync Behind Timekeeping Foundation

**Files:**
- Modify: `docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md`

- [ ] **Step 1: Add dependency note**

At the top of the heartbeat sync plan, add:

```markdown
> **Dependency:** Implement `2026-05-19-sdk-timekeeping-foundation.md` first. Heartbeat time sync must consume `SessionClock` for timestamp identity and readings; it must not introduce a parallel heartbeat clock abstraction.
```

- [ ] **Step 2: Replace timestamp-source construction**

In the heartbeat plan's ClusterManager wiring task, replace ad hoc `session_started.elapsed()` timestamp source with:

```rust
let heartbeat_timestamps = HeartbeatTimestampSource {
    clock_id: self.session_clock.clock_id().to_string(),
    clock_hash: self.session_clock.clock_hash(),
    now_ns: {
        let session_clock = self.session_clock.clone();
        Arc::new(move || session_clock.now_i64_ns())
    },
};
```

- [ ] **Step 3: Run plan scan**

Run:

```bash
rg -n "session_started|Instant::elapsed|HeartbeatClock" docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md
git diff --check
```

Expected: no matches for `session_started`, `Instant::elapsed`, or `HeartbeatClock`.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md
git commit -m "docs: defer heartbeat sync behind session clock foundation"
```

---

### Task 5: Changelog And Verification

**Files:**
- Modify: `crates/auki-time/changelog.md`
- Modify: `crates/auki-domain/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `docs/superpowers/changelog.md`
- Modify: `docs/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Add changelog entries**

Add detailed leaf entries to `auki-time` and `auki-domain`, one-line crate summary to `crates/changelog.md`, docs artifact summary to `docs/superpowers/changelog.md` and `docs/changelog.md`, and root one-liner.

- [ ] **Step 2: Run verification**

Run:

```bash
cargo test -p auki-time
cargo test -p auki-domain
git diff --check
```

Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add crates/auki-time/changelog.md crates/auki-domain/changelog.md crates/changelog.md
git add docs/superpowers/changelog.md docs/changelog.md changelog.md
git commit -m "docs: record sdk timekeeping foundation plan"
```

---

## Self-Review

- Spec coverage: The plan creates one reusable session-clock primitive, moves ClusterManager's current session time source onto it, documents clock ownership, and defers heartbeat sync until it can consume this primitive.
- Placeholder scan: No `TBD`, `TODO`, or vague "add tests later" placeholders remain.
- Type consistency: `SessionClock` owns `ClockRegistryEntry` identity and exposes `now_ns()` / `now_i64_ns()` for consumers that need unsigned participant info or signed timestamp math.
- Scope check: This plan intentionally does not implement heartbeat synchronization. It removes the ambiguous timekeeping substrate first.
