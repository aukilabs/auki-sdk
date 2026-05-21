# Parking lot — auki-network

---

## `swift-bindings` cargo feature

PR A added an additive `swift-bindings` feature that gates UniFFI proc-macros on `PeerIdentity` only. PR B extended its scope to the full v0 networking surface: `Wallet` (via `auki-identity/swift-bindings`), `PeerIdentity`, `NetworkRuntime`, `AllowedPeer`, `SpawnError`, `UpdateError`, `UpdateReport`, `StreamEntry`, `StreamError`, `OpenStreamError`, `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, `DiscoveryError`, and the 5 `StreamSubscription*` Objects (`StreamSubscriptionAudio`, `StreamSubscriptionCamera`, `StreamSubscriptionPointCloud`, `StreamSubscriptionJointEncoders`, `StreamSubscriptionDetection`). The feature lives here, not in a separate `*-swift` upstream crate. PR C (`auki-domain-swift`) will add further annotations to this crate and to `auki-domain` as needed.

---

## SDK relay-reservation helper — v2 story for NAT/firewalled daemons _(filed by Nils's claude, 2026-05-14)_

`auki_network::swarm::resolve_advertise_multiaddrs` shipped 2026-05-14 for v1 (LAN-only demo, plus operator-override for multi-NIC / VPN / container-host ambiguity). It does NOT solve the dual-firewalled case where neither peer has a public IP — those daemons need libp2p Circuit Relay v2: dial a relay → reserve a slot → listen on a circuit address → libp2p emits a `NewListenAddr` with the assembled `/dns4/relay/.../p2p-circuit/p2p/<self>` multiaddr. v1 operators handle this themselves by hand-assembling the circuit multiaddr and passing it as `external_addresses` (replace-semantics override). v2 should provide an SDK helper that owns the dial + reserve + listen dance and surfaces the resolved circuit address back to the caller, so daemons don't reimplement it.

Open design questions to land BEFORE the v2 helper ships:

1. **Helper shape.** Narrow `reserve_relay_listen(swarm, relay_peer_id, relay_multiaddrs, timeout) -> Result<Multiaddr, RelayReservationError>` that returns just the resolved circuit multiaddr — caller threads it into `resolve_advertise_multiaddrs` as the override? Or a wider helper that owns the whole "relay-mode swarm setup" (build_swarm + reserve + listen + advertise) so the daemon flow stays a single call? Narrow leans cleaner; wider is more operator-friendly. Lean narrow ~60%, revisit when Park / Boosterapp adoption surfaces the actual ergonomics.

2. **Default relay address.** Does the SDK ship a baked-in default (Auki-operated relay at a well-known address), or is the relay address always operator-supplied? If baked in: who runs the relay, where (LAN-internal `aukilabs/relay`? cloud-hosted?), with what SLA? Note an `aukilabs/relay` infrastructure-node design already exists per the swarm module's `enable_relay_server` toggle — pairs with this question.

3. **Discovery as relay directory.** Does Discovery distribute the relay list (extending today's "clusters + manager addresses" surface), or is relay discovery strictly out-of-band (CLI flag / env / config file)? Upside: one bootstrap channel + one configuration knob for operators. Downside: Discovery now owns a second resource type (relays alongside clusters), which adds API surface and shifts Discovery from "stateless directory of clusters" toward "general bootstrap service." Lean out-of-band for now (matches how `--discovery-url` itself is operator-supplied today); revisit if operator-UX friction surfaces it.

4. **Multi-relay redundancy.** If a daemon is configured with multiple relays for failover, does the SDK manage the reservation across all of them (try-each-on-disconnect), or is the operator responsible (pass N circuit multiaddrs as `external_addresses`, libp2p's transport picks one at dial time)? Lean operator-managed for v2; SDK-managed failover is a separate v3 task with its own state-machine complexity.

5. **DCUtR coupling.** Existing parking-lot item "DCUtR / hole-punching — when?" below names DCUtR as not-in-M1. The relay-reservation helper and DCUtR are mutually orthogonal (you can ship reservation without DCUtR — traffic just stays on the relay) but they're often deployed together. Land them as one v2 milestone or sequence them? Lean sequence: ship relay-reservation first because it's the load-bearing primitive (without it, dual-NAT daemons can't communicate at all); DCUtR is an optimization that earns its weight when relay traffic volume hurts.

6. **Operator UX shape.** Boosterapp (headless): another CLI flag (`--relay-multiaddrs <addr>...`)? Park (GUI): an "Advanced settings" panel where the operator pastes a relay multiaddr? Or `external_addresses` continues to subsume both (operator pastes a fully-assembled `/dns4/relay/.../p2p-circuit/p2p/<self>` for v1, and v2 adds dedicated flags when the dial-reserve-listen happens inside the SDK)? Lean `external_addresses`-as-escape-hatch for v1 (matches what's shipped today); dedicated `--relay-multiaddrs` for v2 when the SDK helper lands and the operator no longer needs to assemble the circuit address by hand.

Scope landing trigger for the v2 work: when (a) the v1 LAN demo is end-to-end, and (b) Park-from-home or a similar two-network scenario earns the engineering.

---

## Restore the 6 deleted producer/consumer stream tests against `NetworkRuntime::spawn` _(filed by Nils's claude, 2026-05-13)_

When `ClusterRuntime` was replaced by `NetworkRuntime`, the 6 multi-runtime `#[tokio::test]` integration tests in `src/stream_runtime.rs` were deleted along with the `ClusterDoc` / `ParticipantInfo` / `participant_provider` fixture they depended on. The stream protocol itself is unchanged; only the runtime construction shape moved. The 7 stream-protocol wire-shape unit tests still cover the on-wire format; the missing coverage is the end-to-end producer-pair scenarios:

