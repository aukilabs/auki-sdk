//! Manager-role state machine for a Domain's Cluster Registry.
//!
//! Per Greenland T2 + T3 + T4 + T6 + T7: exactly one peer in a Domain
//! holds Manager role at a time. The Manager:
//!
//! - ticks every [`HEARTBEAT_INTERVAL`] and probes each known Member
//!   with a `HeartbeatRequest` over `/auki/heartbeat/0.0.1` (T2 + T3)
//! - tracks per-Member `consecutive_missed_ticks`; declares a Member
//!   departed after [`MISSED_TICKS_FOR_DEPARTURE`] consecutive missed
//!   responses (T4)
//! - holds the authoritative in-memory Cluster Registry — the local
//!   ground-truth of which peers are present (T6) — represented as a
//!   live [`auki_network::cluster_doc::ClusterDoc`]
//! - on every mutation (join, depart, fact change) bumps the
//!   `mutation_ns` timestamp and broadcasts the fresh `ClusterDoc`
//!   wrapped in a [`registry_protocol::SnapshotEnvelope`] over
//!   `/auki/registry/0.0.1` (T7)
//!
//! The state machine is **transport-agnostic** by design. [`Manager`]
//! exposes pure logic methods — `tick`, `record_response`, `add_member`,
//! `remove_member` — and emits [`ManagerEffect`]s describing what the
//! caller must do on the wire. A real deployment wires a swarm-backed
//! transport that turns each `SendHeartbeat` effect into a libp2p
//! request and each `BroadcastSnapshot` effect into a substream open;
//! tests use [`MockEffectSink`] to assert on emitted effects directly.
//!
//! This separation is what makes the timing rules — 10 s tick cadence,
//! 2-missed-tick departure threshold, mutation-driven snapshot
//! cadence — testable without standing up a real swarm.
//!
//! # Status
//!
//! Greenland PR 2b. Manager state machine only — wiring to a real
//! swarm-backed transport lives in `auki-domain::runtime` (planned for
//! PR 2c or absorbed into PR 3 alongside failover) so the state
//! machine here can be reviewed and proven correct in isolation.

use auki_network::cluster_doc::{ClusterDoc, ClusterPeer, SUPPORTED_VERSION};
use auki_network::heartbeat_protocol::{HeartbeatRequest, HeartbeatResponse};
use auki_network::registry_protocol::SnapshotEnvelope;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use std::collections::HashMap;
use std::time::Duration;

/// Cadence at which the Manager probes each known Member. Per
/// Greenland T2 — "10 seconds" is the canonical value. Tunable at
/// construction via [`ManagerConfig`] for tests that want faster ticks.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// How many consecutive missed responses before a Member is declared
/// departed. Per Greenland T4 — "2 missed ticks ≈ 20 s wall-clock".
pub const MISSED_TICKS_FOR_DEPARTURE: u32 = 2;

// ─── Configuration ─────────────────────────────────────────────────

/// Tunables for [`Manager`]. Defaults match the Greenland T2/T4 spec
/// constants — tests override to keep run-times short.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Cadence at which [`Manager::tick`] should be called by the
    /// caller. The state machine itself doesn't drive the clock —
    /// the caller does — but this value is carried for any future
    /// telemetry / log replay that wants to know the configured rate.
    pub heartbeat_interval: Duration,
    /// Consecutive missed-tick threshold before a Member is declared
    /// departed.
    pub missed_ticks_for_departure: u32,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: HEARTBEAT_INTERVAL,
            missed_ticks_for_departure: MISSED_TICKS_FOR_DEPARTURE,
        }
    }
}

// ─── Effects ───────────────────────────────────────────────────────

