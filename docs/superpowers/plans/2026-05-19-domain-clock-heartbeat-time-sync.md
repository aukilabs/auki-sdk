# Domain Clock Heartbeat Time Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Dependency:** Implement `[2026-05-19-sdk-timekeeping-foundation.md](2026-05-19-sdk-timekeeping-foundation.md)` first. Heartbeat time sync must consume `SessionClock` for timestamp identity and readings; it must not introduce a parallel heartbeat clock abstraction.

**Goal:** Give every cluster peer a live `TimeTransformEntry` estimate from its session clock into the cluster's stable domain clock, using the existing `/auki/heartbeat/0.0.1` carrier.

**Architecture:** `auki-network` remains transport plumbing: heartbeat frames carry NTP-style send/receive/echo timestamps and the runtime emits raw timing samples. `auki-time` owns the NTP calculation, sample filtering, and local TimeTransform production. Every peer maintains its own estimator and produces its own `local SessionClock -> domain-clock` transform. `auki-domain::ClusterManager` is only the source of domain-clock authority: it defines the domain clock identity, reports which backing peer/session clock backs it, and carries the handoff continuity offset. The first domain clock source is the initial Manager's session-monotonic clock with a zero offset.

**Clock Target Semantics:** The heartbeat frame carries one clock identity pair: `clock_id` / `clock_hash` names the sender's timestamp clock used for `sent_at_clock_ns` and echo receive timestamps. The heartbeat does not carry the domain clock id or hash. The shared logical target clock lives in `DomainClockSource` and in any future TimeTransform manifest that consumes the estimate. Every peer in the cluster should use the same domain-clock registry entry and therefore the same domain clock hash for a given domain-clock version. `ClusterManager` exposes the domain source as `DomainClockSource.cluster_name == ClusterManager::cluster_name()` and `DomainClockSource.clock_id == "<cluster-name>/domain-clock"`, plus the current backing peer id, backing clock id/hash, and `domain_offset_ns`. Each peer feeds heartbeat samples from the current backing peer into `auki-time`, which estimates `local SessionClock -> backing SessionClock`, then adds `DomainClockSource.domain_offset_ns` to produce `local SessionClock -> domain-clock`. If the heartbeat sender does not match the current `DomainClockSource.backing_peer_id` and `backing_clock_id` / `backing_clock_hash`, the peer should not use that sample for the current domain clock. The Manager is normally the backing peer; the initial Manager produces its own transform from the domain source offset, initially zero. The backing peer does not compute or filter transforms for followers.

**Monotonic Epoch Invariant:** NTP-style offset math works for monotonic clocks with unrelated zero points; the offset is the bridge between those zero points. It only remains meaningful if each `clock_id` / `clock_hash` names one specific monotonic epoch/lifetime. A daemon must not restart a monotonic clock at zero while reusing the same clock identity. If the session clock resets, the session clock registry entry must get a new identity or hash, for example by including the session id or another boot/session epoch marker in the Clock Registry entry.

**Tech Stack:** Rust, tokio, serde JSON heartbeat framing, libp2p stream runtime, `auki-time` NTP estimator, `auki-datatypes::time_transform::TimeTransformEntry`, `auki-registry::ClockRegistryEntry`, PyO3 bindings in `auki-domain-py`.

---

## File Structure

