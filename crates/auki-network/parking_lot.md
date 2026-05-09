# Parking lot — auki-network

---

## `discovery_client` — `DiscoveryRuntime` (re-register / poll loop)

Vinland v1 ships `DiscoveryClient::register/fetch/deregister` as one-shots. The Notion doc explicitly defers a `DiscoveryRuntime` (long-lived task that re-registers periodically and/or polls for updates) until Discovery itself grows TTL (D1) or push (D2). When that happens:

- **Re-register loop.** Daemons should renew their entry every `ttl/3` or so to outlive Discovery's eviction. Suggests a small task: `DiscoveryRuntime::spawn(client, wallet, cluster_name, addresses, expected_app_id?, note?, period?)` that calls `register` on a tokio interval. Owns its own task handle; `shutdown(self)` deregisters and returns.
- **Poll-for-updates loop.** Daemons that want to see new peers without an operator nudge call `fetch(cluster_name)` on a slower interval (every 30–60s). Same runtime can host both loops.
- **Push channel.** If Discovery grows SSE / WebSocket, the poll loop swaps for a streaming consumer of the same shape. **Push side landed Vinland D6 — see [`subscribe` parking-lot decisions below](#vinland-d6--discovery_clientsubscribe-pre-implementation-decisions-filed-by-broodsugars-dobby-2026-05-09).** The push channel obviates the poll loop's need entirely; the re-register loop remains a separate question for whenever Discovery v2's TTL lands.

Defer until Discovery v2 lands. The current one-shot surface is forward-compatible — a `DiscoveryRuntime` builds *on* `DiscoveryClient`, doesn't replace it.

---

## `discovery_client` — TLS knobs / custom roots

`DiscoveryClient::new` builds a default `reqwest::Client` with rustls + webpki-roots; HTTPS against public CAs Just Works. Self-signed Discovery (LAN-internal HTTPS the operator runs themselves) needs a custom client via `DiscoveryClient::with_http(url, custom)`. That's the escape hatch today — operators who need it know enough to construct a `reqwest::Client::builder()` with `add_root_certificate(...)`.

Open: should `new` grow first-class kwargs for the common cases (`tls_config`, `proxy`, `connect_timeout`)? Lean toward no — `with_http` covers it, and a richer constructor is a forward-compat headache. Revisit if a real consumer (Sentinel, Park) hits the friction.

---

## Wallet → peer-key derivation label evolution

`PEER_DERIVATION_LABEL = "peer/v1"` is shipped. The `/v1` suffix is deliberate — when libp2p's PeerId encoding changes, or when we want to rotate the peer key without invalidating the wallet, we can ship `"peer/v2"` and have new code derive from the new label while old `ReachabilityRecord`s under the v1 PeerId still resolve.

Open: should the label format be richer? `"peer/<protocol>/<version>"` (e.g. `"peer/libp2p/v1"`) leaves room for future non-libp2p networking layers. `"peer/v1"` is shorter and assumes libp2p is the only networking substrate we'll ever have. Defer until a second substrate appears (or doesn't).

Related, in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md): BIP32-style HD vs labeled-hash derivation. If the SDK ever switches to BIP32 paths, the peer label becomes a derivation path like `m/44'/auki'/peer'/0'`. Re-derivation invalidates existing peer ids regardless of label format; this is a coordinated SDK + consumer change.

---

## `ReachabilityRecord` extensibility

Today it's `{peer_id, addresses, capabilities, last_seen_ns}`. Likely additions over time:

- **Operator metadata.** Friendly name, owning wallet id, geographic hint.
- **Health / load.** Open-circuit count, recent error rate, advertised capacity.
- **Auth.** Signed by the peer's wallet to prove ownership of the peer key — needs `auki-identity::CreationCert` shape extended for signing arbitrary structs.

Append-fields-with-`#[serde(default)]` is the easy path; a versioned wire format is the honest one. Decide before any consumer relies on the shape being stable.

