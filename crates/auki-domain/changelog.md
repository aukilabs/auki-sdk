# Changelog — auki-domain

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

**Successor-token parking-lot option updated to `auki-proto`.** The v2 hardening question now frames the prost option as a root-proto / generated-`auki-proto` message instead of pointing at the removed `auki-datatypes` crate.

### Nils's codex · May 21, HKT, 2026

**Domain resource metadata now follows the final camera stream vocabulary.** Cluster resource lifting and stream manifests use `camera_frame` payload metadata with the `"camera"` sensor tag, matching the renamed datatypes, registry, and network surfaces.

### Nils's codex · May 21, HKT, 2026

**Heartbeat timestamps and domain-clock backing now use `ClusterManager`'s `SessionClock`.** `create_cluster` and `join_cluster` pass the SDK-owned peer-id rooted `SessionClock` into `HeartbeatTimestampSource`, initial domain-clock metadata, and promoted-Manager domain-clock advertisement. `DaemonInfo.session_clock_id/hash` remain compatibility inputs for old callers but no longer define heartbeat timing or the backing clock identity used for domain time.

Tests: `cargo test -p auki-domain cluster_manager::tests -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**ClusterManager now exposes domain time now.** Added `ClusterManager::domain_time_now()`, which reads the local session monotonic clock once, composes the current cluster domain-clock estimate, and converts that reading into `<cluster_name>/domain-clock`. The method returns typed unavailable errors from `domain_clock_estimate()` and a typed overflow error if the affine conversion exceeds `i64`; it never falls back to wall time.

Tests: `cargo test -p auki-domain domain_time_now -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**Live regression pins domain-clock continuity through Manager handoff.** Added an ignored `cluster_manager_integration` test where A creates a cluster, B joins, B receives A-backed heartbeat domain-clock metadata plus NTP samples until `domain_clock_estimate()` is available, A drops, and B promotes. The test verifies B's post-promotion domain estimate is backed by B's own session clock with the inherited domain offset.

Tests: `cargo test -p auki-domain --test cluster_manager_integration domain_clock_metadata_survives_manager_handoff -- --ignored --nocapture --test-threads=1` against `http://192.168.9.130:8080`.

### Nils's codex · May 20, HKT, 2026

**Promoted Managers advertise domain time only after proving their inherited offset.** The Manager-loss promotion path now tries to compose the promoted peer's local session clock into the cluster domain clock using the stored heartbeat domain source and the current `auki-time` peer-clock estimate. If that succeeds, the promoted peer switches its outbound heartbeat `domain_clock` metadata to itself as backing source with `backing_to_domain_offset_ns = local_session_clock -> domain_clock`. If the source or NTP estimate is missing, it remains silent and keeps returning explicit unavailable state.

Tests: `cargo test -p auki-domain promoted_manager_ -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**ClusterManager now exposes explicit domain-clock availability.** Heartbeat `domain_clock` metadata from the backing peer is stored as a local fact, and `domain_clock_estimate()` composes it with the current `auki-time` peer-clock estimate to return a local session clock -> `<cluster_name>/domain-clock` estimate. Initial Managers use an exact local identity transform; followers return `SourceUnavailable` before receiving domain-clock metadata and `BackingEstimateUnavailable` before their local -> backing NTP estimate exists. No wall-clock fallback is used.

Tests: `cargo test -p auki-domain domain_clock_estimate_ -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**Initial Managers now advertise the cluster domain-clock source on heartbeat frames.** `ClusterManager::create_cluster` seeds heartbeat metadata declaring `<cluster_name>/domain-clock` with a deterministic hash, backed by the creator peer's session clock id/hash at offset `0`. `join_cluster` leaves the metadata absent, so followers do not pretend to have domain time until a later continuity path computes and publishes their inherited offset.

No follower-side storage or domain-clock estimate surface changes in this slice; `ClusterManager` still forwards timing samples to `auki-time` and leaves transform math there.

Tests: `cargo test -p auki-domain heartbeat_source_ -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**Heartbeat NTP sample events now feed `auki-time` peer-clock estimates.** `ClusterManager` owns a cloneable `ClockSyncHandle`, forwards `PeerLivenessEvent::HeartbeatNtpSampleObserved` into it, and exposes read-only `clock_sync_estimate(...)` / `clock_sync_estimates()` accessors. Sample retention, uncertainty filtering, hash-reset behavior, and best-sample selection remain in `auki-time`; the domain layer only adapts event flow.

Tests: `cargo test -p auki-domain heartbeat_ntp_sample_event_updates_clock_sync_handle -- --nocapture`.

### Nils's codex · May 20, HKT, 2026

**ClusterManager ignores raw heartbeat NTP sample events for now.** `PeerLivenessEvent` now has `HeartbeatNtpSampleObserved` for matched heartbeat echo samples. The liveness handler explicitly ignores that variant so `ClusterManager` remains responsible for topology/liveness only; a future transform collector will decide how to use samples inside a cluster.

Tests: `cargo test -p auki-domain`.

### Nils's codex · May 20, HKT, 2026

**ClusterManager accepts heartbeat timing observations without owning NTP math.** The liveness handler now matches the expanded `PeerLivenessEvent::HeartbeatReceived { peer_id, observation }` shape and continues to use it only as a watched-peer-alive signal. The raw timing observation remains available above `auki-network`; transform calculation stays reserved for `auki-time`.

Tests: `cargo test -p auki-domain`.

### Nils's codex · May 20, HKT, 2026

**ClusterManager supplies heartbeat timestamps from the daemon's session clock.** `create_cluster` and `join_cluster` now build the required `HeartbeatTimestampSource` from `DaemonInfo.session_clock_id`, `DaemonInfo.session_clock_hash`, and `session_started.elapsed()` so `/auki/heartbeat/0.0.1` frames identify the sender's concrete monotonic clock without falling back to an invented or best-effort system source.

This keeps heartbeat time provenance tied to the peer's session clock while leaving domain-clock semantics in `auki-domain`: the Manager remains the source of the domain-clock backing clock, and later time-sync code can produce transforms from peer monotonic clocks toward that stable cluster clock.

Tests: `cargo check -p auki-domain`, `cargo test -p auki-domain`.

### Nils's codex · May 20, HKT, 2026

**`ClusterManager` session time now uses `auki_time::SessionClock`.** Create/join construction mints a peer-id rooted session clock from the local libp2p peer id and `DaemonInfo.session_id`. `ParticipantInfo.session_now_ns`, `session_clock_id`, and `session_clock_hash` now come from that SDK clock in both the local accessor and `/auki/info/0.0.1` handler; caller-provided `DaemonInfo.session_clock_id/hash` remain accepted only as compatibility inputs. Added `auki-time` as a dependency and documented the timekeeping ownership/deferred heartbeat-sync boundary. Tests: `cargo test -p auki-domain cluster_manager::tests`.

### Nils's codex · May 19, HKT, 2026

**Manager election now excludes the peer that heartbeat timed out.** When a non-Manager detects the current Manager as lost, `ClusterManager` no longer lets stale libp2p transport state re-elect that dead Manager just because `NetworkRuntime::connected_peers()` has not observed `ConnectionClosed` yet. The handoff path filters the heartbeat-lost peer out of both the connected set and membership snapshot before running the deterministic successor election, so battery-pull / QUIC-idle-timeout cases can still promote the earliest surviving member and rotate Discovery.

Tests: `cargo test -p auki-domain`.

### Nils's codex · May 18, HKT, 2026

**Resource catalogs advertise camera streams as `pinhole_camera_log_entry`.** `stream_payload_for_sensor_kind("rgb_camera")` now returns the camera-log record payload name instead of `jpeg_frame`, matching the `/auki/stream/0.1.0` camera stream change. The generic `ClusterManager::open_stream<T>` docs now name `PinholeCameraLogEntry` as the camera payload type.

Tests: `cargo check -p auki-network -p auki-network-py -p auki-domain-py -p auki-datatypes`.

### Dobby · May 17, HKT, 2026

**Parking-lot purged: Greenland design archaeology and resolved-but-unpropagated items removed.** Eleven "Greenland Decision" blocks (T1, T2/Q2, T3/Q1, T4/Q3, T5/Q4, T6/Q8, T7/Q5+Q-disc-1, T10/Q10, T11/Q11, T13/Q9, T9/Q12, T12/Q7, T8/Q-disc-2, T14/Q14) transcribed from the retired Greenland Notion task table were deleted — the decisions all shipped (`ClusterManager`, heartbeat, `/auki/registries/0.0.1`, election rule, default-Domain singleton) and the parking-lot transcripts had become plan archaeology. The "Glossary reconciliation" item self-marked Resolved 2026-05-11 with a week-stale Propagate placeholder was also removed (Glossary already carries the live `Domain Identity` definition). Codename leakage ("Hagall", retired "Vinland"/"Greenland") stripped from the three live forward-looking items that remain: successor-token encoding for v2 hardening, stale-Manager join policy (Nils's codex, 2026-05-17), and DHT-backed cluster doc as long-term direction. Net: 161 → 48 lines.

### Nils's codex · May 18, HKT, 2026

**Manager handoff now waits for heartbeat timeout and gossips the new Manager identity.** The liveness handler no longer treats raw libp2p `Disconnected` or `HeartbeatStreamClosed` carrier events as immediate semantic peer death. Those events now leave the last heartbeat timestamp intact and let the `HEARTBEAT_TIMEOUT` scan decide, so transient carrier churn has a chance to reconnect before causing peer eviction or Manager promotion.

Membership gossip now applies the advertised `manager_peer_id` from `/auki/membership/0.0.1` when the sender is that Manager and the Manager exists in the membership snapshot. This gives post-handoff broadcasts an explicit convergence signal for peers that did not independently choose the same winner at exactly the same time.

Tests: `cargo check -p auki-domain`, `cargo test -p auki-domain cluster_manager::tests -- --nocapture`, plus serial live Discovery runs for `manager_failover_when_a_dies_b_takes_over`, `manager_graceful_shutdown_passes_cluster_to_surviving_peer`, `three_peer_membership_converges_via_gossip`, and `manager_failover_when_manager_dies_before_first_heartbeat` against `http://192.168.9.130:8080`.

