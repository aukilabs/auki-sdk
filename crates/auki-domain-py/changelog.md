# Changelog — `auki-domain-py`

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