- `producer_accepts_and_streams_camera_frames`
- `producer_declines_unknown_sensor`
- `producer_error_signals_consumer_with_detail`
- `producer_shutdown_signals_consumer_with_typed_end_of_stream`
- `open_stream_against_unreachable_peer_surfaces_typed_error`
- `producer_accepts_and_streams_pointcloud_frames`

Port plan: construct `NetworkRuntime` pairs via `spawn` with mutual `AllowedPeer`s; replace any `consumer.peers().iter().any(...)` checks with `consumer.connected_peers().contains(&...)`. Mechanical port; ~200 LOC of fixture rewiring. These tests are the stream-protocol coverage we lean on; restore before any stream-touching change.

---

## `discovery_client` — `DiscoveryRuntime` (re-register / poll loop)

v1 ships `DiscoveryClient::register/fetch/deregister` as one-shots. A `DiscoveryRuntime` (long-lived task that re-registers periodically and/or polls for updates) is deferred until Discovery itself grows TTL or push. When that happens:

- **Re-register loop.** Daemons should renew their entry every `ttl/3` or so to outlive Discovery's eviction. Suggests a small task: `DiscoveryRuntime::spawn(client, wallet, cluster_name, addresses, expected_app_id?, note?, period?)` that calls `register` on a tokio interval. Owns its own task handle; `shutdown(self)` deregisters and returns.
- **Poll-for-updates loop.** Daemons that want to see new peers without an operator nudge call `fetch(cluster_name)` on a slower interval (every 30–60s). Same runtime can host both loops.
- **Push channel.** If Discovery grows SSE / WebSocket, the poll loop swaps for a streaming consumer of the same shape.

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

## `stream_protocol` — `libp2p-stream` 0.4.0-alpha pin