---

## `SwarmConfig` minimalism — when do we add knobs?

M1's `SwarmConfig` has `listen_addresses`, `agent_version`, `enable_mdns`, `enable_relay_server`. Many libp2p knobs are baked-in: idle connection timeout (60s), identify protocol id (`/auki/identify/0.0.1`), ping defaults. This is deliberate — fewer knobs means fewer ways to mis-configure.

Knobs that consumers will likely want eventually:

- **Idle timeout.** Long-lived idle connections vs aggressive eviction. Daemons probably want longer; gateways shorter.
- **Ping interval / timeout.** Keepalive cadence vs liveness sensitivity.
- **Allowed transports.** Force-TCP-only or force-QUIC-only for testing or networks that block UDP.
- **Connection limits.** `libp2p::connection_limits::Behaviour` parameters.
- **Configured relay multiaddrs** for relay-clients to auto-register reachability with on startup. Currently the relay-client behaviour is wired in but no auto-dial of any specific relay; consumer code does the dial explicitly.

Default: don't expose any of these until a real consumer asks. Adding fields with `Default` impls is non-breaking; removing them is. Stay minimal.

---

## `BuildError::Transport(String)` — structured vs prose

`BuildError::Transport` currently wraps a `String` (formatted from libp2p's transport-setup errors). This loses type information — callers can't programmatically distinguish "tcp listen failed" from "noise key generation failed" from "mdns init failed." It also doesn't surface the underlying `std::io::Error` cleanly.

Three options:

1. **Keep `String`.** Simple; transport setup failures are rare and operator-facing anyway.
2. **Box the underlying error.** `Transport(Box<dyn std::error::Error + Send + Sync>)`. Preserves the chain via `Error::source()`.
3. **Enum the failure modes.** `Tcp`, `Quic`, `RelayClient`, `Noise`, `Yamux`, `Mdns`, `Behaviour` variants. Most type information; most maintenance burden as libp2p evolves.

Lean toward (2) once a consumer wants programmatic dispatch. (1) is fine for M1.

---

## Loopback test workaround — `add_external_address` for relay reservation

The `relay_server_accepts_reservation` test calls `relay_swarm.add_external_address(relay_addr)` so the relay's reservation response includes the loopback listen address (which the swarm doesn't auto-discover as external).

On real networks, external addresses get learned via identify (the client tells the relay what address it dialed) or AutoNAT. This isn't really a parking-lot question — just documented here so a future reader doesn't strip the `add_external_address` call thinking it's redundant. Move to a comment-only note if/when AutoNAT lands.

---

## `stream_protocol` — JSON encoding for binary `T` is wasteful

The grimsby D4-resolved framing is **JSON-serialized `StreamMessage<T>`** with a 4-byte length prefix. For grimsby v1 (`T = JpegFrame { bytes: Vec<u8> }`), `serde_json` renders the `bytes` field as a JSON array of integers — each byte becomes ~4 ASCII bytes (`"123,"`), producing roughly a 4× bandwidth hit vs. raw. For 30 fps × 100 KB JPEGs that's ~12 MB/s on the wire vs. ~3 MB/s raw — fine for a 1–4 robot LAN demo, real concern for anything larger.

**Resolved 2026-05-06 for `PointCloudFrame` only** (Dagaz Batch 1, D3 wire-side). Path (1) — `#[serde(with = "base64_bytes")]` adapter — applied to `PointCloudFrame.bytes` because pointcloud at 22 MB/s × 4 was untenable on a Wi-Fi LAN. The adapter lives at module scope in `stream_protocol`; `JpegFrame.bytes` was deliberately **not** updated to keep grimsby v1 wire compat (existing consumers — boosterapp's Python sidecar, Park's browser-side decoder — would fail closed on the encoding swap). Locked cross-language conformance vector pinned in [`stream_protocol::tests::locked_point_cloud_frame_wire_shape_vector`](src/stream_protocol.rs).

