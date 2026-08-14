# The Five Questions

The Auki protocol is built around five questions any node — a phone, a robot, a cloud server, a browser tab — should be able to answer about any other node:

- **Identity** — who am I?
- **Spatial** — where did this happen?
- **Temporal** — when did this happen?
- **Networking** — how do I talk to you?
- **Tokenomics** — how do I compensate you?

Every abstraction in the SDK exists to answer one of these. Skim the table for the lay of the land, then read each section for the conceptual frame and the code that implements it.

| Question | What's implemented | What's pending |
|----------|-------------------|----------------|
| **Identity** | `auki-identity` (Wallet + ed25519 + libp2p PeerId), `auki-jcs` + `auki-hash` (content-addressing), Sensor / Clock / Frame / Detector Registries with explicit `peer_id` | — |
| **Spatial** | Pose Logs (`from → to` transforms over time), Frame Registry, `auki-geometry` (convention conversion) | Full `convert_pose` (composition along a transform path) |
| **Temporal** | Live in-memory `DomainClockEstimate` (heartbeat-driven), per-peer `local_clock_read` TimeTransform Log (monotonic↔UTC, on disk), `auki-time` math, Clock Registry with explicit scope | `convert_time` (a unified API over the live estimate and the recorded log) |
| **Networking** | libp2p substrate, 8 typed peer protocols, `auki-session` `Peer` / `Session` (declarative app API) + `auki-domain::Domain::join` (network presence) | `Session::materialize_remote_log` (Phase 5 of #216), Python `Domain::join` binding |
| **Tokenomics** | `Wallet` exists as the on-device primitive | All payment / billing rails |

---

## Identity — Who am I?

A peer needs a durable, cryptographically grounded answer to "who am I" — and the same answer needs to mean the same thing to every other peer. Three IDs cover the practical surface area:

| ID | Lifetime | Where it comes from |
|----|----------|---------------------|
| `peer_id` | Durable per device | `Wallet::derive_child("peer/v1")` → libp2p `PeerId` |
| `app_id` | Stable per app on this device | Caller-supplied string (e.g. `"galbot-ctrl"`) |
| `session_id` | Fresh per session start | ULID minted by `Peer::start_session()` |

Beyond those three, every long-lived **thing** the SDK references — a sensor, a clock, a coordinate frame, a detector, an individual log — also needs identity. The SDK uses **content-addressing**: every registry entry's `(peer_id, id, hash)` triple uniquely names it, where the hash is XXH3-128 over RFC 8785 JCS-canonical JSON. The hash *is* the version; refining an entry produces a new sibling row under the same id.

### What addresses it

- [`auki-identity`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-identity) — `Wallet`, deterministic child derivation, signed creation certs, `load_or_mint_seed`
- [`auki-jcs`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-jcs) — RFC 8785 JSON canonicalization
- [`auki-hash`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-hash) — XXH3-128 wrapper
- [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry) — `SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`; each carries an explicit `peer_id` field after the #216 schema migration

### How a consumer composes the answer

```rust
let peer = Peer::new("galbot-01", "galbot-ctrl")
    .with_storage_root("/data/auki/galbot-01".into());
let session = peer.start_session()?;

// peer / app / session — the three SDK IDs
session.peer_id();    // "galbot-01"
session.app_id();     // "galbot-ctrl"
session.session_id(); // 26-char ULID

// Content-addressed registry entry (registries are peer-level)
let frame = peer.register_frame("head_left_camera_optical", FrameDef::ros_optical())?;
// frame.peer_id == "galbot-01", frame.id == "head_left_camera_optical", frame.hash == "<xxh3>"
```

See also: [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) — why every data product also carries a `peer_id`.

---

## Spatial — Where did this happen?

A spatial computing protocol needs to answer "where was X at time *t*" without a central authority deciding the answer. The Auki SDK encodes spatial relationships as a graph of timestamped **pose logs**: each edge is a `(from_frame, to_frame)` transform sampled over time.

```text
T_X_session(t) = T_body_session(t) ∘ T_X_body(t)
```

A consumer walks the transform path, looks up or interpolates each edge at time *t*, and composes the chain. The Frame Registry defines conventions (handedness, axes, units) so the math is unambiguous; `auki-geometry` does the convention conversion when two peers express the same physical reality in different frame conventions.

### What addresses it

- [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry) — Frame Registry with `FrameDef` presets (`ros_body`, `ros_optical`, `opengl`, `unity`)
- [`auki-manifests`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-manifests) — `PoseLogManifest`, `PoseSource`, `PoseWriterMode` (rigid / movable)
- [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) — `register_pose_log`, `PoseLogSpec`, `PoseLogHandle`
- [`auki-geometry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-geometry) — `convert_pose_convention`, `convert_point_convention`, `convert_vector_convention`, `convert_direction_convention`

### What's pending

- **Full `convert_pose`** — the operation that composes pose-log paths and answers "where was X at time *t*" by walking the transform graph. The convention layer (`convert_pose_convention`) is in place; the path-walking composition is not.
- **`Session::resolve_static_transform`** — reading a one-sample sealed rigid pose log (today this returns `NotImplementedError`; tracked as Phase 5 of #216).

### How a consumer composes the answer (today)

```rust
let pose_log = session.register_pose_log(PoseLogSpec {
    from_frame: world,
    to_frame: base_link,
    clock,
    source: PoseSource::Manual,
    writer_mode: PoseWriterMode::Movable,
    expected_rate_hz: 30,
    head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
    segment_duration: Duration::from_secs(1),
    retention: Duration::from_secs(5),
})?;
// pose_log.resource_id() == "world->base_link"
```

For now, traversing multi-edge transform paths is a consumer-side concern. The `convert_pose` card will close that gap.

---

## Temporal — When did this happen?

Multiple peers run on different clocks: a robot's monotonic system clock, a host's UTC clock, a sensor's hardware-stamped clock. The SDK refuses to canonicalize on any one of them. Every timestamp ships with a named clock identity, and the SDK runs two parallel paths for relating clocks to each other — one **live and in-memory** for cluster-wide alignment, one **persisted on disk** for offline replay.

### Live: heartbeat-driven domain-clock convergence

The `/auki/heartbeat/0.0.1` protocol carries the four NTP timestamps (t0/t1/t2/t3) on every beat. Each peer's `ClockSyncHandle` (in `auki-time`) accumulates samples per `(local_clock, remote_clock)` pair and emits a `ClockTransformEstimate(offset_ns, uncertainty_ns)`. The Manager announces a `HeartbeatDomainClock` in *its* heartbeats — "the domain clock is at offset N ns from my backing clock." Each peer composes `(local → backing)` with `(backing → domain)` via `auki_time::estimate_domain_clock` and ends up with a live `DomainClockEstimate(local → domain)`.

`ClusterManager::domain_clock_estimate(local_clock_id)` returns that estimate for use anywhere in app code. **This is what "domain time" actually is, today.** It's live state — never persisted as a TimeTransform Log.

### Persisted: TimeTransform Logs on disk

A TimeTransform Log records sampled offsets between two clocks over time. The only current writer is the per-peer `local_clock_read` sampler in `auki-time` (1 Hz) — it pairs **local `CLOCK_MONOTONIC` ↔ `CLOCK_REALTIME`** on a single device, anchoring monotonic timestamps in UTC for offline replay. It does **not** record the cluster-wide domain clock; that lives in memory only.

### What addresses it

- [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry) — Clock Registry; `ClockBody::MonotonicClock`, `UtcClock`, etc., with explicit `scope` (`device-local`, `global`)
- [`auki-manifests`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-manifests) — `TimeTransformLogManifest`, `TimeTransformSource`
- [`auki-time`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-time) — `SessionClock`, `TimeTransform` math, `NtpExchange` / `NtpSample` / `compute_ntp_sample` / `select_best_ntp_sample`, `ClockSyncHandle` (in-memory NTP accumulator), `estimate_domain_clock` (composes the live estimate), 1 Hz `local_clock_read` sampler that writes the per-peer monotonic↔UTC TimeTransform Log
- [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain) — `ClusterManager::domain_clock_estimate(local_clock)` returns the live `DomainClockEstimate`
- [`auki-network`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-network) — heartbeat protocol carries the four NTP timestamps and the Manager's `HeartbeatDomainClock` descriptor
- [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) — `register_time_transform_log`, `TimeTransformLogSpec`

### What's pending

- **`convert_time`** — a published SDK operation that takes `(local_ts_ns, local_clock_ref, target_clock_ref) → target_ts_ns`. The primitives exist (live `DomainClockEstimate`, on-disk TimeTransform Log entries); the consume-side operation that picks between them and produces a reproducible conversion does not.

### How a consumer composes the answer (today)

```rust
let sdk_clock  = session.register_clock("session/sdk_clock", ClockBody::MonotonicClock(...))?;
let wall_clock = session.register_clock("wall_clock",        ClockBody::UtcClock(...))?;

// Per-peer monotonic ↔ UTC log on disk, written by the local_clock_read sampler.
let _tt_log = session.register_time_transform_log(TimeTransformLogSpec {
    from_clock: sdk_clock,
    to_clock:   wall_clock,
    source:     TimeTransformSource::LocalClockRead,
    ..
})?;

// Live cluster-wide alignment — composed from heartbeats, never persisted.
// (`domain` from `auki_domain::Domain::join(&peer, &session, config)`)
let cluster  = domain.cluster_manager();
let estimate = cluster.domain_clock_estimate()?;
// estimate.total_offset_ns + estimate.uncertainty_ns
// Or get domain time directly: cluster.domain_time_now()? -> i64
```

> **Why no canonical clock?** Picking UTC (or any clock) as the default would silently impose a conversion at every boundary. Keeping the conversion explicit — and (when persisted) recorded as a log of its own — keeps the lineage auditable and the SDK honest about what it's done to a timestamp.

---

## Networking — How do I talk to you?

The Auki SDK runs on libp2p — peer-to-peer, transport-agnostic, no central server. Two peers discover each other through a Discovery HTTP service, exchange identity over `/auki/info`, agree on cluster membership through `/auki/join` + `/auki/membership`, advertise their data products through `/auki/resources/0.2.0`, and stream live data over `/auki/stream/0.2.0`.

App code never touches libp2p protocol plumbing directly. Apps construct a `Peer`, start a `Session`, declare what they have, and call `auki_domain::Domain::join(&peer, &session, config)` — the SDK registers stream handlers, advertises the catalog, and serves anyone who asks.

### What addresses it

- [`auki-network`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-network) — libp2p substrate (TCP/QUIC, Noise, Yamux, Circuit Relay v2), typed `/auki/stream/0.2.0` streams, Discovery HTTP client with Manager + relay address hints
- [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain) — `Domain::join(&peer, &session, config)` is the app-facing entry; `ClusterManager` (cluster bootstrap, Manager election, membership, resource catalog, stream serving) is the engine `Domain` constructs and owns
- [`auki-domain-relay`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain-relay) — Domain Relay for browser-compatible reachability through Circuit Relay v2 (WIP)
- [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) — `Peer` + `Session` (identity, registries, log registration; network-free) and the `materialize_remote_log` stub

### The wire protocols

| Protocol | Purpose |
|----------|---------|
| `/auki/join/0.0.1` | Joiner asks Manager to admit it |
| `/auki/heartbeat/0.0.1` | Pairwise liveness for Manager-death detection |
| `/auki/membership/0.0.1` | Manager gossips membership |
| `/auki/info/0.0.1` | Peer-to-peer `ParticipantInfo` exchange |
| `/auki/resources/0.2.0` | Catalog row fetch (one row per peer-owned log) |
| `/auki/registries/0.3.0` | Hash-pinned registry Get + tip-only `device_model` List (last successful write via TIP) |
| `/auki/blobs/0.1.0` | Content-addressed binary blob transfer |
| `/auki/stream/0.2.0` | Typed live data streaming |

Plus an [HTTP control API](https://github.com/aukilabs/auki-sdk/blob/develop/docs/control-api.md) — a separate operator-facing surface for daemons that produce SDK sessions (BoosterApp, Sentinel), so any UI like [Park](https://github.com/aukilabs/park) can drive them through a uniform contract.

### What's pending

- **`Session::materialize_remote_log`** — Phase 5 of #216. The plumbing for opening a `/auki/stream/0.2.0` substream against a remote peer's log exists; the materialization layer that writes a local replica with its own retention policy does not.
- **Python `Domain::join`** — no Python binding yet (it requires a pre-built libp2p swarm, which Rust callers build themselves); Python daemons drive `auki-domain-py`'s `ClusterManager` directly.

### How a consumer composes the answer

```rust
let domain = auki_domain::Domain::join(&peer, &session, DomainConfig {
    target,
    local_identity,
    local_multiaddrs,
    discovery_url,
    swarm,
    stream_provider,
    daemon_info,
}).await?;

let catalog = domain.catalog();  // what this peer offers
// other peers see this peer's catalog over /auki/resources/0.2.0
```

---

## Tokenomics — How do I compensate you?

Peer-to-peer means no platform-level billing. Eventually a peer offering bytes (a robot's camera feed, an edge inference server's compute) needs a way to charge for it, and a peer consuming bytes needs a way to pay.

This question is **not implemented**. The on-device primitive that future payment rails will bind to is in place:

- [`auki-identity`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-identity) — `Wallet`: ed25519 keypair, deterministic child derivation, signed creation certs

Everything beyond that — payment channels, settlement, billing — is future work. The point of identifying tokenomics as one of the five questions is to make it a first-class architectural concern, not a bolted-on afterthought.

---

## Composing the answers — a Session is the integration point

The `Peer` / `Session` / `Domain` trio is where the five questions converge. A peer declares this device's identity and what it has, each session captures sensor data with the right spatial frame and temporal clock, a domain joins the network on the pair's behalf, and (eventually) payments settle through the Wallet.

```text
Peer
├── peer_id, app_id                       ← Identity
├── register_sensor / register_frame /
│   register_detector                      ← Identity (registries)
└── start_session() → Session
    ├── session_id + monotonic/UTC clocks  ← Identity / Temporal
    ├── register_clock                     ← Temporal
    └── register_*_log + HeadSpec          ← Spatial / Temporal lineage

Domain::join(&peer, &session, config)
├── catalog serving + cluster lifecycle    ← Networking
└── (wallet)                               ← Tokenomics (future)
```

Every line of app code that interacts with the SDK is interacting with one of these. The five questions aren't external commentary on the architecture — they *are* the architecture.

---

## See also

- [Quickstart](Quickstart) — boot a Session and register your first peer-owned log
- [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) — the SDK's core data invariant, which threads through Identity / Spatial / Temporal
- [`VISION.md`](https://github.com/aukilabs/auki-sdk/blob/develop/VISION.md) — aspirational spec, including the spatial reasoning model
- [Top-level README](https://github.com/aukilabs/auki-sdk/blob/develop/README.md) — full crate map and shipped status per question

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Glossary →](Glossary)