/// Side-effect emitted by [`Manager`] that the caller must dispatch
/// onto the wire. The Manager itself never touches a swarm — it stays
/// a pure state machine. The transport layer (see crate docs) consumes
/// effects in order and translates each into the appropriate libp2p
/// operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerEffect {
    /// Send a `HeartbeatRequest` to `peer` over `/auki/heartbeat/0.0.1`.
    /// The transport should call back into [`Manager::record_response`]
    /// (or [`Manager::record_failure`]) once the request resolves.
    SendHeartbeat {
        /// Target Member.
        peer: PeerId,
        /// Request payload — carries the Manager's tick timestamp and
        /// its own `PeerId` per Greenland T3, so responders can spot
        /// stale-Manager requests post-failover.
        request: HeartbeatRequest,
    },
    /// Broadcast `envelope` to every known Member over
    /// `/auki/registry/0.0.1`. Fire-and-forget per Greenland T7 —
    /// snapshots are mutation-driven and idempotent by `mutation_ns`,
    /// so a dropped substream is just resent with the next mutation.
    /// The transport iterates `envelope.doc.peers` at dispatch time.
    BroadcastSnapshot {
        /// Snapshot payload to send. The transport frames it onto the
        /// wire via [`registry_protocol::write_envelope`].
        envelope: SnapshotEnvelope,
    },
    /// Inform observers (UI, logging, downstream `auki-domain` API)
    /// that a Member just departed. This is a notification, not a
    /// state mutation — by the time the effect is emitted, the
    /// Manager has already removed the peer from its registry and a
    /// corresponding [`Self::BroadcastSnapshot`] effect has been
    /// emitted carrying the post-departure state.
    MemberDeparted {
        /// Peer that was just removed from the local registry.
        peer: PeerId,
        /// Number of consecutive missed ticks that triggered the
        /// departure — always `>= config.missed_ticks_for_departure`.
        missed_ticks: u32,
    },
}

// ─── Member tracking ──────────────────────────────────────────────

/// What the Manager knows about each Member it's tracking. Stored
/// keyed by `PeerId` in [`Manager::members`]. Internal to this module
/// — exposed indirectly via [`Manager::member_count`] and the snapshot
/// envelope the Manager broadcasts on every mutation.
#[derive(Debug, Clone)]
struct MemberEntry {
    /// Mirrored fields used to (re-)build the outgoing
    /// [`ClusterPeer`] on every snapshot. Kept structurally rather
    /// than as a pre-built `ClusterPeer` so future fact-mutation
    /// methods (`update_addresses`, etc. — planned for PR 3) can
    /// patch parts without reconstructing.
    addresses: Vec<Multiaddr>,
    /// Mirror of [`ClusterPeer::expected_app_id`].
    expected_app_id: Option<String>,
    /// Mirror of [`ClusterPeer::note`].
    note: Option<String>,
    /// Consecutive ticks since the last successful heartbeat response.
    /// Reset to 0 on every [`Manager::record_response`]; incremented
    /// on every [`Manager::tick`] before dispatch. When this reaches
    /// `config.missed_ticks_for_departure`, the Member is removed.
    consecutive_missed_ticks: u32,
}

impl MemberEntry {
    fn to_cluster_peer(&self, peer_id: PeerId) -> ClusterPeer {
        ClusterPeer {
            peer_id,
            addresses: self.addresses.clone(),
            expected_app_id: self.expected_app_id.clone(),
            note: self.note.clone(),
        }
    }
}

// ─── Manager ───────────────────────────────────────────────────────