`libp2p-stream` ships separately from the `libp2p` umbrella crate (the umbrella's 0.56 release does not expose a `stream` feature flag). The version that pairs with `libp2p` 0.56 / `libp2p-swarm` 0.47 is `0.4.0-alpha` — pre-1.0. We pin exactly (`= 0.4.0-alpha`) to avoid surprise breakage when an alpha-2 ships with API churn. Relax to `^0.4` (or `^X.Y` once it stabilizes) when the upstream surface stops moving. No action items today; revisit at the next libp2p bump.

---

## DCUtR / hole-punching — when?

Not in M1. libp2p `dcutr::Behaviour` upgrades a relayed connection to a direct one via simultaneous-open hole-punching. Small additive change to the `Behaviour` composition; not load-bearing for the M2 demo (Park dialing K1 through a relay works either way).

Ship it when (a) the M2 demo is end-to-end and (b) Park-from-home traffic volume warrants reducing relay load.

---

## Cluster Registry primitive — does `cluster.json` graduate?

v1 ships `cluster.json` as a **flat** single-file directory under `<app_root>/registries/cluster_registries/cluster.json`, deliberately **not** hash-keyed like the existing `Sensor` / `Clock` / `Frame` registries. The flat shape is right for v1: a small handful of pinned peers, edited by hand, no per-cluster history needed.

Open: when (if ever) does this graduate to a real Cluster Registry primitive — hash-keyed entries at `<app_root>/registries/cluster_registries/<cluster_id>/<hash>.json`, content-addressed so consumers can pin a specific cluster snapshot? Plausible triggers:

- **Multi-cluster daemons.** A single daemon participating in more than one cluster (e.g. operator's home cluster + a partner cluster) can't represent that with one `cluster.json`.
- **Cluster-membership history.** Replay needs to know "which peers were in the cluster at time `t`" — flat overwrite loses this.
- **Cryptographic attestation.** Once `cluster.json` is signed, an immutable hash-keyed file is the natural shape (attestation binds to bytes).

Defer until one of those earns it. The flat path is a forward-compatible subset — a hash-keyed Cluster Registry can coexist with `cluster.json` for as long as we want.

---

## `cluster.json` signing — when?

The doc is unsigned for v1. Trust is "operator wrote this file"; tampering on disk is out of scope. Once a Wallet-backed signing primitive is available (the partial answer in [`auki-identity/parking_lot.md`](../auki-identity/parking_lot.md)'s "Encrypted-at-rest format" thread), `cluster.json` becomes a natural candidate: sign the doc with the cluster operator's wallet, distribute the public key alongside, every reader verifies. Likely shape: a sibling `cluster.json.sig` rather than embedded signature — keeps the JSON itself diff-friendly.

---

## Operator UX for peer-id discovery

The brief notes integrators wire `--cluster-doc <path>` through their CLI. Open question for daemon authors: how does an operator obtain the peer-ids to put in the doc in the first place?

- BoosterApp / Sentinel can print their own peer-id to stdout at startup (and to `/api/info` over HTTP). Operator copies it.
- Park can render a peer-id when an operator hits "show network identity."
- A dedicated `auki peer-id` CLI subcommand on a SDK-driven binary would close the loop without needing the daemon running.

Not a `cluster.json` concern per se — just adjacent. Document the recommended pattern in the `cluster.json` spec section once one daemon has the operator-facing UX nailed down.

---

## `app_instance` — container / Docker handling

`app_instance::derive()` typically returns `NoSuitableMac` inside a Docker container — the bridge-network interface gets a locally-administered MAC (first octet `0x02`), and there's usually no IEEE-administered NIC visible from inside the container. v1 accepts this; daemons running in containers will need a fallback strategy (envvar override? hostname-derived? wallet-derived persisted?) before the SDK is comfortable in containerized deployments.

Pin a story before the first daemon is shipped in a container.

---

## `app_instance` — multi-NIC tiebreaker semantics

Today: lex-smallest MAC wins among non-loopback IEEE-administered candidates. Adding or removing a NIC can shift which MAC sorts first → the daemon's `app_instance` changes even though the machine didn't.

Alternatives if this becomes painful:
- **Pin to a specific NIC** by name on first boot, persist that name. Shifts the question to "what about NIC renaming."
- **Hash all eligible MACs** rather than picking one. Stable under add/remove? No — adding a NIC still changes the input set.
- **Combined wallet-derived + first-boot persistence** (see next item) — stop relying on hardware altogether.

Not blocking v1; revisit if real deployments hit it.

---

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

## `app_instance` — eventual stable-id options

MAC-by-convention is fragile in containers, VMs, and multi-NIC environments (see above). Long-term candidates for a stable per-machine id:

- **Wallet-derived, persisted on first boot.** `Wallet::derive_child("app_instance/v1")` → 16 hex chars; daemon writes it to `<state_dir>/app_instance` on first run, reads thereafter. Survives NIC changes and container restarts (state-dir-permitting). Loses identity if `<state_dir>` is wiped.
- **OS machine-id** (`/etc/machine-id` on Linux, `IOPlatformUUID` on macOS, MachineGuid in Windows registry). Cross-platform but each platform has its own gotchas (machine-id can be stale-cloned across VM templates, IOPlatformUUID is reset by some firmware updates).
- **MAC + persisted nonce** — hash the MAC together with a per-install random value, persist the result. Decouples the public id from the underlying MAC; new MAC selection still produces the same id.

Decide before any cross-machine coordination relies on `app_instance` being stable. v1 only needs distinguishability, not stability.

---

## `/auki/stream/0.1.0` — operator visibility into stream subscribers _(filed by Nils, 2026-05-12)_

`NetworkRuntime` doesn't surface who's currently subscribed to streams from this node. Operators inspecting BoosterApp's `/api/cluster` see the cluster's peer list but not which of those peers are actively pulling frames.

Two halves:

- **SDK side (this crate).** Add a `runtime.stream_subscribers() -> Vec<(PeerId, StreamRequest)>` accessor for currently-open inbound substreams. Lifecycle: an entry appears when `handle_inbound_substream` calls the provider with a non-`Decline` dispatch, disappears when the pump task ends (substream dropped, peer disconnected, source ended, shutdown). Bookkeeping: a `tokio::sync::RwLock<HashMap<...>>` (or actor-pattern command on the runtime); the pump task inserts on accept, removes on drop via a `Drop`-guard wrapper. Read-side only — no behavior change.
- **Daemon side (out of crate).** Expose the SDK accessor via a new HTTP endpoint (e.g. `GET /api/streams/subscribers` returning JSON `[{peer_id, sensor_id}]`) in BoosterApp / Park / Sentinel control APIs. Out of scope for `auki-network`; file in each daemon repo once the SDK accessor ships.

**Lean.** Ship the SDK accessor first, non-invasive. Daemons add their HTTP shims afterward.
