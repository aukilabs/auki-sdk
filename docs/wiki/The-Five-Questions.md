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
| **Temporal** | Explicit TimeTransform Logs, fixed affine `auki-time` math, and Clock Registry entries with explicit scope | `convert_time` over recorded transforms and application-supplied clock relations |
| **Networking** | Authenticated Rust/Python `Domain` owner, explicit routes, and `/auki/auth/1/...` protocols | `Session::materialize_remote_log` and later native browser/Swift engines |
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

Multiple peers run on different clocks: a robot's monotonic system clock, a
host's UTC clock, or a sensor's hardware clock. The SDK does not choose a
canonical Domain clock. Every timestamp names a content-pinned Clock Registry
entry, and clock relationships are explicit data.

### Persisted TimeTransform Logs

A TimeTransform Log records sampled offsets between two clocks over time. The
current `local_clock_read` sampler in `auki-time` pairs local monotonic and
realtime clocks for replay. Applications may record additional explicit clock
relations; `auki-domain` owns no hidden heartbeat or synchronized-time state.

### What addresses it

- [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry) — content-pinned Clock Registry entries.
- [`auki-manifests`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-manifests) — `TimeTransformLogManifest` and provenance.
- [`auki-time`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-time) — clock primitives, fixed transform math, and the local sampler.
- [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) — TimeTransform Log registration.

### What's pending

- **`convert_time`** — a published operation that evaluates recorded or
  explicitly supplied clock relations without silently selecting a global
  clock.

> **Why no canonical clock?** Picking UTC (or any clock) as the default would silently impose a conversion at every boundary. Keeping the conversion explicit — and (when persisted) recorded as a log of its own — keeps the lineage auditable and the SDK honest about what it's done to a timestamp.

---

## Networking — How do I talk to you?

The native SDK runs one authenticated libp2p node per `Domain`. The host gives
`DomainBuilder` a stable identity, DDS Domain UUID, signed credential,
verification keys, listeners, and explicit routes. Noise binds the transport
Peer ID; the signed Domain token authorizes application protocols. Knowing an
address is never authority.

### What addresses it

- [`auki-p2p`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-p2p) — stable identity, mutually authenticated transport, explicit routes, relay reservations, and observations.
- [`auki-network`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-network) — bounded payload codecs and plain protocol types; no swarm.
- [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain) — public lifecycle, known peers, catalogs, registries, blobs, messages, and streams.
- [`auki-domain-relay`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain-relay) — Domain Relay for browser-compatible reachability through Circuit Relay v2 (WIP)
- [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session) — `Peer` + `Session` (identity, registries, log registration; network-free) and the `materialize_remote_log` stub

### The wire protocols

| Protocol | Purpose |
|----------|---------|
| `/auki/auth/1/info/1.0.0` | Bounded participant diagnostics |
| `/auki/auth/1/resources/0.2.0` | Resource catalog rows |
| `/auki/auth/1/resources/0.3.0` | Catalog rows plus message channels |
| `/auki/auth/1/resources/0.4.0` | Map Log catalog |
| `/auki/auth/1/registries/0.2.0` | Authenticated Get-only registry payload |
| `/auki/auth/1/registries/0.3.0` | Tagged Get and bounded Device Model List |
| `/auki/auth/1/blobs/0.1.0` | Verified content-addressed blobs |
| `/auki/auth/1/message/0.1.0` | Receiver-owned live messages |
| `/auki/auth/1/stream/0.2.0` | Typed streams bound to an expected producer |

Plus an [HTTP control API](https://github.com/aukilabs/auki-sdk/blob/develop/docs/control-api.md) — a separate operator-facing surface for daemons that produce SDK sessions (BoosterApp, Sentinel), so any UI like [Park](https://github.com/aukilabs/park) can drive them through a uniform contract.

### What's pending

- **`Session::materialize_remote_log`** — persistence of a verified remote
  stream as a local replica remains pending.
- **Browser Domain engine** — the authenticated native Rust and Python owner is
  implemented in source (the coordinated Stage 1 tag is pending); the
  equivalent browser engine is a later platform stage.

### How a consumer composes the answer

```rust
let config = DomainConfig::new(domain_id, identity)
    .with_listen_addresses(["/ip4/127.0.0.1/tcp/0".parse()?])?
    .with_peer_routes(expected_peer, routes)?;
let domain = Domain::builder(&peer, &session, config)
    .authority(verification_keys, signed_credential)
    .join()
    .await?;

let catalog = domain.catalog()?;
// Authenticated peers fetch the same live provider snapshot.
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

Domain::builder(&peer, &session, config)
    .authority(keys, credential)
    .join()
├── authenticated protocols + owned leave  ← Networking
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