### Nils's codex · May 17, HKT, 2026

**`ClusterManager` now serves and fetches `/auki/resources/0.0.1` catalogs.** The domain layer starts the live resource catalog by auto-lifting the registered `SensorCatalogProvider` into `sensor_stream` resource rows and adding a new `ResourceCatalogProvider` trait for producer-owned rows such as rigid `transform_edge`. Provider rows override auto-lifted rows by `(kind, id)`, so applications can replace the SDK's default sensor-stream projection when they need a richer declaration.

New public surface: `set_resource_catalog_provider`, `fetch_resources_catalog`, and `fetch_resources_catalog_with`, plus re-exports for `ResourceEntry`, `ResourceKind`, `ResourcesRequest`, `ResourcesResponse`, `SensorStreamResource`, `TransformEdgeResource`, `ResourcePinholeIntrinsics`, and the JSON-friendly transform structs. When a requester asks for embedded registry details and the producer has called `set_registry_app_root(app_root)`, the resources handler attaches canonical Sensor / Frame Registry JSON after the same hash checks used by the exact `/auki/registries/0.0.1` path. Transform edges can request both endpoint frame entries, while camera stream rows can carry current `fx/fy/cx/cy`, which is the live discovery piece Park needs before composing `camera_link -> head_left_cam_optical` style frame edges locally.

Tests: `cargo check -p auki-domain`, `cargo test -p auki-domain`.

### Nils's codex · May 17, HKT, 2026

**ClusterManager now owns the heartbeat brain.** Manager-star topology, watched-peer calculation, first-frame timeout seeding, last-heartbeat timestamps, and semantic loss handling moved out of `auki-network` and into the domain liveness handler. `ClusterManager` computes outbound heartbeat targets from `(local_peer_id, manager_peer_id, membership)` and pushes them to `NetworkRuntime::set_heartbeat_targets`; the libp2p runtime only carries frames and reports raw carrier events.

The liveness handler now scans the domain-owned watchlist every `HEARTBEAT_TIMEOUT / 2`. Non-Managers watch the current Manager even before a first heartbeat frame arrives; Managers watch current members and evict dead peers. Carrier disconnect/closure and timeout paths both funnel through the same election/eviction code, preserving the failover behavior while making it transport-agnostic enough for a future Zenoh binding.

Tests: `cargo check -p auki-domain`, `cargo test -p auki-domain`, and a serial live run of `cargo test -p auki-domain --test cluster_manager_integration manager_failover -- --ignored --nocapture --test-threads=1` against `http://192.168.9.130:8080`.

### Nils's codex · May 17, HKT, 2026

**ClusterManager now steers peer heartbeats by Manager identity.** `create_cluster` sets the local peer as the heartbeat Manager, `join_cluster` sets the discovered Manager after the membership snapshot lands, and the liveness handler updates the runtime again after elections. The election rule is unchanged; the runtime now receives the role information it needs to make Manager death observable in both peer-id orderings.

Regression coverage: the QUIC failover integration tests now assert their peer-id ordering fixtures explicitly, and a new ignored live test `manager_failover_when_manager_dies_before_first_heartbeat` drops the Manager immediately after admitting the joiner to pin the no-first-heartbeat timeout path.

Filed a follow-up parking-lot question for the stricter stale-Discovery case where the Manager is already dead before any join response can provide a membership snapshot; that cannot safely reuse the heartbeat election path without a policy decision.

Tests: `cargo check -p auki-domain`, `cargo test -p auki-domain`, and focused live Discovery runs for `manager_failover_when_a_dies_b_takes_over`, `manager_failover_over_quic_when_manager_pid_lower`, `manager_failover_over_quic_when_joiner_pid_lower`, and `manager_failover_when_manager_dies_before_first_heartbeat` against `http://192.168.9.130:8080`.

### Nils's codex · May 16, 21:07 HKT, 2026

