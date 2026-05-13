# Changelog — `auki-domain-py`

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · May 13, 10:30 HKT, 2026

**Rewritten end-to-end: now binds `ClusterMembership` + `ClusterMember`, full stop.** The previous surface (`init_domain` / `init_or_join_domain` / `DomainHandle` plus 5 typed Python exceptions) bound `auki-domain` Greenland entry points that no longer exist; deleted. New surface mirrors the Rust API one-to-one — `ClusterMembership(cluster_name)` + `.peers`, `.cluster_name`, `.filename`, `.admit(member)`, `.to_json()`, `ClusterMembership.from_json(s)`; `ClusterMember(peer_id, multiaddrs, join_ts_ns, successor_token=None)` value type. Strings on the boundary: `peer_id` is the canonical libp2p peer-id string, `multiaddrs` are the canonical `/ip4/.../tcp/...` text form, parsed at construction and re-stringified on read. ~1046 LOC → ~210 LOC. Dep stack simplified: dropped `auki-network`, `auki-identity`, `auki-network-py`, `tokio`, `tracing`, `tracing-subscriber` — the new surface is a pure-data binding, no async, no swarm, no Discovery client (consumers use `auki_network.DiscoveryClient` directly).

---

### broodsugar's claude · May 13, HKT, 2026 (Phase 2)

**`stream_provider` kwarg wired + `init_or_join_domain` becomes the underlying call.** Closes [`parking_lot.md`](parking_lot.md) #1 (stream_provider blocker). Three coordinated changes lift the entire Phase 2 surface in one cut:

1. **`auki-network-py`'s `[lib] name` renamed `auki_network` → `auki_network_py`.** Breaks the lib-name collision that blocked depending on `auki-network-py` from sibling PyO3 crates (cargo refuses two libs with the same `[lib] name` in one direct-dep set). Python module name stays `auki_network` — `[tool.maturin] module-name = "auki_network"` in the pyproject already instructs maturin to rename the cdylib output during wheel build, so consumers continue to `import auki_network` unchanged. Rust-side rename is transparent to `cargo test` (the rlib is what tests link against; the cdylib is what maturin packages). 1 stale `module_exposes_cluster_submodule_with_documented_surface` assertion (left over from v0.0.32 — expected `load_doc` / `spawn` on the cluster submodule) updated to assert their **removal** — the cluster-trust-boundary regression guard.

2. **`build_stream_provider` promoted from `pub(crate)` to `pub` in `auki-network-py`.** The Python-callable → Rust-`StreamProvider` adapter (the ~500-line PyStreamDecision / PyAcceptInfo / PyDeclineReason plumbing) is now reachable from sibling crates. `mod stream_types` → `pub mod stream_types` for the cross-crate seam.

3. **`auki-network-py` added as a renamed dep of `auki-domain-py`.** `default-features = false` so the `extension-module` feature doesn't transitively enable on the dep path (breaks `auto-initialize` test linkage). With the lib-name collision gone, this cargo dep edge "just works."

**`auki-domain-py` surface additions:**
- `init_domain(...)` gains a `stream_provider: callable | None = None` kwarg. When supplied, the callable is wrapped via `auki-network-py`'s `build_stream_provider` — same adapter `auki_network.cluster.spawn` used pre-v0.0.33, same `StreamDecision.accept(...)` / `decline(...)` factory contract. When omitted, defaults to `auki_network::stream_runtime::decline_all_streams()` — daemons that only consume streams (no producer side) don't need the kwarg.
- Switched underlying Rust call from `auki_domain::init_domain` to `auki_domain::init_or_join_domain` (newly added in the same PR; see `auki-domain/changelog.md`). Race-loss is now collapsed into the happy path — any peer joining an existing Domain succeeds instead of getting `AlreadyExists`. The Python `init_domain` entry point now means "do whatever it takes to get me into this Domain" rather than "create-or-fail." Daemons that need create-vs-join discrimination can layer on top once a typed Python `auki_domain.init_domain` (matching the Rust create-only function) is added — filed as Phase 3.

**1 new Rust test covers the cross-crate wiring**: `stream_provider_adapter_reachable_from_auki_domain_py` exercises that `build_stream_provider` is callable from this crate (link-time confirmation that the lib-name rename + `pub` promotion + cargo dep all line up). 12 Rust tests pass total (was 11 in Phase 1).

**Parking-lot updates:** #1 (stream_provider) RESOLVED → Propagate. #4 (`DomainAlreadyExists.existing` full ClusterDoc) collapses with the switch to `init_or_join_domain` — the daemon never sees `AlreadyExists` at the Python boundary anymore; the underlying race-loss is handled internally. #2 (consumer-side `open_*_stream()`) and #3 (SSE-driven `update_cluster_doc`) remain open but are unblocked technically — the lib-name fix has removed the cross-crate barrier; they're just unimplemented because no consumer needs them yet (Park is Rust-side; BoosterApp's small static demo cluster tolerates the missing SSE feed).

### broodsugar's claude · May 13, HKT, 2026