/// In-memory authoritative Cluster Registry for a single Domain.
///
/// One `Manager` per Domain, on one peer. Holding two `Manager`s for
/// the same Domain across two peers is a split-brain that PR 3's
/// failover machinery will prevent. This type is intentionally
/// `!Send`-friendly (no internal locks) — embed it in a single-task
/// driver and call its methods serially.
///
/// # Lifecycle
///
/// 1. Construct via [`Manager::new`] with the Manager's own `PeerId`,
///    its own [`ClusterPeer`] fields (own addresses / app id / note),
///    a `cluster_name`, and a [`ManagerConfig`].
/// 2. On each Member admission (T5 / PR 4), call [`Manager::add_member`].
///    Emits a `BroadcastSnapshot` effect carrying the post-join state.
/// 3. Every `config.heartbeat_interval`, the driver calls
///    [`Manager::tick`]. Each tick emits one `SendHeartbeat` effect
///    per still-alive Member, and (if any Member crosses the
///    departure threshold this tick) one `MemberDeparted` per
///    departing peer plus one coalesced `BroadcastSnapshot`.
/// 4. On every heartbeat response, the transport calls
///    [`Manager::record_response`]; on transport failure (request
///    timeout, connection error), [`Manager::record_failure`].
pub struct Manager {
    /// Manager's own `PeerId`. Carried into every emitted
    /// `HeartbeatRequest::manager_peer_id` so Members can detect
    /// stale-Manager requests post-failover.
    self_peer: PeerId,
    /// Manager's own dialable addresses, mirrored into the
    /// `ClusterPeer` entry the Manager publishes for itself in every
    /// snapshot.
    self_addresses: Vec<Multiaddr>,
    /// Manager's own `expected_app_id`.
    self_expected_app_id: Option<String>,
    /// Manager's own `note`.
    self_note: Option<String>,
    /// The Domain's canonical name — for user-named Domains this is
    /// `{wallet_id}/{name}`, for the singleton this is `"Vinland"`.
    /// Mirrored into every snapshot's `ClusterDoc::cluster_name`.
    cluster_name: String,
    /// Tracked Members, keyed by `PeerId`. The Manager itself is NOT
    /// in this map; it appears in outgoing snapshots through the
    /// `self_*` fields above.
    members: HashMap<PeerId, MemberEntry>,
    /// Monotonic timestamp (ns) of the most recent mutation. Every
    /// emitted snapshot carries this as `mutation_ns` so receivers
    /// can discard out-of-order arrivals (substream A delivers slower
    /// than substream B → A's payload is stale, drop it).
    last_mutation_ns: u64,
    /// Behaviour tunables.
    config: ManagerConfig,
}

impl Manager {
    /// Construct a fresh Manager with no Members tracked.
    ///
    /// `cluster_name` MUST be the canonical Domain identity string
    /// (`{wallet_id}/{name}` for user-named, `"Vinland"` for the
    /// singleton). Mismatched names produce snapshots that receivers
    /// will reject as cross-cluster updates per
    /// [`ClusterRuntime::update_cluster_doc`]'s `ClusterNameMismatch`
    /// invariant.
    pub fn new(
        self_peer: PeerId,
        self_addresses: Vec<Multiaddr>,
        self_expected_app_id: Option<String>,
        self_note: Option<String>,
        cluster_name: impl Into<String>,
        config: ManagerConfig,
    ) -> Self {
        Self {
            self_peer,
            self_addresses,
            self_expected_app_id,
            self_note,
            cluster_name: cluster_name.into(),
            members: HashMap::new(),
            last_mutation_ns: 0,
            config,
        }
    }

    /// Number of Members currently tracked. Does NOT include the
    /// Manager itself.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Iterator over the `PeerId`s of currently-tracked Members.
    /// Order is unspecified — backed by a `HashMap`.
    pub fn members(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.members.keys().copied()
    }

    /// Most recent mutation timestamp. Bumped by `add_member`,
    /// `remove_member`, and `tick` when departures occur. Visible
    /// for tests / observability; production callers use the
    /// snapshot envelopes.
    pub fn last_mutation_ns(&self) -> u64 {
        self.last_mutation_ns
    }

    /// Register a new Member with its facts. Idempotent — calling
    /// twice with the same `peer` overwrites the facts and resets
    /// the missed-tick counter (treated as a re-join), bumps the
    /// mutation timestamp, and emits a fresh snapshot.
    ///
    /// `now_ns` is used both as the new `last_mutation_ns` and as
    /// the emitted envelope's `mutation_ns`. The caller is
    /// responsible for monotonicity — clock source should be
    /// session-monotonic (e.g. `Instant::elapsed`-based) rather
    /// than wall-clock.
    ///
    /// Emits [`ManagerEffect::BroadcastSnapshot`] carrying the
    /// post-add state.
    pub fn add_member(
        &mut self,
        peer: PeerId,
        addresses: Vec<Multiaddr>,
        expected_app_id: Option<String>,
        note: Option<String>,
        now_ns: u64,
        effects: &mut impl EffectSink,
    ) {
        self.members.insert(
            peer,
            MemberEntry {
                addresses,
                expected_app_id,
                note,
                consecutive_missed_ticks: 0,
            },
        );
        self.last_mutation_ns = now_ns;
        effects.push(ManagerEffect::BroadcastSnapshot {
            envelope: self.build_envelope(now_ns),
        });
    }

