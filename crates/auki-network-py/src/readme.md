# `auki-network-py/src/`

PyO3 bindings for `auki-network`'s cluster layer — the libp2p-peer-from-Python wrapper. Spec: this crate's [outer `README.md`](../README.md).

## What's here

- [`lib.rs`](lib.rs) — the cluster + grimsby surface entry points (`#[pymodule]` macro generates the C entry point Python imports). PyO3 0.22 `Bound<...>` API.
- [`stream_bridge.rs`](stream_bridge.rs) + [`stream_types.rs`](stream_types.rs) — grimsby's `Stream<T>` PyO3 wrappers (deliverable #4) lifted by Dagaz Batch 2 to multi-`T` dispatch (`JpegFrame` + `PointCloudFrame`). Async-iter ↔ Rust `futures::Stream` bridge for the producer side; sync-blocking iterator on the consumer side.
- [`discovery.rs`](discovery.rs) — Vinland Batch 2 PyO3 wrapper for `auki_network::discovery_client`. Sync-shaped `DiscoveryClient(url)` with `register` / `fetch` / `deregister`; three typed Python exceptions (`DiscoveryUnreachable`, `DiscoveryRejected`, `DiscoveryClockError`).

## Public Python surface

```python
auki_network.cluster.ParticipantInfo(*,
    app, name, session_id, session_clock_id, session_clock_hash,
    session_now_ns, cluster_joined_at_ns, peer_id, app_instance,
)                                          # typed wire shape; consumer constructs to return from provider

auki_network.cluster.PeerSnapshot          # read-only — runtime emits these via peers()
auki_network.cluster.ClusterDoc            # opaque; load_doc returns one
auki_network.cluster.ClusterRuntime        # opaque; spawn returns one

auki_network.cluster.load_doc(path: str) -> ClusterDoc
auki_network.cluster.resolve_cluster_name(flag: str | None = None) -> str
    # Strict resolver — flag wins, AUKI_CLUSTER_NAME is fallback,
    # neither set → ValueError("cluster_name unset: ...").
    # Sawslin Decision #1; daemons call this once at startup.
auki_network.cluster.spawn(
    seed: bytes,                           # 32-byte ed25519 seed
    doc: ClusterDoc,
    participant_provider: Callable[[], ParticipantInfo | None],
    *,
    listen_addresses: list[str] | None = None,    # default: TCP+QUIC on 0.0.0.0
    agent_version: str | None = None,             # default: "auki-network-py/<version>"
    enable_mdns: bool = True,
) -> ClusterRuntime

ClusterRuntime.peers() -> list[PeerSnapshot]
ClusterRuntime.open_stream(peer_id: str, sensor_id: str) -> StreamSubscription   # JPEG (grimsby v1)
ClusterRuntime.open_pointcloud_stream(peer_id: str, sensor_id: str) -> StreamSubscription  # CDR PointCloud2 (Dagaz Batch 2)
ClusterRuntime.shutdown() -> None          # consumes; second call raises RuntimeError

# Stream<T> producer surface — Dagaz Batch 2 widens the JPEG-only grimsby v1 surface.
# Each substream stays mono-T; the producer's StreamDecision factory picks which T.
auki_network.cluster.JpegFrame(bytes: bytes)              # grimsby v1 payload
auki_network.cluster.PointCloudFrame(bytes: bytes)        # Dagaz Batch 2 — raw CDR PointCloud2
auki_network.cluster.ProducerFrame(*,
    timestamp_ns: int,
    payload: JpegFrame | PointCloudFrame,                 # type-checked at construction
)
auki_network.cluster.StreamDecision.accept(*, info, source)             # JPEG substream
auki_network.cluster.StreamDecision.accept_pointcloud(*, info, source)  # PointCloud substream
auki_network.cluster.StreamDecision.decline(reason)

# Vinland Batch 2 (v0.0.19) — Discovery REST client.
auki_network.discovery.DiscoveryClient(url: str)             # cheap; shareable across threads
DiscoveryClient.base_url -> str                               # property; trailing slash trimmed
DiscoveryClient.register(*,
    seed: bytes,                                              # 32-byte PARENT wallet seed (not peer seed!)
    cluster_name: str,
    addresses: list[str],                                     # multiaddr strings; empty list allowed
    expected_app_id: str | None = None,                       # advisory operator metadata
    note: str | None = None,                                  # advisory operator metadata
) -> ClusterDoc                                               # the same Python type cluster.load_doc returns

DiscoveryClient.fetch(cluster_name: str) -> ClusterDoc        # GET /clusters/{name}; read-only
DiscoveryClient.deregister(*, seed: bytes, cluster_name: str) -> None

# Three typed Python exceptions for the Rust DiscoveryError variants.
auki_network.discovery.DiscoveryUnreachable                   # transport / DNS / timeout
auki_network.discovery.DiscoveryRejected                      # HTTP non-2xx; .status: int + .body: str attrs
auki_network.discovery.DiscoveryClockError                    # rare; system clock broken
```