**Still open for `JpegFrame`** (and any future binary-heavy `T`). Three forward paths, in order of effort:

1. **Base64-encode `JpegFrame.bytes` inside JSON.** Same adapter `PointCloudFrame` already uses. Wire-compat-breaking — every grimsby consumer renegotiates. Defer until a JPEG consumer reports stutter or a producer reports outbound saturation; not a v1 bottleneck.
2. **Switch the codec to CBOR** (`ciborium` or `serde_cbor`). Native binary support; `Vec<u8>` rides as raw bytes. Wire-compat-breaking for everyone; no longer human-readable in tcpdump but still serde-driven.
3. **Hybrid framing** — JSON envelope with a `payload_size: u32` field, plus a separate length-prefixed binary section after. Most efficient, most bespoke; preserves human readability of the envelope.

Path (2) is a coordinated bump for every `Stream<T>` consumer at once. Path (3) is the most engineering work but doesn't sacrifice tcpdump readability of the envelope. Stay deferred until a real consumer asks.

## `stream_protocol` — `libp2p-stream` 0.4.0-alpha pin

`libp2p-stream` ships separately from the `libp2p` umbrella crate (the umbrella's 0.56 release does not expose a `stream` feature flag). The version that pairs with `libp2p` 0.56 / `libp2p-swarm` 0.47 is `0.4.0-alpha` — pre-1.0. We pin exactly (`= 0.4.0-alpha`) to avoid surprise breakage when an alpha-2 ships with API churn. Relax to `^0.4` (or `^X.Y` once it stabilizes) when the upstream surface stops moving. No action items today; revisit at the next libp2p bump.

## DCUtR / hole-punching — when?

Not in M1. libp2p `dcutr::Behaviour` upgrades a relayed connection to a direct one via simultaneous-open hole-punching. Small additive change to the `Behaviour` composition; not load-bearing for the M2 demo (Park dialing K1 through a relay works either way).

Ship it when (a) the M2 demo is end-to-end and (b) Park-from-home traffic volume warrants reducing relay load.

---

## Cluster Registry primitive — does `cluster.json` graduate?

ansuz #1 ships `cluster.json` as a **flat** single-file directory under `<app_root>/registries/cluster_registries/cluster.json`, deliberately **not** hash-keyed like the existing `Sensor` / `Clock` / `Frame` registries. The flat shape is right for ansuz: a small handful of pinned peers, edited by hand, no per-cluster history needed.

Open: when (if ever) does this graduate to a real Cluster Registry primitive — hash-keyed entries at `<app_root>/registries/cluster_registries/<cluster_id>/<hash>.json`, content-addressed so consumers can pin a specific cluster snapshot? Plausible triggers:

- **Multi-cluster daemons.** A single daemon participating in more than one cluster (e.g. operator's home cluster + a partner cluster) can't represent that with one `cluster.json`.
- **Cluster-membership history.** Replay needs to know "which peers were in the cluster at time `t`" — flat overwrite loses this.
- **Cryptographic attestation.** Once `cluster.json` is signed, an immutable hash-keyed file is the natural shape (attestation binds to bytes).

Defer until one of those earns it. The flat path is a forward-compatible subset — a hash-keyed Cluster Registry can coexist with `cluster.json` for as long as we want.

---

## `cluster.json` signing — when?

The doc is unsigned for ansuz. Trust is "operator wrote this file"; tampering on disk is out of scope. Once a Wallet-backed signing primitive is available (the partial answer in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md)'s "Encrypted-at-rest format" thread), `cluster.json` becomes a natural candidate: sign the doc with the cluster operator's wallet, distribute the public key alongside, every reader verifies. Likely shape: a sibling `cluster.json.sig` rather than embedded signature — keeps the JSON itself diff-friendly.

---

## Operator UX for peer-id discovery

The brief notes integrators wire `--cluster-doc <path>` through their CLI. Open question for daemon authors: how does an operator obtain the peer-ids to put in the doc in the first place?

- BoosterApp / Sentinel can print their own peer-id to stdout at startup (and to `/api/info` over HTTP). Operator copies it.
- Park can render a peer-id when an operator hits "show network identity."
- A dedicated `auki peer-id` CLI subcommand on a SDK-driven binary would close the loop without needing the daemon running.

Not a `cluster.json` concern per se — just adjacent. Document the recommended pattern in the `cluster.json` spec section once one daemon has the operator-facing UX nailed down.

---

## `app_instance` — container / Docker handling

`app_instance::derive()` typically returns `NoSuitableMac` inside a Docker container — the bridge-network interface gets a locally-administered MAC (first octet `0x02`), and there's usually no IEEE-administered NIC visible from inside the container. ansuz accepts this; daemons running in containers will need a fallback strategy (envvar override? hostname-derived? wallet-derived persisted?) before the SDK is comfortable in containerized deployments.

Pin a story before the first daemon is shipped in a container.

## `app_instance` — multi-NIC tiebreaker semantics

Today: lex-smallest MAC wins among non-loopback IEEE-administered candidates. Adding or removing a NIC can shift which MAC sorts first → the daemon's `app_instance` changes even though the machine didn't.

Alternatives if this becomes painful:
- **Pin to a specific NIC** by name on first boot, persist that name. Shifts the question to "what about NIC renaming."
- **Hash all eligible MACs** rather than picking one. Stable under add/remove? No — adding a NIC still changes the input set.
- **Combined wallet-derived + first-boot persistence** (see next item) — stop relying on hardware altogether.

Not blocking ansuz; revisit if real deployments hit it.

## `Capability(pub String)` — open-string vs typed enum _(filed by Dobby, 2026-05-08)_

`auki-network` exports `pub struct Capability(pub String);` at crate root, used inside `ParticipantInfo` and `ReachabilityRecord`. A reader of the public surface today cannot tell whether this is *deliberately* open-string (forward-compat for capabilities a consumer hasn't seen yet, e.g. a future `/auki/credits/1.0.0` advertised by a peer running a newer SDK) or just under-typed.

Two options:

1. **Document the open-string-by-design contract** with a short doc-comment on `Capability` ("opaque protocol-id string; consumers do not need to enumerate to recognize a single value they care about; new capabilities ship without an SDK bump"). Keeps it forward-compat. Add a parking-lot entry on the consumer side noting that consumers should compare-by-string-equality, not pattern-match.
2. **Tighten to a typed enum with an `Other(String)` escape hatch**: `enum Capability { ClusterV1, StreamV1, Other(String) }`. Lets consumers exhaustively match the known protocols while still surviving an unknown future protocol-id from a peer. Costs: every new SDK protocol becomes a variant addition (small chore, but a public-API touch); `Other(String)` doesn't fully escape the round-trip problem because two strings denoting the same future capability could differ in casing/whitespace.

Lean: (1). The same forward-compat reasoning that produced `PEER_DERIVATION_LABEL = "peer/v1"` applies to capabilities — protocol-ids are versioned strings, not closed sets. Add a doc-comment, keep the newtype, leave the open-string contract explicit. No urgency; pin before any consumer hard-codes pattern-matching.

---

## `PEER_DERIVATION_LABEL` constant — wrong crate _(filed by Dobby, 2026-05-08)_

`pub const PEER_DERIVATION_LABEL: &str = "peer/v1"` lives at the root of `auki-network`. The constant's *meaning* belongs to [`auki-identity`](../auki-identity) — it's the label fed to `Wallet::derive_child(...)` to materialize the peer key from the wallet seed. Only the *consumer* of that derivation (the libp2p layer) lives in `auki-network`.

A reader who lands in `auki-identity` looking for "what labels can I derive a child for?" finds no canonical list — the SDK's most-load-bearing label is one crate over. A reader who lands in `auki-network` finds a label constant whose semantics they cannot resolve without crossing into `auki-identity`.

Two forward paths:

1. **Move the constant to `auki-identity`** (e.g. `auki_identity::derivation::PEER_LABEL`); have `auki-network` re-export it for backward-compat: `pub use auki_identity::derivation::PEER_LABEL as PEER_DERIVATION_LABEL;`. Cheapest split — no source breakage, label semantics now live next to the wallet primitive that consumes them.
2. **Move the constant + introduce a labels module in `auki-identity`** as the canonical home for every label any future child derivation will use (e.g. an eventual `app_instance/v1` per the Wallet-derived alternative discussed in this same parking-lot above). Sets up the convention before a second label needs it.

Cross-references the existing [Wallet → peer-key derivation label evolution](#wallet--peer-key-derivation-label-evolution) thread above and the [BIP32-vs-labeled-hash derivation](../auki-identity/parking_lot.md) thread in `auki-identity`. Picking (2) makes the most sense the moment a second label is committed to.

---

## `StreamDispatch` is the streaming-stability lever — README should call it out _(filed by Dobby, 2026-05-08)_

`pub enum StreamDispatch { AcceptJpeg, AcceptPointCloud, Decline }` is a **closed** enum. Adding a new payload type — when an SLAM odometry stream or a cell-phone-camera variant lands — is a coordinated SDK + consumer release: bump the crate, add the variant, every consumer that wants the new sensor type opts in. The May 6 changelog entry (Dagaz Batch 1 #1) explicitly lays out the rationale ("trait-object dispatch (open-set) was rejected because Rust generics + serde bounds don't compose well across `dyn Fn` boundaries…").

The decision is correct. The disclosure is missing. The root [`README.md`](../../README.md) "API surface" section presents `StreamDispatch` as an implementation detail of `/auki/stream/0.1.0` ("dispatched by `sensor_id` via the closed `StreamDispatch` enum"). To a downstream consumer reading the README to plan their integration, that's an aside — but it's actually the SDK's primary stability lever for streaming. Every new `T` is a public-API touch; that's the point.

Suggest: add one sentence to the API-surface table's `/auki/stream/0.1.0` row, or to the libp2p wire-protocols section that follows, calling out the closed-enum stability model explicitly. Something like *"New payload types ship as a coordinated SDK release: a new `StreamDispatch` variant is a public-API change consumers opt into."* Doc-only; no code touch.

Surfacing for editorial pass; not gating anything.

---

## `app_instance` — eventual stable-id options

MAC-by-convention is fragile in containers, VMs, and multi-NIC environments (see above). Long-term candidates for a stable per-machine id:

- **Wallet-derived, persisted on first boot.** `Wallet::derive_child("app_instance/v1")` → 16 hex chars; daemon writes it to `<state_dir>/app_instance` on first run, reads thereafter. Survives NIC changes and container restarts (state-dir-permitting). Loses identity if `<state_dir>` is wiped.
- **OS machine-id** (`/etc/machine-id` on Linux, `IOPlatformUUID` on macOS, MachineGuid in Windows registry). Cross-platform but each platform has its own gotchas (machine-id can be stale-cloned across VM templates, IOPlatformUUID is reset by some firmware updates).
- **MAC + persisted nonce** — hash the MAC together with a per-install random value, persist the result. Decouples the public id from the underlying MAC; new MAC selection still produces the same id.

Decide before any cross-machine coordination relies on `app_instance` being stable. ansuz only needs distinguishability, not stability.

---

## Vinland D6 — `discovery_client::subscribe` pre-implementation decisions _(filed by broodsugar's dobby, 2026-05-09)_

Discovery's SSE endpoint (`GET /clusters/{cluster_name}/events`, `event: cluster_doc\ndata: {ClusterDoc-JSON}\n\n`) shipped 2026-05-07 against `aukilabs/discovery` commit `97c4dd8`. SDK side adds a fourth method to `DiscoveryClient` next to `register / fetch / deregister`:

```rust
pub async fn subscribe(
    &self,
    cluster_name: &str,
) -> Result<impl Stream<Item = Result<ClusterDoc, SubscribeError>> + Send + 'static, SubscribeError>;
```

Paired with `ClusterRuntime::update_cluster_doc(new_doc)` so the daemon has somewhere to deliver fresh docs without tearing down the runtime. Single PR; the two pieces are tightly coupled. Daemon-side adoption (boosterapp / park / sentinel each pick up `subscribe` and feed the doc into their `ClusterRuntime`) is per-daemon-repo follow-ups after the SDK ships.

Four pre-implementation decisions filed before the implementing PR per the [auki-labs-repos convention](../../CLAUDE.md):

### Decision — reconnect / backoff in `subscribe`

**Decided 2026-05-09. Caller owns retry; `subscribe` ends on transport failure.** Reasons: (a) any retry policy baked in (jittered exponential backoff? max retries? circuit breaker?) becomes opinionated before any daemon has hit the wall and told us what semantics they need; (b) the natural place for retry is the daemon's outer supervisory loop, which already exists for `register`; (c) making `subscribe` "just ends on failure" matches the shape of the other three `DiscoveryClient` methods (one-shot semantics, caller decides what's next). Revisit if the first daemon shipping `subscribe` reports the boilerplate is non-trivial; a sibling helper (`DiscoveryRuntime::subscribe_with_reconnect(...)` etc.) is the right place for retry, not the primitive itself.

### Decision — lag signal in the subscribe stream

**Decided 2026-05-09. Silently drop intermediate events; the next emitted `ClusterDoc` reconciles.** Reasons: (a) Discovery's `tokio::sync::broadcast::channel(16)` per `cluster_name` already drops events for receivers more than 16 events behind, but the next event still carries the full snapshot (idempotent recovery); (b) surfacing `Lagged` to the SDK consumer would force the consumer to decide what to do with a state that resolves itself within one more event (resubscribe? log?); (c) cluster-membership events are convergent — the consumer cares about "current peers," not "every transition," and a snapshot delivers the former. Revisit if a consumer ever needs strong ordering / no-loss semantics for cluster events (currently no consumer does).

### Decision — diff events vs full snapshots on the wire

**Decided 2026-05-09. Snapshots only for v1; the wire shape is locked.** Reasons: (a) snapshots are simpler to encode (`ClusterDoc` JSON; no cross-event state machine); (b) idempotent under reconnect — a fresh subscriber and a reconnecting subscriber are indistinguishable from Discovery's side; (c) survive lagged subscribers without bookkeeping (the lag-decision item above depends on this); (d) cluster sizes for the demo are <10 peers, snapshot bandwidth is negligible. Revisit when a single cluster crosses ~100 peers and snapshot serialization shows up in profiles. The forward path is additive — Discovery could emit a `cluster_doc_delta` event type in parallel without breaking `cluster_doc` consumers.

### Decision — multi-cluster `subscribe` on the same `DiscoveryClient` instance

**Decided 2026-05-09. v1 spawns one HTTP connection per `subscribe` call.** Reasons: (a) `DiscoveryClient`'s existing `register` / `fetch` / `deregister` methods are one-shot HTTP; an SSE long-poll on top of that is the simplest extension; (b) the v1 case is one-cluster-per-daemon (boosterapp / park / sentinel each subscribe to their own cluster, not multiple); (c) connection pooling semantics for long-lived SSE streams are non-obvious (per-host connection limits in `reqwest`, keepalive interaction, connection-shutdown ownership). Revisit if a daemon ever subscribes to multiple clusters from the same client — at that point the question is "do we share a single connection, or open one per cluster" and either answer needs explicit design.