    /// Remove a Member explicitly (e.g. operator-initiated eviction
    /// in some future API; not currently part of the Greenland brief
    /// but exercised by failover tests in PR 3). Idempotent — no
    /// effect if `peer` isn't tracked. When the peer was present,
    /// bumps the mutation timestamp and emits a snapshot.
    pub fn remove_member(&mut self, peer: PeerId, now_ns: u64, effects: &mut impl EffectSink) {
        if self.members.remove(&peer).is_some() {
            self.last_mutation_ns = now_ns;
            effects.push(ManagerEffect::BroadcastSnapshot {
                envelope: self.build_envelope(now_ns),
            });
        }
    }

    /// Driven by the caller's timer at `config.heartbeat_interval`
    /// cadence. For each tracked Member:
    ///
    /// 1. Increment its `consecutive_missed_ticks`.
    /// 2. If the new value reaches `config.missed_ticks_for_departure`,
    ///    remove the Member and emit `MemberDeparted`.
    /// 3. Otherwise emit a `SendHeartbeat` effect for that Member.
    ///
    /// If any Member departed this tick, ONE coalesced
    /// `BroadcastSnapshot` is emitted after all departures — never
    /// one snapshot per departure — so receivers see a single
    /// post-tick state transition rather than N intermediate ones.
    ///
    /// `now_ns` is used to stamp `HeartbeatRequest::tick_ns` on
    /// every emitted probe AND to bump `last_mutation_ns` if any
    /// departure occurs.
    pub fn tick(&mut self, now_ns: u64, effects: &mut impl EffectSink) {
        let threshold = self.config.missed_ticks_for_departure;

        // Phase 1: increment miss counters, partition into
        // (still-alive, departing). We materialize the lists up
        // front because emitting effects with `&mut self` and
        // iterating `self.members` are mutually exclusive borrows.
        let mut alive: Vec<PeerId> = Vec::with_capacity(self.members.len());
        let mut departing: Vec<(PeerId, u32)> = Vec::new();

        for (peer, entry) in self.members.iter_mut() {
            entry.consecutive_missed_ticks = entry.consecutive_missed_ticks.saturating_add(1);
            if entry.consecutive_missed_ticks >= threshold {
                departing.push((*peer, entry.consecutive_missed_ticks));
            } else {
                alive.push(*peer);
            }
        }

        // Phase 2: process departures first so the broadcast carries
        // the post-departure state. Each departure emits a
        // notification; a single coalesced snapshot is emitted after
        // all departures for this tick are folded in.
        let any_departure = !departing.is_empty();
        // Sort for deterministic emit order — HashMap iteration is
        // unstable across runs and tests pin on order. Negligible
        // cost at the v1 ≤10-peer scale.
        departing.sort_by_key(|(p, _)| *p);
        for (peer, missed_ticks) in departing {
            self.members.remove(&peer);
            effects.push(ManagerEffect::MemberDeparted { peer, missed_ticks });
        }
        if any_departure {
            self.last_mutation_ns = now_ns;
            effects.push(ManagerEffect::BroadcastSnapshot {
                envelope: self.build_envelope(now_ns),
            });
        }

        // Phase 3: probe still-alive Members. Same sort rationale.
        alive.sort();
        for peer in alive {
            effects.push(ManagerEffect::SendHeartbeat {
                peer,
                request: HeartbeatRequest {
                    tick_ns: now_ns,
                    manager_peer_id: self.self_peer,
                },
            });
        }
    }