## How the wrappers map to the Rust crates

| Python | Rust |
|---|---|
| `cluster.ParticipantInfo(...)` | `auki_network::participant::ParticipantInfo` (struct field-for-field; `peer_id` parsed from string) |
| `cluster.PeerSnapshot` | `auki_network::cluster_runtime::PeerSnapshot` |
| `cluster.ClusterDoc` | opaque wrapper around `auki_network::cluster_doc::ClusterDoc` |
| `cluster.ClusterRuntime` | `Mutex<Option<auki_network::cluster_runtime::ClusterRuntime>>` |
| `cluster.load_doc(path)` | `auki_network::cluster_doc::load(&Path)` |
| `cluster.spawn(seed, doc, provider, **kwargs)` | `auki_network::cluster_runtime::ClusterRuntime::spawn(&[u8; 32], doc, SwarmConfig, Arc<dyn Fn() -> Option<ParticipantInfo>>)` |
| `runtime.peers()` | `ClusterRuntime::peers() -> Vec<PeerSnapshot>` |
| `runtime.shutdown()` | `ClusterRuntime::shutdown(self)` (consumes) |
| `discovery.DiscoveryClient(url)` | `auki_network::discovery_client::DiscoveryClient::new(url)` |
| `client.register(seed, cluster_name, addresses, **opts)` | `discovery_client::DiscoveryClient::register(&Wallet, &str, &[Multiaddr], Option<&str>, Option<&str>)` (wrapper reconstructs Wallet from `seed`, parses multiaddr strings, `block_on`s on `cluster_tokio_runtime()`) |
| `client.fetch(cluster_name)` | `discovery_client::DiscoveryClient::fetch(&str)` |
| `client.deregister(seed, cluster_name)` | `discovery_client::DiscoveryClient::deregister(&Wallet, &str)` |

The participant_provider closure is the seam where the GIL meets tokio. The wrapper holds the Python callable as `Py<PyAny>`; on each invocation (one per inbound `/auki/cluster/1.0.0` request), it acquires the GIL, calls the callable with `call0()`, and:

- returns `Some(rust_info)` if the callable returned a `ParticipantInfo`,
- returns `None` if the callable returned Python `None`, raised an exception (caught + `tracing::warn!`-logged), or returned a non-`ParticipantInfo` (also warn-logged).

The runtime treats `None` as "drop the reply channel" — the requester sees a request timeout. Runtime stays alive; future requests still get answered.

## Error mapping