- Modify `crates/auki-network/src/heartbeat_protocol.rs`: extend the heartbeat wire payload with sender-clock NTP echo fields while keeping framed JSON.
- Modify `crates/auki-network/src/network_runtime.rs`: add an explicit heartbeat timestamp source, track outbound heartbeat sequence timestamps, and emit `HeartbeatTimingSample` events.
- Modify `crates/auki-network/src/README.md` and `crates/auki-network/README.md`: document that network emits raw timing samples but does not define the domain clock.
- Create `crates/auki-time/src/ntp.rs`: own NTP math, filtering, stale-state, and `TimeTransformEntry` conversion.
- Modify `crates/auki-time/src/lib.rs` and `crates/auki-time/README.md`: expose and document the NTP transform producer.
- Modify `crates/auki-domain/src/cluster_manager.rs`: pass the session clock sampler to `NetworkRuntime` and expose the domain clock source. Do not store per-peer NTP estimates in `ClusterManager`.
- Modify `crates/auki-domain/src/lib.rs`: re-export domain-clock source value types.
- Modify `crates/auki-domain/README.md`, `crates/auki-domain/src/README.md`, and `crates/auki-domain/src/sprint.md`: document the domain-clock heartbeat plan and current surface.
- Modify `bindings/python/auki-domain-py/src/lib.rs`, `bindings/python/auki-domain-py/README.md`, and `bindings/python/auki-domain-py/src/README.md`: expose a minimal Python snapshot API.
- Modify changelogs at the touched leaves and propagate one-liners upward: `crates/auki-network/changelog.md`, `crates/auki-domain/changelog.md`, `crates/changelog.md`, `bindings/python/auki-domain-py/changelog.md`, `bindings/python/changelog.md`, `bindings/changelog.md`, and `changelog.md`.

---

### Task 1: Extend Heartbeat Wire Payload

**Files:**

- Modify: `crates/auki-network/src/heartbeat_protocol.rs`
- **Step 1: Write failing wire-shape tests**

Add tests for the new fields and NTP echo round-trip. Keep `sent_at_unix_ns` in the payload as legacy/debug provenance, but add explicit Clock Registry binding fields only for the sender timestamp clock.

```rust
#[test]
fn heartbeat_wire_shape_includes_ntp_echo_fields() {
    let msg = Heartbeat {
        sent_at_unix_ns: 1_715_423_400_000_000_000,
        // This is the sender's timestamp clock. If this frame is
        // sent by the current backing peer, auki-domain can treat
        // this clock as the concrete source for the domain clock.
        // If this frame is sent by a follower, it is the follower's
        // own source clock; heartbeat itself does not name a domain
        // clock target.
        clock_id: "12D3KooWPeerExample/session-123/monotonic".into(),
        clock_hash: "abc123".into(),
        sequence: 7,
        sent_at_clock_ns: 10_000,
        echo: Some(HeartbeatEcho {
            sequence: 6,
            received_at_clock_ns: 20_000,
        }),
    };

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"sent_at_unix_ns\":"));
    assert!(json.contains("\"clock_id\":"));
    assert!(json.contains("\"clock_hash\":"));
    assert!(json.contains("\"sequence\":"));
    assert!(json.contains("\"sent_at_clock_ns\":"));
    assert!(json.contains("\"echo\":"));
    assert!(json.contains("\"received_at_clock_ns\":"));
}
```

- **Step 2: Run red test**

Run: `cargo test -p auki-network heartbeat_protocol --features swarm`

Expected: FAIL because `Heartbeat::clock_id`, `Heartbeat::clock_hash`, `Heartbeat::sequence`, `Heartbeat::sent_at_clock_ns`, and `HeartbeatEcho` do not exist.

- **Step 3: Implement the new heartbeat types**