    /// Record that `peer` returned a heartbeat response. Resets its
    /// `consecutive_missed_ticks` counter.
    ///
    /// No-op if `peer` isn't currently tracked (responses can arrive
    /// after a departure has been finalized — race between the
    /// response arriving and the tick that declared the peer dead;
    /// the response loses and we discard it).
    pub fn record_response(&mut self, peer: PeerId, _response: HeartbeatResponse) {
        if let Some(entry) = self.members.get_mut(&peer) {
            entry.consecutive_missed_ticks = 0;
        }
    }

    /// Record that a heartbeat request to `peer` failed at the
    /// transport layer (timeout, dial error). Currently a no-op —
    /// the `consecutive_missed_ticks` counter was already bumped at
    /// `tick` time and the failure is implicit in the absence of a
    /// matching [`Self::record_response`] call before the next tick.
    /// Exposed as a separate method so the transport has somewhere
    /// to plumb `OutboundFailure` events; future versions may use
    /// the signal for faster-than-2-ticks eviction on hard failures.
    pub fn record_failure(&mut self, _peer: PeerId) {
        // Intentional no-op at v1. See doc comment.
    }

    // ─── Internal helpers ──────────────────────────────────────

    /// Build the snapshot envelope reflecting current Manager state.
    /// The envelope's `doc.peers` includes the Manager itself plus
    /// every currently-tracked Member, in `PeerId`-sorted order for
    /// deterministic serialization.
    fn build_envelope(&self, mutation_ns: u64) -> SnapshotEnvelope {
        // Build sorted peer list: Manager first by virtue of its
        // PeerId sort position, plus every Member. Sorting ensures
        // the JSON wire form is stable for the same logical state.
        let mut peers: Vec<ClusterPeer> = Vec::with_capacity(self.members.len() + 1);
        peers.push(ClusterPeer {
            peer_id: self.self_peer,
            addresses: self.self_addresses.clone(),
            expected_app_id: self.self_expected_app_id.clone(),
            note: self.self_note.clone(),
        });
        for (peer_id, entry) in &self.members {
            peers.push(entry.to_cluster_peer(*peer_id));
        }
        peers.sort_by_key(|p| p.peer_id);

        SnapshotEnvelope {
            mutation_ns,
            doc: ClusterDoc {
                version: SUPPORTED_VERSION,
                cluster_name: self.cluster_name.clone(),
                peers,
            },
        }
    }
}

// ─── Effect collection ────────────────────────────────────────────

/// Where [`Manager`] writes its emitted effects. A real deployment
/// uses a small struct that immediately dispatches each effect onto
/// the wire (and to subscribers via a watch channel); tests use
/// [`MockEffectSink`] to collect them for assertions.
///
/// Implemented for `Vec<ManagerEffect>` for trivial collection.
pub trait EffectSink {
    /// Receive one emitted effect. Implementations MUST NOT block
    /// for long — the Manager is on a single-task driver and a slow
    /// sink directly delays the next tick. Production sinks queue
    /// onto an mpsc channel and return immediately.
    fn push(&mut self, effect: ManagerEffect);
}

impl EffectSink for Vec<ManagerEffect> {
    fn push(&mut self, effect: ManagerEffect) {
        Vec::push(self, effect);
    }
}

/// In-memory effect collector for tests. Identical to
/// `Vec<ManagerEffect>` semantically; the named type is convenient
/// for assertion macros.
#[derive(Debug, Default)]
pub struct MockEffectSink {
    /// Emitted effects in the order [`Manager`] pushed them.
    pub effects: Vec<ManagerEffect>,
}

impl MockEffectSink {
    /// Construct an empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain the collected effects, returning them in emit order
    /// and resetting the sink. Useful for asserting per-tick output
    /// without carrying earlier effects into later assertions.
    pub fn drain(&mut self) -> Vec<ManagerEffect> {
        std::mem::take(&mut self.effects)
    }
}