**Crate bootstrap — `init_domain` Python entry point ships (Phase 1).** Wraps [`auki_domain::init_domain`](../auki-domain/src/lib.rs) so Python daemons (BoosterApp, Sentinel) can construct a `ClusterRuntime` through Discovery now that `auki-network-py`'s `cluster.spawn` is gone (cluster-trust-boundary PR B / v0.0.33). New crate following the per-component PyO3 wrapper convention (`auki-identity-py`, `auki-network-py`, `auki-domain-py`).

**Surface this PR ships:**
- `auki_domain.init_domain(wallet_seed, peer_seed, discovery_url, domain_name, addresses, participant_provider, *, listen_addresses=None, agent_version=None, expected_app_id=None, note=None) -> DomainHandle` — synchronous-blocking. Builds the libp2p swarm internally (via `auki_network::swarm::build_swarm`), drives the async `init_domain` on a process-wide tokio runtime, returns a `DomainHandle` whose internal `ClusterRuntime` was constructed by `from_swarm` with the libp2p allow-list pre-populated from `ClusterDoc.peers`.
- `auki_domain.DomainHandle` pyclass: `.identity` (string getter), `.peers()` (list of dicts; same shape as `auki_network.cluster.ClusterRuntime.peers()`), `.shutdown()` (consumes; subsequent calls raise).
- Typed Python exceptions: `DomainAlreadyExists` (carries `.identity` + `.cluster_name`), `DiscoveryUnreachable`, `DiscoveryRejected` (carries `.status` + `.body`), `DiscoveryClockError`, `RuntimeSpawnError`. Map from `InitDomainError::{AlreadyExists, Discovery, RuntimeSpawn}` and `DiscoveryError::{Transport, Status, ClockSkew, ...}`.
- `participant_provider` callable is wrapped via **duck-typed attribute access** (reads `.app`, `.name`, `.session_id`, etc.) instead of typed extraction. Daemons that already return an `auki_network.cluster.ParticipantInfo(...)` instance work unchanged — the pyclass exposes every field via `#[getter]`. The duck-typing avoids depending on `auki-network-py`'s Rust crate, which is blocked by a `[lib] name` collision (`auki-network`'s default `auki_network` vs. `auki-network-py`'s explicit `auki_network`).

**Surface NOT in this PR (filed in [`parking_lot.md`](parking_lot.md)):**
- `stream_provider` Python callable — wrapper unconditionally passes `auki_network::stream_runtime::decline_all_streams()`. Producer daemons can `init_domain` but every inbound `/auki/stream/0.1.0` subscription is typed-declined. Blocker is the same lib-name collision; resolution paths sketched in #1.
- `handle.open_*_stream()` consumer-side methods — no Python consumer exists today; Park is Rust-side.
- `handle.update_cluster_doc(new_doc)` for SSE-driven membership refresh — the `ClusterDoc` pyclass lives in `auki-network-py`; same blocker. In the meantime, the local libp2p allow-list reflects cluster membership at `init_domain`-time; new peers joining after the daemon boots aren't dialable until restart. The SDK-side "ClusterRuntime owns its SSE subscription internally" tightening filed in `auki-network/parking_lot.md` would collapse this entirely.
- `DomainAlreadyExists.existing` payload (the winner's `ClusterDoc`) — surfaced only as `.cluster_name` / `.identity` strings; full ClusterDoc surfacing blocked by the same. Greenland T12's `try-join → create-if-none → fall-back-to-join` retry still works (re-issue `init_domain` with the same name; the second call goes through the non-create register path).

**Implementation notes.** Process-wide `OnceLock<Runtime>` for the tokio runtime (separate instance from `auki-network-py`'s — they can't share without a bridge crate; the cost is one extra thread pool, which is fine). Seed-length validation up-front (32 bytes exact for both `wallet_seed` and `peer_seed`). `domain_name` and `addresses` validated synchronously before any Discovery call so misconfiguration surfaces immediately. The swarm is built with `SwarmConfig { enable_relay_server: false, ... }` — relay-server is `aukilabs/relay`'s job; wrapper consumers never want it.

**Tests.** 8 Rust unit tests under `cargo test -p auki-domain-py` exercising: module population (function + class + 5 exception classes present); `seed_array` length validation (16-byte rejects, 33-byte rejects, 32-byte accepts); `init_domain_py` synchronous-validation rejections (empty `domain_name`, empty `addresses`, short `wallet_seed`, short `peer_seed`, unparseable multiaddr); `build_participant_provider` duck-typing — drops on missing attribute, drops on `None` return, extracts a well-shaped object into a `RustParticipantInfo`. The cross-language seed → wallet_id → canonical-string locked vector lives in [`auki-domain`](../auki-domain)'s own tests; this crate doesn't duplicate that.

**Daemon-side cascade.** BoosterApp [scripts/parking_lot.md #10](https://github.com/aukilabs/boosterapp/blob/develop/scripts/parking_lot.md) (Greenland T12) becomes implementable against `init_domain` once it bumps SDK pin; full unblock requires the `stream_provider` follow-up. Sentinel has the parallel T12 task in its own repo.