| Rust | Python |
|---|---|
| `cluster_doc::LoadError::Io(_)` | `OSError` |
| `cluster_doc::LoadError::Parse(_)` | `ValueError` |
| `cluster_doc::LoadError::UnsupportedVersion(v)` | `ValueError` (message names `v` and the supported version) |
| `cluster_doc::LoadError::InvalidPeerId(s)` | `ValueError` (message names `s`) |
| `cluster_doc::LoadError::InvalidMultiaddr(s)` | `ValueError` (message names `s`) |
| `PeerId::from_str` failure (in `ParticipantInfo` ctor) | `ValueError` (message names the bad string + libp2p's reason) |
| `discovery_client::DiscoveryError::Transport(_)` | `auki_network.discovery.DiscoveryUnreachable` (transport / DNS / connect / TLS / timeout) |
| `discovery_client::DiscoveryError::Status { status, body }` | `auki_network.discovery.DiscoveryRejected` (with `.status: int` + `.body: str` attrs) |
| `discovery_client::DiscoveryError::Clock(msg)` | `auki_network.discovery.DiscoveryClockError` |
| `register` / `deregister` seed wrong length | `ValueError` ("seed must be exactly 32 bytes") |
| `register` invalid multiaddr in `addresses` | `ValueError` ("invalid multiaddr") |
| `len(seed) != 32` (in `cluster.spawn`) | `ValueError` (message names the actual length) |
| `Multiaddr::from_str` failure (in `cluster.spawn`) | `ValueError` (message names the bad string + parser reason) |
| `cluster_runtime::SpawnError::BuildSwarm(_)` | `RuntimeError` |
| `cluster_runtime::SpawnError::NoTokioRuntime` | `RuntimeError` (should not occur — the wrapper enters runtime context before calling spawn) |
| Use-after-shutdown (`peers()` / `shutdown()` post-`shutdown`) | `RuntimeError` (`"ClusterRuntime has been shut down"`) |

## Tokio runtime singleton

`cluster_tokio_runtime() -> &'static Runtime` — `OnceLock<Runtime>` lazily holding a multi-thread `tokio::runtime::Runtime`. Created on the first `cluster.spawn`; lives for the rest of the process. Multi-thread because the simplest path that makes `tokio::runtime::Handle::try_current()` succeed inside `ClusterRuntime::spawn`. A typical wrapper consumer holds 1–2 `ClusterRuntime`s, so the worker pool is heavily under-utilized — that's fine.

The same lazy-init also installs a default `tracing-subscriber` (`fmt` + `env-filter`) writing to stderr with `RUST_LOG`-driven filtering (default `warn`). `try_init` is idempotent: a host process that already installed a subscriber wins.

## GIL and tokio worker interaction

The runtime task spawned by `ClusterRuntime::spawn` lives on tokio's worker pool. When the runtime needs to invoke the participant_provider, the wrapper closure runs on whichever tokio worker happened to be polling — that worker acquires the GIL, calls the Python callable, releases the GIL, returns to tokio.

While the GIL is held by the wrapper, no other Python thread can run; the runtime's task is blocked. Brief contention is fine; sustained contention is not. **Documented in the outer README's "Provider performance contract" section.**

## Build modes

The crate is dual-mode by design — same pattern as `auki-identity-py`.

- **Python extension build** (`maturin develop` / `maturin build`) — `crate-type = ["cdylib"]` with PyO3's `extension-module` feature on. Maturin enables it via `[tool.maturin].features = ["pyo3/extension-module"]` in [`pyproject.toml`](../pyproject.toml).
- **Rust-side test build** (`cargo test`) — `crate-type` includes `rlib`; default Cargo features are *empty* so `extension-module` is off; the dev-dep on `pyo3` with `auto-initialize` links a real Python interpreter into the test binary. This lets the smoke tests in [`lib.rs`](lib.rs)'s `#[cfg(test)] mod tests` use `Python::with_gil` without a host process.

The two feature modes are mutually exclusive; the default-empty + maturin-enables-it pattern keeps both code paths working without manual feature flags.

## Tests

`cargo test -p auki-network-py` runs **40 Rust-side smoke tests**; `pytest python_tests/` runs **44 Python-side tests** (or 51 with `DISCOVERY_BIN=/path/to/discovery` set, enabling the live Discovery integration tests).

### Rust-side (`lib.rs`)

| Test | Asserts |
|---|---|
| `module_exposes_cluster_submodule_with_documented_surface` | `auki_network.cluster` exposes the 6-name surface (4 classes + 2 functions) |
| `participant_info_round_trips_through_constructor_and_getters` | Every field round-trips through `#[new]` + getters |
| `participant_info_rejects_invalid_peer_id` | Bad peer_id string → `ValueError` |
| `participant_info_eq_compares_all_fields` | Field-wise equality; mutation breaks |
| `load_doc_round_trips_a_minimal_cluster_json` | Happy path: minimal valid file → `ClusterDoc` with correct `cluster_name` and `peer_count` |
| `load_doc_rejects_missing_file_with_oserror` | Filesystem error → `OSError` |
| `load_doc_rejects_unsupported_version_with_value_error` | Bad version → `ValueError` (message names the version) |
| `load_doc_rejects_invalid_peer_id_with_value_error` | Bad peer_id in doc → `ValueError` |
| `print_python_e2e_peer_ids` | Output emitter — prints PeerIds for `[0x10;32]` / `[0x11;32]` so the Python E2E baked literals can be regenerated |
| `spawn_rejects_wrong_seed_length_with_value_error` | 16-byte seed → `ValueError` (no Rust panic) |
| `spawn_rejects_invalid_multiaddr_with_value_error` | Bad multiaddr string → `ValueError` |
| `spawn_then_peers_then_shutdown_round_trip` | Real-runtime exercise: spawn empty cluster on loopback, peers() == [], shutdown succeeds, second shutdown raises, post-shutdown peers() raises |

### Python-side (`python_tests/test_basic.py`)

| Test | Asserts |
|---|---|
| `test_module_exposes_only_documented_apis` | Pin the 6-name cluster sub-module surface |
| `test_top_level_module_exposes_cluster` | `import auki_network` + `from auki_network import cluster` both work |
| `test_participant_info_round_trips_through_constructor_and_getters` | Every field round-trips |
| `test_participant_info_accepts_none_cluster_joined_at_ns` | `cluster_joined_at_ns=None` is valid |
| `test_participant_info_rejects_invalid_peer_id` | Bad peer_id → `ValueError` |
| `test_participant_info_eq_compares_all_fields` | Equality + inequality |
| `test_participant_info_repr_is_informative` | `repr()` mentions app, name, peer_id |
| `test_load_doc_round_trips_minimal` | Happy path |
| `test_load_doc_with_peers` | Peer count visible after load |
| `test_load_doc_missing_file_raises_oserror` | Filesystem error → `OSError` |
| `test_load_doc_invalid_json_raises_value_error` | JSON syntax error → `ValueError` |
| `test_load_doc_unsupported_version_raises_value_error` | Bad version → `ValueError` |
| `test_load_doc_invalid_peer_id_raises_value_error` | Bad peer_id in doc → `ValueError` |
| `test_spawn_rejects_wrong_seed_length` | 16-byte seed → `ValueError` |
| `test_spawn_rejects_invalid_listen_multiaddr` | Bad `listen_addresses` → `ValueError` |
| `test_spawn_then_shutdown_round_trip` | Empty cluster → spawn → peers() == [] → shutdown → second-shutdown raises → post-shutdown peers() raises |
| `test_two_runtimes_discover_each_other_via_cluster_doc` | **The big one.** Two `cluster.spawn` instances at fixed loopback ports converge within 10s; each sees the other's `peer_id` / `app` / `name` / non-zero `first_seen_ns` |
| `test_spawn_with_raising_provider_does_not_panic` | A provider that raises is acceptable; runtime stays alive |

### Vinland Batch 2 — `discovery::tests` (Rust-side, 6) + `python_tests/test_discovery.py` (17)

Rust-side smoke tests in [`discovery.rs`](discovery.rs):

| Test | Asserts |
|---|---|
| `discovery_client_constructs_and_trims_url` | `DiscoveryClient::new(url)` trims trailing `/` |
| `register_rejects_wrong_seed_length` | 16-byte seed → `ValueError` |
| `register_rejects_invalid_multiaddr` | Bad multiaddr in `addresses` → `ValueError` |
| `deregister_rejects_wrong_seed_length` | 64-byte seed → `ValueError` |
| `fetch_against_unreachable_url_raises_unreachable` | Port 1 (RST'd immediately) → typed `DiscoveryUnreachable` |
| `module_registration_exposes_the_documented_surface` | `register_module` adds `DiscoveryClient` + 3 exception classes |

Plus the surface pin in [`lib.rs`](lib.rs)'s `tests::module_exposes_discovery_submodule_with_documented_surface`.

Python-side (`python_tests/test_discovery.py`, 17 tests):

| Tier | Test | Asserts |
|---|---|---|
| Offline | `test_discovery_submodule_exposes_documented_surface` | Pin the 4-name surface (`DiscoveryClient` + 3 exceptions) |
| Offline | `test_top_level_module_exposes_discovery_submodule` | `from auki_network import discovery` works |
| Offline | `test_constructor_trims_trailing_slash` | URL hygiene |
| Offline | `test_repr_includes_base_url` | `repr` mentions the URL |
| Offline | `test_register_rejects_short_seed` / `_long_seed` | 16- or 64-byte seed → `ValueError` |
| Offline | `test_register_rejects_invalid_multiaddr` | Bad multiaddr → `ValueError` |
| Offline | `test_deregister_rejects_short_seed` | Seed validation symmetric across methods |
| Offline | `test_register_accepts_empty_addresses` | Empty `addresses` is allowed; the network call still fails because port 1 isn't listening, surfaces as `DiscoveryUnreachable` |
| Offline | `test_fetch_against_unreachable_url_raises_unreachable` | Port 1 → `DiscoveryUnreachable` |
| Live | `test_register_returns_full_cluster_doc` | Single-peer boot — `register` returns the full `ClusterDoc` |
| Live | `test_three_peers_converge` | Sentinel/Booster/Park sequential register → `fetch` shows all three |
| Live | `test_deregister_removes_peer` | Register two, deregister one, fetch shows just the other |
| Live | `test_deregister_already_removed_raises_404` | Idempotency: second deregister → `DiscoveryRejected` with `.status == 404` |
| Live | `test_fetch_unknown_cluster_raises_404` | Unknown cluster → `DiscoveryRejected` with `.status == 404` |
| Live | `test_path_traversal_cluster_name_rejected` | `"../etc/passwd"` rejected (charset guard) |
| Live | `test_returned_cluster_doc_is_usable_with_cluster_runtime` | Schema-parity: returned doc `isinstance(doc, cluster.ClusterDoc)` |

The 7 "Live" tests gate on `DISCOVERY_BIN=/path/to/discovery` and skip cleanly when unset; the fixture spawns Discovery as a subprocess on a tempdir + ephemeral loopback port and tears it down on teardown. Same shape as the Rust-side `tests/discovery_integration.rs`.

## Dependencies

- `auki-network` (path, `swarm` + `discovery_client` features) — the upstream cluster runtime, protocol, doc, participant types, and (Vinland Batch 2) the discovery REST client. Renamed to `auki-network-rs` in `Cargo.toml` via `package =` to avoid colliding with our own lib name `auki_network` (which is also the Python module name).
- `auki-identity` (path) — `Wallet::from_seed` reconstruction at the FFI seam for `discovery.register` / `discovery.deregister`. The wrapper takes the parent wallet seed as 32 bytes; reconstructing the Wallet here keeps the Python surface seed-bytes-only and avoids cross-crate pyclass sharing.
- `libp2p-identity` 0.2 — `PeerId` parsing/formatting on the FFI seam.
- `multiaddr` 0.18 — parsing the `listen_addresses` kwarg + the `discovery.register` `addresses` list into `Multiaddr`.
- `tokio` with `rt-multi-thread` — the runtime singleton (shared by `cluster.spawn` and `discovery.DiscoveryClient` `block_on` callsites).
- `tracing` + `tracing-subscriber` — default subscriber writing to stderr with `RUST_LOG`-driven filtering; `tracing::warn!` on caught provider exceptions.
- `pyo3` 0.22 with `abi3-py38` — stable-ABI bindings, one wheel works on Python 3.8+.

Build-time:

- `maturin` 1.5+ — invoked through `pyproject.toml`'s `build-system`.

## Consumers

- **Boosterapp Python sidecar** — wraps `cluster.spawn` to participate in ansuz, exposes `runtime.peers()` via the new `/api/cluster` endpoint and threads `cluster_joined_at_ns` into outbound `/api/info` once the first peer connects. First and primary consumer; deliverable #7 (sidecar libp2p integration) on the BoosterApp side.
- *(future)* Any Python consumer of the Auki SDK that wants libp2p cluster participation. The surface is stable; future bindings (`auki-logs-py`, `auki-session-py`, etc.) will retire other Python reimplementations one component at a time.