**Sensor catalogs can opt into embedded registry details.** `SensorCatalogProvider` keeps its existing `snapshot()` contract and gains a default `snapshot_for_request(...)` enrichment path. When the requester sends a detail `SensorsRequest` and the producer has called `set_registry_app_root(app_root)`, `ClusterManager` reads the exact local `SensorRegistryEntry` for each catalog row and optionally the referenced `FrameRegistryEntry` for spatial sensors, verifies each canonical hash against the advertised hash, and embeds the canonical JSON on the returned `SensorEntry`.

Consumers get `ClusterManager::fetch_sensors_catalog_with(peer_id, request)` for the detail path while `fetch_sensors_catalog(peer_id)` remains catalog-only. The existing hash-addressed `/auki/registries/0.0.1` fetch helpers remain the authoritative exact-entry path; embedded details are an opt-in round-trip reduction for Park-style sensor mounting.

Tests: unit coverage for sensor+frame enrichment from a local registry app root, plus the existing live catalog integration updated for the expanded `SensorEntry` shape.

### Arshak's claude · May 16, HKT, 2026

**ClusterManager serves and fetches Detector Registry entries over `/auki/registries/0.0.1`.** `read_registry_envelope` gains the `RegistryKind::Detector` arm — `set_registry_app_root(app_root)`'s auto-serve handler now responds to `(kind=Detector, id, hash)` requests by reading `<app_root>/registries/detectors/<id>/<hash>.json`. New `envelope_for_detector` mirrors the sensor/clock/frame helpers; new `ClusterManager::fetch_detector_entry(peer_id, detector_id, detector_hash)` mirrors `fetch_sensor_entry` / `fetch_frame_entry`.

**Why this matters for Cuba.** Park previously planned to enumerate detectors via an axum HTTP shim on each detector daemon (Cuba T6 / T7). With this commit the SDK's libp2p `/auki/registries/0.0.1` covers the same job — the daemon doesn't open a port, and Park doesn't speak HTTP to it. The HTTP shim is being deleted on the `detector-aruco-v0.0.45` rewrite.

**Tests**: +1 — `registry_envelope_reads_canonical_detector_from_app_root`. The new test independently hashes the entry and asserts it matches the envelope hash, locking the symmetry with the per-entry `hash()` method in `auki-registry`.

**Context**: Commit 4/6 of the Cuba v0.0.45 SDK migration. Sibling protocol-enum change lives in `auki-network`'s changelog on the same date.

### Nils's codex · May 16, 17:53 HKT, 2026

**Producer-side `StreamManifestBuilder` added.** `StreamManifestBuilder::from_registry(app_root, sensor_id, sensor_hash, clock_id, clock_hash)` reads the exact local `SensorRegistryEntry`, builds the stream accept `StreamManifest`, and centralizes the producer-side frame metadata projection that Park surfaced during live point-cloud testing.

For `RgbCamera` and `PointCloud` bodies, the builder copies `frame_id` and `frame_hash` from the sensor entry and verifies the exact `FrameRegistryEntry` exists via `read_frame(app_root, frame_id, frame_hash)`. For `Audio` and `JointEncoders`, it emits empty frame fields by contract. It never scans registry directories or guesses a hash.

**Typed loud failures:** `BuildStreamManifestError::{SensorEntryMissing, FrameIdMissing, FrameHashMissing, FrameEntryMissing, Io, Registry}` distinguish missing sensor metadata, malformed spatial sensor entries, missing frame entries, and lower-level registry failures. Producer stream providers can now reject/decline at accept time instead of returning half-populated manifests.

**Tests:** happy spatial path, non-spatial audio path, missing sensor, missing `frame_id`, missing `frame_hash`, missing frame entry, and registry JSON error coverage. Focused run: `cargo test -p auki-registry -p auki-domain -p auki-ros-adapter`.

### Nils's codex · May 16, 13:28 HKT, 2026

**`ClusterManager` wires registry exchange and typed fetch helpers.** Producers register their app root with `set_registry_app_root(app_root)`. The SDK's inbound `/auki/registries/0.0.1` handler reads existing `auki-registry` entries from that app root and replies with canonical JSON envelopes, or `entry: None` when the exact `(kind, id, hash)` is missing.

Consumers get `fetch_sensor_entry(peer_id, sensor_id, sensor_hash)`, `fetch_clock_entry(peer_id, clock_id, clock_hash)`, and `fetch_frame_entry(peer_id, frame_id, frame_hash)`. Each helper calls `NetworkRuntime::request_registry_entry`, verifies the returned envelope's kind/id/hash, hashes `canonical_json.as_bytes()` before decoding, then checks the decoded typed entry id. `FetchRegistryEntryError` separates request failures, not-found, invalid envelopes, hash mismatches, invalid JSON, and stopped managers.

Tests: unit coverage for app-root frame envelope serving and hash-mismatch rejection; ignored live integration test `cluster_peers_fetch_frame_registry_entry_over_libp2p` verifies a Park-like peer fetching a Booster-like peer's `FrameRegistryEntry` over libp2p.

### Nils's codex · May 15, 11:40 HKT, 2026

**Documentation refresh: `auki-domain` READMEs and sprint now match `ClusterManager` as shipped.** The crate docs now present `ClusterManager` as the single app-facing owner for Discovery, cluster bootstrap, membership, Manager state, liveness checks, handoff, peer info, sensor catalogs, and typed stream access. The obsolete Greenland `DomainIdentity` / `init_domain` framing is demoted to stale-history context in the sprint, while the README and implementation map describe the current `ClusterTarget`, `ClusterMembership`, `DaemonInfo`, and public methods. No Rust behavior changed.

### Nils's claude · May 15, 10:38 HKT, 2026

**SDK-fronted Discovery: `ClusterManager` becomes the single entry point for cluster lifecycle.** Hagall constraint #5 enforcement — "the SDK should handle as much as possible of the daemon-side networking, so that Booster and Park work the same way." Park and Boosterapp had divergent app-level Discovery-talking (Park wrapped `DiscoveryClient` in `Park::list_clusters`; Boosterapp's `_pick_cluster_target` ran `discovery.list_clusters()` then decided create-vs-join in Python). This PR pulls the decision logic into the SDK so both apps converge on one surface.

**New API:**
- [`ClusterManager::list_clusters(discovery_url) -> Vec<ClusterEntry>`](src/cluster_manager.rs) static. Apps no longer construct `DiscoveryClient` to read the directory.
- [`ClusterTarget`](src/cluster_manager.rs) enum: `Create { name }`, `Join { name }`, `JoinOrCreate { name }`, `MostRecentOrCreate { fallback_name }` — captures every cluster-bootstrap decision shape app daemons have historically needed. Static constructors (`ClusterTarget::create("foo")`, etc.) for ergonomics.
- [`ClusterManager::bootstrap(target, ...) -> Result<Self, BootstrapError>`](src/cluster_manager.rs) static. **Single entry point for headless daemons.** Internally lists Discovery, decides, dispatches to `create_cluster` / `join_cluster`. Race-tolerant for `JoinOrCreate` (surfaces `AlreadyExists` on lost race; app re-bootstraps if it wants to retry).
- [`BootstrapError`](src/cluster_manager.rs) aggregates `CreateClusterError` + `JoinClusterError` failure modes.