Add the echo type and fields:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    pub sent_at_unix_ns: i64,
    /// The Clock Registry id for `sent_at_clock_ns` and any echo
    /// timestamp in this frame. `ClusterManager` binds this to the
    /// daemon's SDK-owned `SessionClock`.
    pub clock_id: String,
    pub clock_hash: String,
    pub sequence: u64,
    /// Sender's reading of `clock_id` at frame-write time.
    pub sent_at_clock_ns: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<HeartbeatEcho>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeartbeatEcho {
    pub sequence: u64,
    /// Receiver's reading of the same clock named by the enclosing
    /// heartbeat's `clock_id` / `clock_hash` when it received the
    /// echoed sequence.
    pub received_at_clock_ns: i64,
}
```

Update the existing `sample()` helper to include deterministic values. Keep `MAX_HEARTBEAT_FRAME_BYTES` at `1024`; the expanded JSON remains well below the cap.

Add a doc comment to `Heartbeat::clock_id` / `clock_hash` that says the identity must name a single monotonic epoch. A repeated clock id with a restarted zero point is invalid input for TimeTransform production. Say explicitly in the module docs that the domain-clock target is not a heartbeat field; callers compare Manager heartbeat samples against their current `DomainClockSource.backing_clock_id` / `backing_clock_hash`.

- **Step 4: Run green test**

Run: `cargo test -p auki-network heartbeat_protocol --features swarm`

Expected: PASS.

- **Step 5: Commit**

```bash
git add crates/auki-network/src/heartbeat_protocol.rs
git commit -m "feat: add heartbeat ntp echo payload"
```

---

### Task 2: Emit Raw Heartbeat Timing Samples

**Files:**

- Modify: `crates/auki-network/src/network_runtime.rs`
- **Step 1: Write failing unit tests for NTP sample extraction**

Add a small pure helper near the heartbeat loop:

```rust
fn build_timing_sample(
    peer_id: PeerId,
    remote_clock_id: impl Into<String>,
    remote_clock_hash: impl Into<String>,
    local_send_ns: i64,
    remote_receive_ns: i64,
    remote_send_ns: i64,
    local_receive_ns: i64,
) -> HeartbeatTimingSample {
    HeartbeatTimingSample {
        peer_id,
        remote_clock_id: remote_clock_id.into(),
        remote_clock_hash: remote_clock_hash.into(),
        local_send_ns,
        remote_receive_ns,
        remote_send_ns,
        local_receive_ns,
    }
}
```

Then test the standard NTP offset formula inputs are preserved exactly for the time layer:

```rust
#[test]
fn heartbeat_timing_sample_preserves_four_ntp_timestamps() {
    let peer = fixed_peer_id(1);
    let s = build_timing_sample(
        peer,
        "12D3KooWPeerExample/session-123/monotonic",
        "abc123",
        1_000,
        2_100,
        2_300,
        1_250,
    );

    assert_eq!(s.peer_id, peer);
    assert_eq!(s.remote_clock_id, "12D3KooWPeerExample/session-123/monotonic");
    assert_eq!(s.remote_clock_hash, "abc123");
    assert_eq!(s.local_send_ns, 1_000);
    assert_eq!(s.remote_receive_ns, 2_100);
    assert_eq!(s.remote_send_ns, 2_300);
    assert_eq!(s.local_receive_ns, 1_250);
}
```

- **Step 2: Run red test**

Run: `cargo test -p auki-network network_runtime::tests::heartbeat_timing_sample_preserves_four_ntp_timestamps --features swarm`

Expected: FAIL because `HeartbeatTimingSample` does not exist.

- **Step 3: Add event and timestamp-source types**

Add a type alias and event variant:

```rust
pub type HeartbeatNowNs = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Clone)]
pub struct HeartbeatTimestampSource {
    pub clock_id: String,
    pub clock_hash: String,
    pub now_ns: HeartbeatNowNs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTimingSample {
    pub peer_id: PeerId,
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub local_send_ns: i64,
    pub remote_receive_ns: i64,
    pub remote_send_ns: i64,
    pub local_receive_ns: i64,
}

pub enum PeerLivenessEvent {
    Connected { peer_id: PeerId },
    Disconnected { peer_id: PeerId },
    HeartbeatReceived { peer_id: PeerId },
    HeartbeatTimingSample(HeartbeatTimingSample),
    HeartbeatStreamClosed { peer_id: PeerId },
}
```

Change the existing `NetworkRuntime::spawn(...)` signature to require `HeartbeatTimestampSource`. Heartbeat timestamping is part of the default runtime contract; `auki-network` receives SDK-owned timestamp identity and readings from its caller instead of inventing them internally.

```rust
pub fn spawn(
    swarm: libp2p::Swarm<swarm::Behaviour>,
    allowed_peers: Vec<AllowedPeer>,
    stream_provider: stream_runtime::StreamProvider,
    heartbeat_timestamps: HeartbeatTimestampSource,
) -> Result<Self, SpawnError> {
    Self::spawn_inner(swarm, allowed_peers, stream_provider, heartbeat_timestamps)
}
```

- **Step 4: Wire the heartbeat loop**

Inside `run_heartbeat_pair`, maintain:

```rust
let mut next_sequence: u64 = 1;
let mut sent: HashMap<u64, i64> = HashMap::new();
let mut pending_echo: Option<HeartbeatEcho> = None;
```

On write:

```rust
let seq = next_sequence;
next_sequence = next_sequence.wrapping_add(1).max(1);
let sent_at_clock_ns = (heartbeat_timestamps.now_ns)();
sent.insert(seq, sent_at_clock_ns);
let hb = Heartbeat {
    sent_at_unix_ns: unix_now_ns(),
    clock_id: heartbeat_timestamps.clock_id.clone(),
    clock_hash: heartbeat_timestamps.clock_hash.clone(),
    sequence: seq,
    sent_at_clock_ns,
    echo: pending_echo.take(),
};
if write_heartbeat(&mut writer, &hb).await.is_err() {
    break;
}
```

On read:

```rust
let local_receive_ns = (heartbeat_timestamps.now_ns)();
let hb = match read_heartbeat(&mut reader).await {
    Ok(hb) => hb,
    Err(_) => break,
};
pending_echo = Some(HeartbeatEcho {
    sequence: hb.sequence,
    received_at_clock_ns: local_receive_ns,
});
let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatReceived { peer_id: peer });
if let Some(echo) = hb.echo {
    if let Some(local_send_ns) = sent.remove(&echo.sequence) {
        let _ = liveness_tx.try_send(PeerLivenessEvent::HeartbeatTimingSample(
            HeartbeatTimingSample {
                peer_id: peer,
                remote_clock_id: hb.clock_id,
                remote_clock_hash: hb.clock_hash,
                local_send_ns,
                remote_receive_ns: echo.received_at_clock_ns,
                remote_send_ns: hb.sent_at_clock_ns,
                local_receive_ns,
            },
        ));
    }
}
```

Prune `sent` when it grows beyond 64 entries by removing the oldest sequence keys. This avoids unbounded memory if a peer stops echoing.

- **Step 5: Run green tests**

Run:

```bash
cargo test -p auki-network heartbeat_protocol --features swarm
cargo test -p auki-network network_runtime::tests --features swarm
```

Expected: PASS.

- **Step 6: Commit**

```bash
git add crates/auki-network/src/network_runtime.rs
git commit -m "feat: emit heartbeat timing samples"
```

---

### Task 3: Add NTP Transform Core To auki-time

**Files:**

- Create: `crates/auki-time/src/ntp.rs`
- Modify: `crates/auki-time/src/lib.rs`
- Modify: `crates/auki-time/Cargo.toml`
- **Step 1: Write failing tests for NTP math**

Create `ntp.rs` with tests first. The core formula estimates `remote_clock_ns - local_clock_ns`:

```text
offset_ns = ((remote_receive_ns - local_send_ns) + (remote_send_ns - local_receive_ns)) / 2
uncertainty_ns = (local_receive_ns - local_send_ns) - (remote_send_ns - remote_receive_ns)
```

The NTP core is generic. It does not know about Managers, followers, or domains. Domain-clock conversion is a caller-supplied target offset: for heartbeat domain sync, each peer estimates `backing SessionClock - local SessionClock`, then adds `DomainClockSource.domain_offset_ns` to produce `domain-clock - local SessionClock`.

```rust
#[test]
fn ntp_formula_returns_remote_minus_local_offset() {
    let sample = NtpSample {
        remote_clock_id: "manager/session-monotonic".into(),
        remote_clock_hash: "managerhash".into(),
        local_send_ns: 1_000,
        remote_receive_ns: 2_080,
        remote_send_ns: 2_120,
        local_receive_ns: 1_060,
    };

    let estimate = NtpEstimate::from_sample(sample).unwrap();
    assert_eq!(estimate.offset_ns, 1_070);
    assert_eq!(estimate.uncertainty_ns, 20);
}