impl EffectSink for MockEffectSink {
    fn push(&mut self, effect: ManagerEffect) {
        self.effects.push(effect);
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p_identity::Keypair;

    fn peer_id(seed: u8) -> PeerId {
        // Stable PeerId from a seed byte — generate a fresh ed25519
        // keypair from `[seed; 32]` and take its PeerId. Used to
        // make test assertions deterministic without standing up
        // real wallets.
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        let secret = libp2p_identity::ed25519::SecretKey::try_from_bytes(bytes)
            .expect("32 bytes is a valid ed25519 secret");
        let kp: libp2p_identity::ed25519::Keypair = secret.into();
        Keypair::from(kp).public().to_peer_id()
    }

    fn mgr() -> Manager {
        Manager::new(
            peer_id(1),
            Vec::new(),
            Some("manager".to_string()),
            None,
            "test-cluster",
            fast_config(),
        )
    }

    fn fast_config() -> ManagerConfig {
        ManagerConfig {
            heartbeat_interval: Duration::from_millis(10),
            missed_ticks_for_departure: 2,
        }
    }

    #[test]
    fn new_manager_has_no_members_and_zero_mutation_ns() {
        let m = mgr();
        assert_eq!(m.member_count(), 0);
        assert_eq!(m.last_mutation_ns(), 0);
    }

    #[test]
    fn add_member_bumps_mutation_ns_and_broadcasts_snapshot() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();

        m.add_member(
            peer_id(2),
            Vec::new(),
            Some("member-a".to_string()),
            None,
            1_000_000,
            &mut sink,
        );

        assert_eq!(m.member_count(), 1);
        assert_eq!(m.last_mutation_ns(), 1_000_000);
        assert_eq!(sink.effects.len(), 1);
        match &sink.effects[0] {
            ManagerEffect::BroadcastSnapshot { envelope } => {
                assert_eq!(envelope.mutation_ns, 1_000_000);
                assert_eq!(envelope.doc.cluster_name, "test-cluster");
                assert_eq!(envelope.doc.version, SUPPORTED_VERSION);
                // Snapshot includes both Manager and Member.
                assert_eq!(envelope.doc.peers.len(), 2);
                let ids: Vec<PeerId> = envelope.doc.peers.iter().map(|p| p.peer_id).collect();
                assert!(ids.contains(&peer_id(1)));
                assert!(ids.contains(&peer_id(2)));
            }
            other => panic!("expected BroadcastSnapshot, got {:?}", other),
        }
    }

    #[test]
    fn add_member_is_idempotent_per_peer_id() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(
            peer_id(2),
            Vec::new(),
            None,
            Some("v1".to_string()),
            100,
            &mut sink,
        );
        m.add_member(
            peer_id(2),
            Vec::new(),
            None,
            Some("v2".to_string()),
            200,
            &mut sink,
        );
        // Still one Member tracked.
        assert_eq!(m.member_count(), 1);
        assert_eq!(m.last_mutation_ns(), 200);