**Signature change** (breaking; consumers must update in lockstep — the matching Park + Boosterapp PRs are the migration):
- `ClusterManager::create_cluster` and `::join_cluster` now take `discovery_url: impl Into<String>` instead of a pre-built `DiscoveryClient`. The SDK constructs the client internally. Apps don't need to import or instantiate `DiscoveryClient` at all in the happy path. (`DiscoveryClient` stays `pub` in `auki-network` for now — demotion to `pub(crate)` is a follow-up after live confirms the migration.)

**Re-exports** for app-import ergonomics: `auki_domain::{ClusterTarget, BootstrapError, DiscoveryClusterEntry, DiscoveryClientError}`. Apps stay scoped to `auki_domain::*` imports.

**Tests**: `cargo test --workspace --lib` clean (no behaviour change in existing paths — only signature changes + new methods). `auki-domain` 31 unit tests pass. Live integration roundtrip verified against deployed Discovery at `192.168.9.130:8080` (`DISCOVERY_URL=... cargo test -p auki-network --features discovery_client --test discovery_integration -- --ignored` passes).

**Atomic merge:** Park PR (`Park::list_clusters` rewires through `ClusterManager::list_clusters`; `create_cluster` / `join_cluster` callers pass `discovery_url` strings) + Boosterapp PR (`_pick_cluster_target` deleted; `maybe_spawn_cluster_manager` collapses to one `auki_domain.ClusterManager.bootstrap(ClusterTarget.most_recent_or_create("hagall"), ...)` call) must merge in lockstep. Boosterapp K1 daemons + Park redeploy against v0.0.41 candidate.

### Nils's claude · May 15, 09:02 HKT, 2026

**Consumer-side rename for the Hagall `/heartbeat` → `/liveness` wire break.** Companion to [`auki-network` changelog 2026-05-15 09:02](../auki-network/changelog.md) which renames `DiscoveryClient::heartbeat` → `liveness_check`. This crate owns the cadence + the spawned background task that pushes the liveness check; both get renamed in lockstep.