#[test]
fn estimate_adds_target_offset_when_exporting_transform() {
    let estimate = NtpEstimate {
        remote_clock_id: "manager/session-monotonic".into(),
        remote_clock_hash: "managerhash".into(),
        offset_ns: 1_070,
        uncertainty_ns: 20,
        measured_at_local_clock_ns: 1_030,
    };

    let entry = estimate.to_time_transform_entry(500);
    assert_eq!(entry.offset_ns, 1_570);
    assert_eq!(entry.uncertainty_ns, 20);
}
```

- **Step 2: Run red test**

Run: `cargo test -p auki-time ntp`

Expected: FAIL because `ntp` does not exist.

- **Step 3: Implement the core types**

`auki-time` already re-exports `TimeTransformEntry`; implement the NTP core beside the existing sampler instead of in `auki-domain`.

Add:

```rust
pub const NTP_SYNC_STALE_AFTER: Duration = Duration::from_secs(3);
pub const NTP_SYNC_WINDOW: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtpSample {
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub local_send_ns: i64,
    pub remote_receive_ns: i64,
    pub remote_send_ns: i64,
    pub local_receive_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtpEstimate {
    pub remote_clock_id: String,
    pub remote_clock_hash: String,
    pub offset_ns: i64,
    pub uncertainty_ns: u32,
    pub measured_at_local_clock_ns: i64,
}

#[derive(Debug, Clone)]
pub struct NtpEstimator {
    samples: VecDeque<NtpEstimate>,
}
```

Add `NtpEstimator` with one bounded window per local peer/remote clock pairing. When selecting the current estimate, choose the lowest-uncertainty estimate in the last `NTP_SYNC_WINDOW` samples, breaking ties by newest `measured_at_local_clock_ns`. If `remote_clock_id` or `remote_clock_hash` changes, clear the window; a new clock identity means a new monotonic epoch.

Do not key the estimator by peer id alone. The correctness boundary is the remote clock identity (`remote_clock_id`, `remote_clock_hash`) because the same peer can restart with a new session clock.

- **Step 4: Convert estimates into TimeTransformEntry**

Add:

```rust
impl NtpEstimate {
    pub fn to_time_transform_entry(&self, target_offset_ns: i64) -> TimeTransformEntry {
        TimeTransformEntry {
            offset_ns: self.offset_ns.saturating_add(target_offset_ns),
            uncertainty_ns: self.uncertainty_ns,
        }
    }
}
```

For heartbeat domain sync, `target_offset_ns` is `DomainClockSource.domain_offset_ns`. The log timestamp, when a caller persists it later, is the local session-clock midpoint represented by `measured_at_local_clock_ns`.

When this estimate is persisted later, its manifest identity must be:

```text
from_clock_id   = <local SessionClock.clock_id>
from_clock_hash = <local SessionClock.clock_hash>
to_clock_id     = <cluster-name>/domain-clock
to_clock_hash   = <DomainClockSource.clock_hash>
source          = future TimeTransformSource::HeartbeatExchange { backing_peer_id }
```

The `TimeTransformEntry` payload stays only `{ offset_ns, uncertainty_ns }`; the domain-clock target belongs in the manifest/snapshot identity, not inside the heartbeat carrier.

- **Step 5: Run green tests**

Run: `cargo test -p auki-time ntp`

Expected: PASS.

- **Step 6: Commit**

```bash
git add crates/auki-time/src/lib.rs crates/auki-time/src/ntp.rs
git commit -m "feat: add ntp transform estimator"
```

---

### Task 4: Expose Domain Clock Source From ClusterManager

**Files:**

- Modify: `crates/auki-domain/src/cluster_manager.rs`
- Modify: `crates/auki-domain/src/lib.rs`
- **Step 1: Write failing ClusterManager unit tests**

Add tests near existing heartbeat/election tests. These tests pin only domain-clock authority, not NTP estimation:

```rust
#[test]
fn domain_clock_id_is_cluster_scoped() {
    let manager = fixed_peer_id(1);
    let clock = DomainClockSource::new(
        "hagall",
        manager,
        "manager/session-123/monotonic",
        "managerhash",
        0,
    );

    assert_eq!(clock.cluster_name, "hagall");
    assert_eq!(clock.clock_id, "hagall/domain-clock");
    assert_eq!(clock.registry_entry.body.scope(), Scope::DomainLocal);
    assert_eq!(clock.backing_peer_id, manager);
}

#[test]
fn initial_manager_domain_clock_is_backed_by_session_clock_zero_offset() {
    let manager = fixed_peer_id(1);
    let source = DomainClockSource::new(
        "hagall",
        manager,
        "manager/session-123/monotonic",
        "managerhash",
        0,
    );

    assert_eq!(source.backing_clock_id, "manager/session-123/monotonic");
    assert_eq!(source.backing_clock_hash, "managerhash");
    assert_eq!(source.domain_offset_ns, 0);
}
```

If `ClockBody::scope()` helper does not exist, assert by matching `ClockBody::MonotonicClock(meta)` and checking `meta.scope`.

- **Step 2: Run red tests**

Run: `cargo test -p auki-domain cluster_manager::tests::domain_clock_id_is_cluster_scoped cluster_manager::tests::initial_manager_domain_clock_is_backed_by_session_clock_zero_offset`

Expected: FAIL because `DomainClockSource` does not exist.

- **Step 3: Add DomainClockSource**

Add:

```rust
pub const DOMAIN_CLOCK_NAME: &str = "domain-clock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainClockSource {
    pub cluster_name: String,
    pub clock_id: String,
    pub clock_hash: String,
    pub registry_entry: ClockRegistryEntry,
    pub backing_peer_id: PeerId,
    pub backing_clock_id: String,
    pub backing_clock_hash: String,
    pub domain_offset_ns: i64,
}
```

`DomainClockSource::new(cluster_name, backing_peer_id, backing_clock_id, backing_clock_hash, domain_offset_ns)` stores `cluster_name` explicitly. Do not force callers to recover it by parsing `clock_id`.

It builds:

```rust
ClockRegistryEntry {
    clock_id: format!("{cluster_name}/{DOMAIN_CLOCK_NAME}"),
    body: ClockBody::MonotonicClock(ClockMeta {
        unit: "ns".into(),
        monotonic: true,
        epoch: None,
        scope: Scope::DomainLocal,
    }),
}
```

For the initial Manager, set `backing_peer_id` to the Manager peer id, set `backing_clock_id` / `backing_clock_hash` to the Manager's SDK-owned `SessionClock` id/hash, and set `domain_offset_ns = 0`. A promoted Manager sets `backing_peer_id` to itself and `domain_offset_ns` from its own locally produced `SessionClock -> domain-clock` transform before promotion. `ClusterManager` records that offset as domain-clock source continuity; it does not compute follower transforms.

The `registry_entry` must be identical across peers for a given domain-clock version. Its hash is used as the TimeTransform Log target hash; it is not carried in heartbeat frames.

- **Step 4: Add ClusterManager field and accessor**

Add:

```rust
domain_clock_source: Arc<Mutex<DomainClockSource>>,
```

Add:

```rust
pub fn domain_clock_source(&self) -> DomainClockSource {
    self.domain_clock_source
        .lock()
        .expect("domain_clock_source lock")
        .clone()
}
```

Do not add NTP estimator state or local transform accessors to `ClusterManager`. Those belong to `auki-time` or a higher-level SDK composition that owns the local peer's time-transform producer.

- **Step 5: Pass the daemon's heartbeat timestamp source into NetworkRuntime**

In create/join/bootstrap construction, define the heartbeat timestamp source from `SessionClock`, and bind its timestamp values to the same Clock Registry identity used by `ParticipantInfo`:

```rust
let heartbeat_timestamps = HeartbeatTimestampSource {
    clock_id: session_clock.clock_id().to_string(),
    clock_hash: session_clock.clock_hash(),
    now_ns: {
        let session_clock = session_clock.clone();
        Arc::new(move || session_clock.now_i64_ns())
    },
};
let runtime = NetworkRuntime::spawn(
    swarm,
    allowed_peers,
    stream_provider,
    heartbeat_timestamps,
)?;
```

`NetworkRuntime` may emit timing samples on a separate receiver or event stream, but `ClusterManager` must not filter or retain offset windows. Each peer's local `auki-time` NTP estimator consumes samples for the current backing peer and clock identity, then produces the peer's own transform.

- **Step 6: Run green tests**

Run:

```bash
cargo test -p auki-domain cluster_manager::tests -- --nocapture
```

Expected: PASS.

- **Step 7: Commit**

```bash
git add crates/auki-domain/src/cluster_manager.rs crates/auki-domain/src/lib.rs
git commit -m "feat: expose domain clock source"
```

---

### Task 5: Expose Python Domain Clock Source

**Files:**

- Modify: `bindings/python/auki-domain-py/src/lib.rs`
- Modify: `bindings/python/auki-domain-py/python_tests/test_surface.py`
- Modify: `bindings/python/auki-domain-py/python_tests/test_cluster_manager.py`
- **Step 1: Write failing Python surface tests**

Add a test for the value surface without requiring a live cluster:

```python
def test_domain_clock_source_value_type():
    clock = auki_domain.DomainClockSource(
        cluster_name="hagall",
        clock_id="hagall/domain-clock",
        clock_hash="abc123",
        backing_peer_id="12D3KooWExample",
        backing_clock_id="manager/session-123/monotonic",
        backing_clock_hash="managerhash",
        domain_offset_ns=0,
        registry_entry_json='{"clock_id":"hagall/domain-clock","type":"monotonic_clock","unit":"ns","monotonic":true,"epoch":null,"scope":"domain-local"}',
    )

    assert clock.cluster_name == "hagall"
    assert clock.clock_id == "hagall/domain-clock"
    assert clock.backing_peer_id == "12D3KooWExample"
    assert clock.domain_offset_ns == 0