        // Latest snapshot should have the v2 note.
        if let ManagerEffect::BroadcastSnapshot { envelope } = &sink.effects[1] {
            let entry = envelope
                .doc
                .peers
                .iter()
                .find(|p| p.peer_id == peer_id(2))
                .expect("member-a in snapshot");
            assert_eq!(entry.note.as_deref(), Some("v2"));
        } else {
            panic!("expected BroadcastSnapshot");
        }
    }

    #[test]
    fn remove_unknown_member_is_noop() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.remove_member(peer_id(99), 500, &mut sink);
        assert_eq!(m.member_count(), 0);
        assert_eq!(m.last_mutation_ns(), 0);
        assert!(sink.effects.is_empty());
    }

    #[test]
    fn remove_known_member_bumps_mutation_ns_and_broadcasts() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        sink.drain();
        m.remove_member(peer_id(2), 200, &mut sink);
        assert_eq!(m.member_count(), 0);
        assert_eq!(m.last_mutation_ns(), 200);
        assert_eq!(sink.effects.len(), 1);
        if let ManagerEffect::BroadcastSnapshot { envelope } = &sink.effects[0] {
            assert_eq!(envelope.mutation_ns, 200);
            assert_eq!(envelope.doc.peers.len(), 1); // just the Manager
            assert_eq!(envelope.doc.peers[0].peer_id, peer_id(1));
        } else {
            panic!("expected BroadcastSnapshot");
        }
    }

    #[test]
    fn tick_emits_one_heartbeat_per_member_with_correct_request() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(3), Vec::new(), None, None, 200, &mut sink);
        sink.drain();

        m.tick(300, &mut sink);

        // No departures yet — both Members get probed.
        let effects = sink.drain();
        let heartbeats: Vec<&ManagerEffect> = effects
            .iter()
            .filter(|e| matches!(e, ManagerEffect::SendHeartbeat { .. }))
            .collect();
        assert_eq!(heartbeats.len(), 2);

        // Each request carries the Manager's PeerId and the tick_ns.
        for e in &heartbeats {
            if let ManagerEffect::SendHeartbeat { request, .. } = e {
                assert_eq!(request.tick_ns, 300);
                assert_eq!(request.manager_peer_id, peer_id(1));
            }
        }
    }

    #[test]
    fn tick_with_no_members_emits_nothing() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.tick(100, &mut sink);
        assert!(sink.effects.is_empty());
        assert_eq!(m.last_mutation_ns(), 0);
    }

    #[test]
    fn tick_skips_heartbeat_for_member_at_departure_threshold() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        sink.drain();

        // Tick 1: probe sent, miss counter -> 1.
        m.tick(200, &mut sink);
        assert!(
            sink.drain()
                .iter()
                .any(|e| matches!(e, ManagerEffect::SendHeartbeat { .. }))
        );

        // Tick 2: miss counter -> 2 (== threshold) → departure.
        // No heartbeat emitted for the departed peer.
        m.tick(300, &mut sink);
        let effects = sink.drain();

        let departures: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                ManagerEffect::MemberDeparted { peer, missed_ticks } => {
                    Some((*peer, *missed_ticks))
                }
                _ => None,
            })
            .collect();
        assert_eq!(departures, vec![(peer_id(2), 2)]);

        let heartbeats: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, ManagerEffect::SendHeartbeat { .. }))
            .collect();
        assert!(heartbeats.is_empty(), "no probe for departed peer");

        let broadcasts: Vec<_> = effects
            .iter()
            .filter(|e| matches!(e, ManagerEffect::BroadcastSnapshot { .. }))
            .collect();
        assert_eq!(broadcasts.len(), 1, "one snapshot after departure batch");
        assert_eq!(m.member_count(), 0);
        assert_eq!(m.last_mutation_ns(), 300);
    }

    #[test]
    fn record_response_resets_miss_counter() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        sink.drain();

        // Tick → miss = 1
        m.tick(200, &mut sink);
        sink.drain();

        // Response arrives → miss = 0
        m.record_response(
            peer_id(2),
            HeartbeatResponse {
                responder_peer_id: peer_id(2),
            },
        );

        // Tick again → miss = 1 (not 2), still alive
        m.tick(300, &mut sink);
        let effects = sink.drain();
        assert!(effects.iter().any(|e| matches!(
            e,
            ManagerEffect::SendHeartbeat { peer, .. } if *peer == peer_id(2)
        )));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, ManagerEffect::MemberDeparted { .. }))
        );
        assert_eq!(m.member_count(), 1);
    }

    #[test]
    fn record_response_for_unknown_peer_is_noop() {
        let mut m = mgr();
        m.record_response(
            peer_id(99),
            HeartbeatResponse {
                responder_peer_id: peer_id(99),
            },
        );
        // Doesn't panic; nothing to assert on a no-op besides absence
        // of side-effects.
        assert_eq!(m.member_count(), 0);
    }

    #[test]
    fn coalesced_departure_emits_single_snapshot() {
        // Three Members, all silent. On tick 2 all three cross the
        // threshold. The Manager must emit exactly ONE snapshot
        // carrying the post-departure (empty) state, not three.
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(3), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(4), Vec::new(), None, None, 100, &mut sink);
        sink.drain();

        m.tick(200, &mut sink); // miss = 1 for all
        sink.drain();
        m.tick(300, &mut sink); // miss = 2 → all depart
        let effects = sink.drain();

        let departures = effects
            .iter()
            .filter(|e| matches!(e, ManagerEffect::MemberDeparted { .. }))
            .count();
        let broadcasts = effects
            .iter()
            .filter(|e| matches!(e, ManagerEffect::BroadcastSnapshot { .. }))
            .count();
        assert_eq!(departures, 3);
        assert_eq!(broadcasts, 1, "coalesced single snapshot per tick");
        assert_eq!(m.member_count(), 0);
    }

    #[test]
    fn mutation_ns_advances_monotonically_under_caller_supplied_clock() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        assert_eq!(m.last_mutation_ns(), 0);
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        assert_eq!(m.last_mutation_ns(), 100);
        m.add_member(peer_id(3), Vec::new(), None, None, 200, &mut sink);
        assert_eq!(m.last_mutation_ns(), 200);
        m.remove_member(peer_id(2), 300, &mut sink);
        assert_eq!(m.last_mutation_ns(), 300);
        // Tick with no departures must NOT bump mutation_ns.
        m.tick(400, &mut sink);
        assert_eq!(m.last_mutation_ns(), 300);
    }

    #[test]
    fn heartbeat_emit_order_is_deterministic() {
        // HashMap iteration order is unstable. The tick must sort
        // before emitting so test assertions and operator logs see
        // a stable order.
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(5), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(7), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(3), Vec::new(), None, None, 100, &mut sink);
        sink.drain();

        m.tick(200, &mut sink);
        let order: Vec<PeerId> = sink
            .effects
            .iter()
            .filter_map(|e| match e {
                ManagerEffect::SendHeartbeat { peer, .. } => Some(*peer),
                _ => None,
            })
            .collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted, "heartbeats emitted in sorted PeerId order");
    }

    #[test]
    fn snapshot_peers_are_sorted_by_peer_id() {
        // The Manager builds snapshot.peers in sorted order so the
        // JSON wire form is stable for the same logical state. A
        // receiver verifying a signature over the JSON would
        // otherwise see signature mismatches caused by HashMap
        // iteration order alone.
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(5), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        m.add_member(peer_id(7), Vec::new(), None, None, 100, &mut sink);
        let env = match sink.effects.last().unwrap() {
            ManagerEffect::BroadcastSnapshot { envelope } => envelope,
            _ => panic!(),
        };
        let ids: Vec<PeerId> = env.doc.peers.iter().map(|p| p.peer_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn record_failure_is_currently_noop() {
        let mut m = mgr();
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        sink.drain();

        let before = m.last_mutation_ns();
        let count = m.member_count();
        m.record_failure(peer_id(2));
        assert_eq!(m.last_mutation_ns(), before);
        assert_eq!(m.member_count(), count);
    }

    #[test]
    fn manager_self_addresses_appear_in_snapshot() {
        // The Manager is itself a peer in the Domain — its addresses
        // need to appear in the snapshot so Members can dial it.
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001"
            .parse()
            .expect("static multiaddr parses");
        let mut m = Manager::new(
            peer_id(1),
            vec![addr.clone()],
            None,
            None,
            "test-cluster",
            fast_config(),
        );
        let mut sink = MockEffectSink::new();
        m.add_member(peer_id(2), Vec::new(), None, None, 100, &mut sink);
        let env = match &sink.effects[0] {
            ManagerEffect::BroadcastSnapshot { envelope } => envelope,
            _ => panic!(),
        };
        let mgr_entry = env
            .doc
            .peers
            .iter()
            .find(|p| p.peer_id == peer_id(1))
            .expect("manager in snapshot");
        assert_eq!(mgr_entry.addresses, vec![addr]);
    }
}