**Symbol renames** (no behavioural change beyond cadence):
- Public const: `MANAGER_HEARTBEAT_INTERVAL: Duration = 3s` → `LIVENESS_CHECK_INTERVAL: Duration = 1s`. Retune from 3s to 1s matches the Hagall doc's "1s cadence + 3s sweep + 3 missed checks" convention; Discovery's sweep tightens 10s → 3s in the sibling Discovery PR.
- Function: `spawn_manager_heartbeat` → `spawn_manager_liveness_check`. Body unchanged beyond the inner `discovery.heartbeat()` call site (now `discovery.liveness_check()`) and the doc-comment retune.
- Struct field on `ClusterManager`: `heartbeat_task: Arc<Mutex<Option<JoinHandle<()>>>>` → `liveness_check_task` (internal, but renaming for symbol-consistency with the public surface — a future reader scanning the SDK shouldn't see a stale `heartbeat_task` name pointing at a liveness-check loop).
- Test: `manager_heartbeat_interval_matches_v1_contract` → `liveness_check_interval_matches_v1_contract`; assertion flips from `Duration::from_secs(3)` to `Duration::from_secs(1)`.
- Public re-export in [`lib.rs`](src/lib.rs): `MANAGER_HEARTBEAT_INTERVAL` → `LIVENESS_CHECK_INTERVAL`.

**Wire break by design** — no compatibility shim. Consumers pinning this crate must move atomically with the [aukilabs/discovery](https://github.com/aukilabs/discovery) sibling PR + the deployment redeploy.

**Tests**: workspace `cargo test --workspace --lib` clean (auki-domain 31 unit tests pass). Live integration roundtrip verified against a local Discovery built from the sibling branch — see the [`auki-network`](../auki-network/changelog.md) entry for the e2e roundtrip details.

### Nils's claude · May 13, 18:13 HKT, 2026

**Bug fix: graceful Manager shutdown no longer deregisters the cluster when other peers exist.** Surfaced by Nils: "manager leaving cluster causes cluster to close, no new manager is elected." Root cause: `ClusterManager::shutdown` unconditionally called `discovery.deregister(...)` when `was_manager`. With multiple peers, the surviving peer's libp2p ConnectionClosed → election → `discovery.rotate_manager(...)` round-trip 404'd because A's shutdown had already nuked the cluster from Discovery's directory. The cluster ended up dead instead of handing off.

Per Hagall design (quest doc): "Graceful and ungraceful Manager exits are the same code path — peers detect the loss + run the election + rotate." Fix: `shutdown()` only deregisters when the local peer is the LAST member (`membership.peers.len() <= 1`). With other peers around, the surviving members run their election + `rotate_manager`, which keeps the cluster name alive in Discovery and rotates the Manager hint to the new winner.

New regression test `manager_graceful_shutdown_passes_cluster_to_surviving_peer`: A creates, B joins, A calls `shutdown()` gracefully (NOT `drop`), B must take over + Discovery's directory must still hold the cluster with `manager_peer_id = pid_b`.

Two existing tests adjusted for the new shutdown semantics:
- `cluster_manager_full_lifecycle_against_live_discovery` — admits a fake peer (no real libp2p connection means no Lost event ever fires, so the fake peer stays in membership). `shutdown` now correctly skips deregister; added explicit `deregister` for test cleanup.
- `two_managers_create_then_join_against_live_discovery` — added 500ms sleep between B.shutdown and A.shutdown so A's liveness handler has time to evict B from membership before A's own shutdown checks `peers.len()`.

All 8 live integration tests pass against `192.168.9.130:8080`.

### Nils's claude · May 13, 17:21 HKT, 2026

**Fix: `ClusterManager::shutdown` is now `&self`; closes the "ghost cluster on Discovery" leak.** The Park bug Nils caught in today's demo — `nils` cluster left in Discovery heartbeating with `peer_count=1` for 10+ minutes after Park visibly created `nils2` and switched contexts — traced to `shutdown(mut self)` consuming the handle. Park's stream consumers hold `Arc<ClusterManager>` clones (the boosterapp-clone-fan-out + tile-consumer pattern), so Park could not call shutdown without first reaching unique ownership of the Arc. The "leave cluster" path was just dropping its own Arc reference, letting the Manager-side Discovery heartbeat tick keep running from whichever cloned Arc was still alive in a stream-provider closure — Discovery's 10s sweep never fired because the cluster kept getting refreshed.

**Option B fix:** `shutdown` signature changes from `pub async fn shutdown(mut self) -> Result<...>` to `pub async fn shutdown(&self) -> Result<...>`. Idempotent: an `AtomicBool stopped` flag is `swap`'d at the top — the first caller proceeds with task aborts + Discovery `DELETE /clusters/{name}` + runtime shutdown; concurrent / repeat callers observe `true` and short-circuit with `Ok(())`. The five bare `Option<JoinHandle<()>>` task fields (`join_handler_task`, `liveness_handler_task`, `membership_handler_task`, `info_handler_task`, `sensors_handler_task`) move to `Mutex<Option<JoinHandle<()>>>` so the `.take()` pattern works under a shared reference. `heartbeat_task` was already `Arc<Mutex<Option<_>>>` from SDK-T7.

**`AdmitError::Stopped` + `FetchParticipantInfoError::Stopped` new variants.** Pub I/O methods (`admit_peer`, `fetch_participant_info`) check the `stopped` flag at entry and fast-fail with the typed variant — callers holding stale `Arc<ClusterManager>` clones after shutdown see a clean signal rather than the cascading runtime-channel-closed / libp2p-substream-failed errors they'd otherwise get. Snapshot accessors (`membership`, `peer_count`, `is_manager`, `participant_info`, …) are intentionally not gated; returning the last-observed state is harmless and lets consumers drain their final view before dropping their Arc.

**Open-stream is NOT gated** by the stopped flag — `OpenStreamError` lives in `auki-network` and pulling a `Stopped` variant into it would pollute the runtime crate with a domain-layer concern. Callers see a libp2p substream / channel-closed error path instead, which is graceful enough for the consumer-drops-then-shutdown ordering Park uses.

**`NetworkRuntime::shutdown` parallel refactor** (in `auki-network`) — same shape change: `mut self → &self`, `shutdown_tx` + `task` move to `std::sync::Mutex<Option<_>>`, idempotent via `.take()`. Drop impl calls the same `cleanup()` path (still without the inbound grace signal — only explicit `shutdown()` triggers the `EndOfStream` flush window). No public-API breakage outside auki-domain.

**New live integration test** `shutdown_via_arc_clone_deregisters_and_remains_idempotent`: builds a manager, wraps it in `Arc`, makes two clones (`consumer_clone` simulating a Park stream consumer, `leftover_clone` simulating Park's daemon-side holder), drops the original handle, calls `shutdown()` through `consumer_clone`, verifies (1) Discovery `list_clusters` no longer contains the cluster, (2) a second `shutdown()` through `leftover_clone` returns `Ok(())` without re-DELETE, (3) post-shutdown `admit_peer` / `fetch_participant_info` calls through `leftover_clone` return `AdmitError::Stopped` / `FetchParticipantInfoError::Stopped`, (4) dropping the last Arc is a no-op. Live tests now pass against `192.168.9.130:8080`.

Park is unblocked: on the leave-cluster path it should call `.shutdown()` from any Arc clone (then drop all references) instead of relying on Drop semantics. Downstream Park PR pending — Park-side code lives outside this repo.

### Nils's claude · May 13, 15:30 HKT, 2026

**`/auki/sensors/0.0.1` wired into `ClusterManager`. Park's sensor-chip row unblocked.** Producers (Booster, future robotics SDK consumers) tell the SDK what sensors they're publishing via a new `SensorCatalogProvider` trait — `Arc<dyn SensorCatalogProvider>` installed once at construction (or swapped later) via `ClusterManager::set_sensor_catalog_provider`. Inbound `/auki/sensors/0.0.1` requests snapshot the registered provider and reply; if no provider is registered the SDK returns an empty `sensors: []` — "I have a catalog and it's empty" is a valid producer state, NOT an error. NO FALLBACK inside the SDK.

New `spawn_sensors_handler` task drains inbound `SensorsRequestEvent`s from the runtime, snapshots the application-supplied provider, serializes the catalog, replies via the event's oneshot. Lives for the lifetime of the ClusterManager; cancelled on `shutdown` alongside the existing five handler tasks.

New `ClusterManager::fetch_sensors_catalog(peer_id) -> Result<SensorsResponse, FetchSensorsCatalogError>` public method. Thin async wrapper over `NetworkRuntime::request_sensors_catalog`. Park calls this for every cluster peer to populate its sensor-chip row (one chip per `SensorEntry`).

Crate re-exports `SensorEntry`, `SensorsResponse`, `SensorCatalogProvider`, `FetchSensorsCatalogError` for consumers.

New live integration test `cluster_peers_fetch_each_other_sensors_catalog_over_libp2p` (one Booster-like B with a fixed one-camera catalog, one Park-like A with no provider) verifies round-trip + empty-catalog semantics. `#[ignore]`'d like every other live test — runs against `192.168.9.130:8080` when explicitly invoked.

### Nils's claude · May 13, 14:34 HKT, 2026

**`/auki/info/0.0.1` wired into `ClusterManager` + breaking refactor of `DaemonInfo`. NO FALLBACKS gap closed.** Park (and any future cross-daemon Auki UI) can now resolve a cluster peer's `ParticipantInfo` over libp2p instead of via mDNS + HTTP `/api/info`. This is the last Hagall-shape gap before Park can drop mDNS entirely (PARK-T7 + PARK-T9 unblocked).

**Breaking: `DaemonInfo` loses `session_now_ns` + `cluster_joined_at_ns`.** Those are dynamic — daemons couldn't keep them fresh without re-passing on every call. The SDK now owns both: `ClusterManager` stores `session_started: Instant` at construction and computes `session_now_ns = session_started.elapsed()` on each `participant_info()` call; `cluster_joined_at_ns: Arc<Mutex<Option<u64>>>` is set lazily on the first observation of any non-self peer in membership (per ansuz D3 — a one-peer cluster shouldn't tick this field).

**Breaking: `ClusterManager::create_cluster` / `.join_cluster` gain a `daemon_info: DaemonInfo` parameter** (now 7-arg). Stored on the manager; reused for every `participant_info()` build (local + inbound info-request responses).

**Breaking: `ClusterManager::participant_info(&self)` no longer takes a `DaemonInfo` arg.** Builds from stored state. Same `ParticipantInfo` wire shape.

New `spawn_info_handler` task drains inbound `InfoRequestEvent`s from the runtime, builds a fresh `ParticipantInfo` via the shared `build_participant_info` helper, serializes to JSON, replies via the event's oneshot. Lives for the lifetime of the ClusterManager; cancelled on `shutdown`.

New `ClusterManager::fetch_participant_info(peer_id) -> Result<ParticipantInfo, FetchParticipantInfoError>` public method. Thin async wrapper over `NetworkRuntime::request_participant_info` that parses the response JSON back into a `ParticipantInfo`. Park calls this for every cluster peer to populate `/api/cluster/peers` and (PARK-T7) to make the `#/` directory cluster-driven.

5 live integration tests now pass against `192.168.9.130:8080` in ~12s — the new one (`cluster_peers_fetch_each_other_participant_info_over_libp2p`) creates a "park-like" A + "boosterapp-like" B, then verifies A's `fetch_participant_info(pid_b)` returns B's full `ParticipantInfo` with `app: "boosterapp"`, `name: "walker-1"`, etc., and vice versa from B's side.

### Nils's claude · May 13, 13:48 HKT, 2026

**SDK-T2 PARTIAL → DONE — membership gossip wired into `ClusterManager`.** Three pieces:

- **`spawn_membership_handler`** task drains the new `MembershipEvent` channel returned from `NetworkRuntime::spawn`. On each event: parse the JSON, swap the local `ClusterMembership`, rebuild the allow-list, push via `NetworkRuntimeHandle::set_allowed_peers`. Deliberately does NOT mutate `manager_peer_id` — the election in `spawn_liveness_handler` is the single source of truth for Manager identity, and trusting gossip-sender-as-Manager would let a non-Manager cluster member fake the role.
- **`broadcast_current_membership`** helper. Serializes the locked `ClusterMembership` and calls `NetworkRuntimeHandle::broadcast_membership`. Called in three places: (a) `spawn_join_handler` after a successful admit (so existing peers learn about the new joiner — the joiner itself got the same JSON in `JoinResponse::Accept`); (b) `spawn_liveness_handler`'s Manager-promotion path after the new Manager evicts the dead one; (c) `spawn_liveness_handler`'s Manager-evicts-Lost-peer path. `ClusterManager::admit_peer` (manual public API) also broadcasts post-success for symmetry.
- **`ClusterManager` struct gains** `membership_handler_task: Option<JoinHandle<()>>`. Spawned by both `create_cluster` and `join_cluster`; cancelled on `shutdown` alongside the join + liveness + heartbeat tasks.

New 3-peer convergence integration test against the live Discovery (`192.168.9.130:8080`): A creates cluster `foo`, B joins, C joins → poll for B's `peer_count()` to reach 3 within 5s. Without gossip B would stay at 2 (its admit-time snapshot). With gossip B converges within ~600ms. Passes alongside the other three live tests (full-lifecycle, two-managers-join, manager-failover); 4 live tests now total, finishing in ~12s.

This unblocks **Hagall demo step 13** ("the Manager adds him to `foo.json` and propagates that to all peers in the cluster") which step 14 (Charlie-Park consumes Booster streams via the cluster-gated `/auki/stream/0.1.0`) depends on — without gossip, Booster #2's allow-list never includes Charlie-Park and Charlie-Park's stream substream is silently dropped at the gate.

### Nils's claude · May 13, 13:00 HKT, 2026

**SDK-T6 + SDK-T7 — cluster-internal election + Manager-handoff orchestration. Failover works end-to-end.** Three new pieces:

- **`elect_successor(membership, local_peer_id, connected) -> Option<PeerId>`** — pure function. Sorts membership by `(join_ts_ns, peer_id)` ascending; returns the earliest-joined peer that's "reachable" (in `connected` or equal to `local_peer_id`). 5 unit tests pin the rule (earliest-joined wins, unreachable earlier peers skipped, peer-id tie-break, local-alone-wins, empty-membership-returns-None).
- **`spawn_liveness_handler`** — task that drains `PeerLivenessEvent`s from the network runtime. On `Lost { peer_id: lost }`: if `lost` is the Manager and we're not, run the election; if we win, become Manager (update local state, call `discovery.rotate_manager`, spawn the Manager-side Discovery heartbeat tick, evict the dead Manager from membership + push the updated allow-list). If we're the Manager and a peer was lost, evict + push allow-list. Dedupe per disconnection.
- **`ClusterManager::heartbeat_task`** is now `Arc<Mutex<Option<JoinHandle<()>>>>` so the liveness handler can spawn it on Manager-promotion. `join_cluster` initializes it to `None` (joiner isn't Manager yet); `create_cluster` initializes it `Some` (creator is the initial Manager).

End-to-end live integration test (3rd in the file): A creates cluster `foo`; B `join_cluster`s it; A is `drop`'d without `shutdown` (unclean exit, simulates a process kill); within 5s, B detects loss via the heartbeat-timeout monitor, runs the election (B is the only reachable peer, so B wins), promotes itself, calls `discovery.rotate_manager`, and starts the Manager heartbeat tick. The test verifies B's `is_manager == true`, `manager_peer_id == B`, AND that Discovery's directory snapshot has rotated to B. All three live integration tests pass against `192.168.9.130:8080`: full-lifecycle, two-managers-join, and the new failover scenario.

`elect_successor` re-exported from `auki-domain`'s public surface for downstream testing.

### Nils's claude · May 13, 12:30 HKT, 2026

### Nils's claude · May 13, 12:30 HKT, 2026

**SDK-T3 — `ClusterManager::join_cluster` ships; Manager-side join handler task admits + gossips membership.** Two new pieces wire `auki-network`'s `/auki/join/0.0.1` protocol into a usable end-to-end flow:

- **`ClusterManager::join_cluster(name, identity, multiaddrs, discovery, swarm, stream_provider)`** — looks the cluster up in Discovery, spawns the runtime with the Manager pre-allowed for dial, waits for the libp2p connection to establish (up to 10s), opens a `/auki/join/0.0.1` substream, sends the `JoinRequest`, parses the Manager's `JoinResponse::Accept { membership_json, successor_token }`, expands the runtime's allow-list to cover every peer in the Manager-gossiped membership, returns a `ClusterManager` with `is_manager = false` and `manager_peer_id` pointing at the Manager. On `Reject` returns `JoinClusterError::Rejected(reason)`.
- **Manager-side join handler task** (`spawn_join_handler`) — drains the `JoinEvent` channel returned by `NetworkRuntime::spawn`, decides admit-or-reject. As Manager: appends the new peer to the membership, builds the updated allow-list, pushes it via `NetworkRuntimeHandle::set_allowed_peers`, replies with `Accept { membership_json, successor_token }`. As non-Manager: always replies with `Reject { reason: "not the manager" }`. Duplicate admits return `Reject { reason: "already a member" }`. The task is spawned by both `create_cluster` and `join_cluster`; cancelled on `shutdown`.

Live integration test (2-peer): peer A `create_cluster`s; peer B `join_cluster`s the same cluster; both verify identical membership (Manager + joiner); peer B sees A as its Manager; both shutdown cleanly and Discovery's entry is gone. Passes against the running deployment at `192.168.9.130:8080`.

Python: `auki_domain.ClusterManager.join_cluster(wallet_seed, cluster_name, discovery_url, listen_addresses, agent_version)` mirrors the Rust API. Same daemon-friendly façade as `create_cluster`. Booster / Park can now `import auki_domain; mgr = auki_domain.ClusterManager.join_cluster(...)` and get a fully-populated handle ready for `/api/info`.

Deps: `serde_json` promoted from dev-dep to runtime dep (the Manager-side handler serializes the membership; the join-side parses the Manager-gossiped membership). New error type `JoinClusterError` with 6 variants: `Discovery`, `NotFound(name)`, `SendJoin(SendJoinRequestError)`, `Rejected(reason)`, `InvalidMembership(serde_json::Error)`, `Runtime(SpawnError)`.

### Nils's claude · May 13, 11:45 HKT, 2026

**SDK-T2 lands — `ClusterManager` ships with `create_cluster` + `admit_peer` + `participant_info` + Manager-side Discovery heartbeat.** New `cluster_manager` module. `ClusterManager` owns the cluster's `ClusterMembership`, the libp2p `NetworkRuntime` (from `auki-network`), the `DiscoveryClient`, and a Manager-side Discovery heartbeat tick (3s cadence — matches the v1 contract's 10s sweep). Surface:

- `ClusterManager::create_cluster(name, identity, multiaddrs, discovery, swarm, stream_provider)` — atomic create on Discovery, initialize membership with self as the sole member, spawn the runtime, spawn the heartbeat tick. Returns the handle.
- Accessors: `cluster_name`, `local_peer_id`, `is_manager`, `manager_peer_id`, `membership`, `peer_count`.
- `admit_peer(peer_id, multiaddrs) -> ClusterMember` — Manager-only; appends to membership, pushes the updated allow-list to the runtime. Duplicate admit returns `AdmitError::AlreadyMember`. Non-Manager admit returns `AdmitError::NotManager { cluster, manager }`. v1 successor token is empty bytes (signature verification disabled per Discovery v1 contract); SDK-T4 swaps in a real signed token.
- `participant_info(daemon_info) -> ParticipantInfo` — builds the `/api/info` JSON shape with cluster-aware fields (`is_manager`, `manager_peer_id`, `peer_id`) populated by the SDK; daemon supplies its own identity fields via `DaemonInfo`. Per BA-Q3.
- `shutdown(self)` — cancels the heartbeat tick, deregisters from Discovery (if we're the Manager), shuts down the runtime.

End-to-end live integration test against the running Discovery at `192.168.9.130:8080` verifies the full lifecycle: create → accessors → participant_info shape → admit_peer + duplicate rejection → heartbeat keeps Discovery's entry alive past the 10s sweep window (test waits 12s and asserts the cluster is still there) → shutdown deregisters cleanly. Two unit tests + 1 integration test (ignored by default).

**Not in this commit** (deferred to follow-up PRs): the libp2p join protocol (SDK-T3 — needed for `join_cluster`), peer-side heartbeat (SDK-T5), cluster-internal election (SDK-T6), Manager-handoff orchestration (SDK-T7), signed successor tokens (SDK-T4, blocked on SDK-Q3), anti-entropy / reconciliation / last-writer-wins (SDK-Q5 deeper convergence). `join_cluster` as a method is not yet exposed — only `create_cluster` works end-to-end.

New deps: `tokio` (was dev-only) elevated to a runtime dep for the heartbeat tick (`rt`/`time`/`macros` features); `libp2p` + `futures` added to dev-deps for the integration test (waits for the swarm's OS-chosen listen port before construction).

### Nils's claude · May 13, 09:45 HKT, 2026

**Hagall SDK-T1 — `ClusterMembership` type + serde lands.** New `cluster_membership` module: `ClusterMembership { cluster_name, peers: Vec<ClusterMember> }` carries the cluster's authoritative membership document. `ClusterMember` fields = `peer_id: PeerId`, `multiaddrs: Vec<Multiaddr>`, `join_ts_ns: i64`, `successor_token: Option<Vec<u8>>` (opaque per SDK-Q3 still being open; v1 Discovery contract skips signature verification entirely, so empty bytes are fine for the demo). serde JSON with the same Multiaddr-as-string adapter as `auki-network`'s `ClusterDoc`. `ClusterMembership::filename()` returns `<cluster_name>.json` — the wire/disk filename per Hagall convention; the `foo.json` of the Hagall demo cluster is exactly that, no special-casing. `ClusterMembership::admit(member)` appends in admission order and returns the index. 9 unit tests cover round-trip (with peers, empty cluster, member-without-token, empty-multiaddrs), peer-order preservation, filename derivation, and a wire-shape-locked test that pins JSON key names against rename. 1 new doctest. **Greenland's `ClusterDoc` is untouched** — per SDK-Q1's resolution (replace), the deletion of `init_domain` / `init_or_join_domain` / Greenland-era types lands in a follow-up breaking PR once Hagall is functional end-to-end. New deps: `libp2p-identity` (with `serde` feature for canonical `PeerId` strings), `serde` (derive), and `serde_json` (dev). All 21 existing tests still pass; 0 new clippy warnings.

### broodsugar's claude · May 13, HKT, 2026

**`init_or_join_domain` added — race-loss collapsed into the happy path.** Sibling to `init_domain`; same arg shape, different semantics. `init_domain` returns `Err(AlreadyExists)` when Discovery's atomic `create_cluster` 409s — the caller learns Manager-vs-joiner role and can branch. `init_or_join_domain` collapses both outcomes into a "I just want into this Domain" success path: whichever peer wins `create_cluster`, the caller registers against the resulting cluster and builds the runtime exactly once. The swarm is consumed exactly once regardless of which `CreateClusterOutcome` variant fires, so there's no race window in which the swarm would need rebuilding.

Targeted at producer-only daemons (BoosterApp, Sentinel) that don't care about Manager identity today — Greenland's Manager-role state (T2+T3+T4+T6+T7) is still stubbed, so the create-vs-join distinction doesn't affect functional behaviour. Daemons that need the discrimination later (failover trigger, JoinRequest admission, etc.) continue calling `init_domain` and branching on `AlreadyExists`. Two public entry points cleanly separate the use-cases.

Implementation is a thin variant of `init_domain` — same DomainIdentity derivation, same register + from_swarm sequence; only `create_cluster`'s `Outcome::AlreadyExists` branch is `_ = ...`'d instead of returning `Err`. No new error variant; the same `InitDomainError::{Discovery, RuntimeSpawn}` cases apply (the third — `AlreadyExists` — is unreachable from this function by construction). ~40 LOC added; existing `init_domain` unchanged.

### broodsugar's claude · May 13, 11:21 HKT, 2026

**`init_domain` becomes the canonical (and only sanctioned) public `ClusterRuntime` constructor.** Pairs with the [`auki-network` PR B](../auki-network/changelog.md) that killed `cluster.json` and made `ClusterRuntime::from_swarm` `#[doc(hidden)] pub`. Together they close every bypass: peers only visible within their cluster, no fallback, no Discovery-less path.

**Signature change.** `init_domain` now takes `swarm: Swarm<Behaviour>`, `participant_provider: ParticipantInfoProvider`, `stream_provider: StreamProvider` in addition to the previous args, and returns `DomainHandle { identity: DomainIdentity, runtime: ClusterRuntime }` with both fields public. The `ClusterDoc` Discovery's `register` returns never leaves the SDK — it goes straight into `ClusterRuntime::from_swarm`, whose `apply_initial_doc` step populates the libp2p allow-list before the event loop starts. Park's bypass-init_domain shortcut (which existed because the old `init_domain` discarded the `ClusterDoc`) becomes unnecessary.

**`DomainHandle` now owns the runtime.** Was a thin wrapper around `DomainIdentity` only; now `pub struct DomainHandle { pub identity: DomainIdentity, pub runtime: ClusterRuntime }`. Daemons feed `discovery.subscribe(&cluster_name)` events into `handle.runtime.update_cluster_doc(new_doc)` themselves — the runtime doesn't yet own its SSE subscription (filed in `auki-network/parking_lot.md` as a tightening follow-up).

**`InitDomainError::RuntimeSpawn(SpawnError)`** variant added for the post-register failure path. Discovery-side calls (`create_cluster`, `register`) have already succeeded by the time the runtime is constructed, so a `RuntimeSpawn` failure means the cluster is created and the peer is registered but the runtime didn't construct — caller may need to deregister before retrying.

**`Cargo.toml` dep change.** `auki-network` feature set gains `"swarm"` (previously had `"discovery_client"` only). Needed to name `Swarm<Behaviour>` and pass it to `ClusterRuntime::from_swarm`.

**Tests** — 12 unit tests + 1 doctest pass with `--all-features`. (The old `init_domain` doctest's signature changed; the unchanged tests cover `DomainIdentity` shape.)

**Daemon-side migration:** Park / BoosterApp / Sentinel daemons each need a per-repo PR to migrate from the old `init_domain(wallet, name, discovery, addresses, ...)` to the new `init_domain(wallet, name, discovery, swarm, addresses, ..., participant_provider, stream_provider)`. Park's PR #36 bypass path (which called `DiscoveryClient::register` directly) goes away — `init_domain` now does everything Park needs.

### broodsugar's claude · May 11, 17:46 HKT, 2026

**`init_domain` now creates the cluster before registering — `InitDomainError::AlreadyExists` variant landed.** Pairs with the [`auki-network::DiscoveryClient::create_cluster`](../auki-network/changelog.md) addition shipped in the same PR. Closes the rollout-breakage the SDK agent surfaced: Discovery's new T8 deployment ([aukilabs/discovery#2](https://github.com/aukilabs/discovery/pull/2) merged) removed lazy-create on `POST /clusters/{name}/peers`, so any SDK consumer that called `register` against a fresh cluster would 404 against the new Discovery.

**Call sequence inside `init_domain`** is now:

1. Build `DomainIdentity` from `wallet` + `name` (Vinland singleton handled as before).
2. `DiscoveryClient::create_cluster(wallet, &cluster_name)` — signed JCS over `{ cluster_name, op: "create", peer_id, public_key, timestamp_ns }`. The signer is recorded by Discovery as the initial Manager (`ClusterDoc.current_manager_peer_id`).
3. On `CreateClusterOutcome::Created` — fall through to step 4.
4. On `CreateClusterOutcome::AlreadyExists { existing }` — return `InitDomainError::AlreadyExists { identity, existing }` so the caller can route to the Vinland-race fall-back-to-join branch in Greenland T12.
5. `DiscoveryClient::register` — unchanged.

**`InitDomainError::AlreadyExists`** carries the local caller's `DomainIdentity` plus the winner's full `ClusterDoc` (parsed from Discovery's `{ error: "already_exists", existing: ClusterDoc }` 409 body). The winner's `current_manager_peer_id` is the live Manager the loser should route a `JoinRequest` at once Greenland T5 lands. Lifts the variant out of the PR 3 deferral noted in the prior schema PR — Discovery now produces a real 409 to map.

**Glossary / Notion alignment.** The Notion T1 description called for `existing: DomainIdentity` on the variant; this PR ships `existing: ClusterDoc` instead — strictly more information (the cluster doc carries the winning Manager peer-id + creation timestamp + peer list), the identity is recoverable from `existing.cluster_name`. Caller-side ergonomics win.

No new `auki-domain` tests beyond what the existing 12 cover; the wire-shape and signing tests live in `auki-network::discovery_client` (6 new tests there). `cargo test -p auki-domain --all-features` 12/12 + 2 doctests green.

### broodsugar's dobby · May 11, 14:48 HKT, 2026

**Greenland design corrections — T7 inverted to libp2p, T8 endpoint name, T14 added.** Three updates landed on the [Greenland Notion page](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2) earlier today; this doc-only PR transcribes them into the local parking-lot and sprint plan ahead of PR 2 (heartbeat batch).

- **T7 (broadcast envelope) — Discovery is OUT of the live-registry fan-out path.** Manager publishes full `ClusterDoc` snapshots **directly to cluster members over libp2p** via a new dedicated protocol `/auki/registry/0.0.1` (sibling to `/auki/heartbeat/0.0.1`). The earlier framing — "Manager publishes to Discovery, which fans out over the existing SSE channel from PR #84" — is inverted (Q-disc-1 resolution). Rationale: heartbeat is already peer-to-peer per T3; layering a parallel Discovery-pushed snapshot stream creates two live-state surfaces. Single libp2p surface is cleaner. PR 2's T7 wire change is now SDK-only — no Discovery SSE plumbing required.
- **T8 (Discovery endpoint for headless join) — `GET /clusters/latest`, not `/domains/latest`.** Q-disc-2 resolution: Domain is the topic the cluster forms around — `cluster_name` IS the Domain identifier. SDK code unchanged (already passes the canonical Domain Identity string as `cluster_name`); the `/domains/*` noun in the original task description was design-time vocabulary. Discovery is relaxing its `cluster_name` regex to accept `/` in its own PR. Plus a Discovery-side schema add `created_ns: i64` to `ClusterDoc` (server-side timestamp); SDK picks it up on the next `auki-network` tag bump.
- **T14 (new task) — Manager-handoff notification to Discovery on election.** Promoted from Q14, resolved same session. On Manager handoff (graceful T13 or crash-detected T10), the newly-elected Manager sends a signed JCS payload to Discovery (probably `POST /clusters/{name}/manager`) so late-joiners hitting `GET /clusters/latest` route their JoinRequests to the live Manager peer-id rather than the dead one. Signed by the new Manager's `peer/v1` derivation key. Discovery's `ClusterDoc` response grows `current_manager_peer_id: Option<PeerId>`. T14 slots into PR 3 (failover batch) alongside T10/T11/T13.

Files touched: [`crates/auki-domain/parking_lot.md`](parking_lot.md) (T7 + T8 + T12 decision-block rewrites; new T14 decision block); [`crates/auki-domain/src/sprint.md`](src/sprint.md) (PR 2 + PR 3 section updates; "Architectural decisions to honor" bullet list refreshed).

### broodsugar's dobby · May 11, 14:34 HKT, 2026

**Greenland T1 — `DomainIdentity` + `init_domain` shipped.** First implementing PR of the [Greenland quest](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2). New `DomainIdentity` value type carrying the wallet-scoped canonical string `{wallet_id}/{name}` with the reserved `"Vinland"` singleton exception (T12). `init_domain(&Wallet, &str, &DiscoveryClient, &[Multiaddr], Option<&str>, Option<&str>)` builds the identity, calls `DiscoveryClient::register`, returns a minimal `DomainHandle{identity}` (Manager-role state grows in PR 2). 12 unit tests + 2 doctests + 2 locked cross-language conformance vectors (user-named string structure, singleton string). Glossary updated: new `Domain Identity` entry alongside the existing `Domain ID`; `Domain ID` keeps its existing definition for TagClaims, `Domain Identity` is the network-topic / Discovery-indexing string. Resolves the Glossary-reconciliation parking-lot question filed in PR 0.

### broodsugar's dobby · May 11, 14:20 HKT, 2026

**Crate scaffolded — [`auki-domain`](../auki-domain).** Greenland PR 0 lays the home for SDK-side Domain lifecycle (creation, joining, Manager/Member roles, heartbeats, live Cluster Registry, failover). No functional code — empty `lib.rs`. Lands the folder convention (`Cargo.toml`, `README.md`, `parking_lot.md`, `changelog.md`, `src/readme.md`, `src/sprint.md`), workspace registration in root `Cargo.toml`, and parking-lot pre-files of every Greenland architectural decision transcribed from the [Notion Tasks table](https://www.notion.so/Greenland-35d5c8e9659280dbb8cff0d196f3c3d2). PR 1 (T1: `DomainIdentity` + `init_domain`) lands next, per [`src/sprint.md`](src/sprint.md).