```

Add manager method expectations to the existing ClusterManager tests:

```python
clock = manager.domain_clock_source()
assert clock.cluster_name == manager.cluster_name
assert clock.clock_id.endswith("/domain-clock")
assert clock.backing_clock_id.endswith("/monotonic")
```

- **Step 2: Run red tests**

Run: `pytest bindings/python/auki-domain-py/python_tests/ -q`

Expected: FAIL because the Python value type and method do not exist.

- **Step 3: Implement PyO3 value types**

Add pyclass:

```rust
#[pyclass(name = "DomainClockSource")]
pub struct PyDomainClockSource {
    #[pyo3(get)]
    pub cluster_name: String,
    #[pyo3(get)]
    pub clock_id: String,
    #[pyo3(get)]
    pub clock_hash: String,
    #[pyo3(get)]
    pub backing_peer_id: String,
    #[pyo3(get)]
    pub backing_clock_id: String,
    #[pyo3(get)]
    pub backing_clock_hash: String,
    #[pyo3(get)]
    pub domain_offset_ns: i64,
    #[pyo3(get)]
    pub registry_entry_json: String,
}
```

Expose `ClusterManager.domain_clock_source()`. Do not expose local NTP estimates or local `TimeTransformEntry` production from `auki-domain-py`; those belong to `auki-time`.

- **Step 4: Run green tests**

Run:

```bash
cargo test -p auki-domain-py
pytest bindings/python/auki-domain-py/python_tests/ -q
```

Expected: PASS.

- **Step 5: Commit**

```bash
git add bindings/python/auki-domain-py/src/lib.rs bindings/python/auki-domain-py/python_tests/test_surface.py bindings/python/auki-domain-py/python_tests/test_cluster_manager.py
git commit -m "feat: expose domain clock source to python"
```

---

### Task 6: Document And Verify

**Files:**

- Modify: `crates/auki-network/README.md`
- Modify: `crates/auki-network/src/README.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/auki-domain/README.md`
- Modify: `crates/auki-domain/src/README.md`
- Modify: `crates/auki-domain/src/sprint.md`
- Modify: `crates/auki-domain/changelog.md`
- Modify: `bindings/python/auki-domain-py/README.md`
- Modify: `bindings/python/auki-domain-py/src/README.md`
- Modify: `bindings/python/auki-domain-py/changelog.md`
- Modify: `bindings/python/changelog.md`
- Modify: `bindings/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`
- **Step 1: Update network docs**

Document `/auki/heartbeat/0.0.1` as carrying NTP-style echo fields and the sender timestamp clock id/hash. Say explicitly that heartbeat frames do not carry the domain clock id/hash, and that `auki-network` emits raw `HeartbeatTimingSample` values without defining filtering policy or TimeTransform interpretation.

- **Step 2: Update domain docs**

Document:

- Cluster name: `DomainClockSource.cluster_name`, sourced from `ClusterManager::cluster_name()`.
- Domain clock id: `<cluster-name>/domain-clock`.
- Domain clock registry entry: monotonic, ns, `scope = "domain-local"`, `epoch = null`.
- Backing peer/source: the current `backing_peer_id` owns `backing_clock_id`; domain time is `backing_clock + domain_offset_ns`.
- Initial Manager offset: `0`.
- Promoted Manager offset: the promoted peer's last known `local session clock -> domain clock` offset.
- Heartbeat frame `clock_id` names the sender's measured clock; heartbeat frames do not name the logical domain target.
- `DomainClockSource.registry_entry` is identical across peers for a given domain-clock version, so `domain_clock_hash` should match across the cluster.
- `DomainClockSource` and future TimeTransform Log manifests are where the domain-clock target is visible.
- Monotonic clock ids/hashes must identify a single epoch/lifetime; restarting a clock at zero requires a new clock identity or hash.
- Each peer's local `auki-time` NTP estimator produces its own `local session clock -> domain clock` transform.
- Manager transform: `manager session clock + domain_offset_ns`.
- Handoff behavior: the domain clock id stays stable and the promoted Manager carries forward the last known offset; each peer's estimator repopulates from new Manager samples.
- **Step 3: Update Python docs**

Add a small example:

```python
clock = manager.domain_clock_source()
print(clock.cluster_name)
print(clock.clock_id)
print(clock.backing_clock_id)
```

- **Step 4: Update changelogs**

Add detailed leaf entries to `auki-network`, `auki-domain`, and `bindings/python/auki-domain-py`; add one-line summaries to `crates/changelog.md`, `bindings/python/changelog.md`, `bindings/changelog.md`, and root `changelog.md`.

- **Step 5: Run full verification**

Run:

```bash
cargo test -p auki-network --features swarm
cargo test -p auki-domain
cargo test -p auki-domain-py
pytest bindings/python/auki-domain-py/python_tests/ -q
git diff --check
```

Expected: all pass.

- **Step 6: Commit**

```bash
git add crates/auki-network/README.md crates/auki-network/src/README.md crates/auki-network/changelog.md
git add crates/auki-domain/README.md crates/auki-domain/src/README.md crates/auki-domain/src/sprint.md crates/auki-domain/changelog.md
git add bindings/python/auki-domain-py/README.md bindings/python/auki-domain-py/src/README.md bindings/python/auki-domain-py/changelog.md bindings/python/changelog.md bindings/changelog.md
git add crates/changelog.md changelog.md
git commit -m "docs: document domain clock heartbeat sync"
```

---

## Self-Review

- Spec coverage: The plan covers heartbeat wire changes, raw timing extraction, `auki-time` NTP estimation, domain-clock source identity, per-peer live TimeTransform conversion, Manager handoff continuity, Python exposure, docs, and changelog propagation.
- Placeholder scan: No `TBD`, `TODO`, or "add tests later" placeholders remain.
- Type consistency: `HeartbeatTimingSample` uses local/remote timestamp names consistently; `NtpEstimate.offset_ns` means `remote_clock_ns - local_clock_ns`; exported `TimeTransformEntry.offset_ns` means `target_clock_ns - local_clock_ns` after applying the caller-provided target offset.
- Scope check: This plan intentionally does not implement persistent TimeTransform Log writing. It creates the live transform that a session/logging layer can append later once `auki-session-py`'s TimeTransform handle exists.
