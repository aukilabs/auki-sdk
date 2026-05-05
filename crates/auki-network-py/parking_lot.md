# Parking lot — auki-network-py

Open questions specific to the Python bindings. Cross-cutting questions about the underlying primitives belong in [`auki-network/parking_lot.md`](../auki-network/parking_lot.md). Cross-component decisions live in the [ansuz Notion doc](https://www.notion.so/3565c8e96592809fb674f769d826c1de).

---

## Logging routing — stderr vs `pyo3-log`

Default is stderr — Rust `tracing` → stderr → systemd → journald (matches `auki-identity-py`'s pattern). Filtering is via `RUST_LOG`. Default subscriber is initialized lazily on first `cluster.spawn` via `try_init` so a host process that already installed a subscriber wins.

If a downstream consumer wants `tracing` events folded into Python's `logging` module (so they can filter / format alongside the rest of their app's logs), the route is a small `pyo3-log` integration that runs on first `cluster.spawn`. Defer until a real ask appears; BoosterApp's K1 sidecar uses `journalctl` and is fine with stderr.

---

## Two-runtime test uses fixed loopback ports

`python_tests/test_basic.py::test_two_runtimes_discover_each_other_via_cluster_doc` binds `127.0.0.1:45051` and `127.0.0.1:45052`. The wrapper exposes no introspection API to learn an OS-chosen address after spawn, so the test pre-commits to fixed ports — same trade-off any production deployment makes (operators hand-edit `cluster.json` with known addresses).

If the chosen ports conflict with something on the host, the test fails. Today it works on dev machines and CI is yet-to-be-stood-up.

Possible future: expose `runtime.listen_addresses() -> list[str]` so a Python harness can spin up two peers on OS-chosen ports, learn the addresses post-spawn, then write the cluster.json. Not in scope today; flag here for revisit if the test goes flaky in CI.

---

## PyPI distribution

Same status as `auki-identity-py`: Git-only install today (`maturin develop` for development, `pip install git+...#subdirectory=` for downstream). Rust toolchain required at install time.

When v0.0.14 (the tag that ships this crate, following v0.0.13 which bundled ansuz Batch 1+2 + Option<ParticipantInfo> + auki-logs::set_retention without the wrapper) cuts, the PyPI publication question revives — same tradeoffs as the `auki-identity-py` parking lot. Defer until BoosterApp actually wants it.

---

## Type stubs (`auki_network.pyi`)

Improves IDE autocomplete in BoosterApp's sidecar and any future Python consumer. Worth doing once the surface stabilizes. Not urgent for the initial PR. Small file:

```pyi
class ParticipantInfo:
    app: str
    name: str
    session_id: str
    session_clock_id: str
    session_clock_hash: str
    session_now_ns: int
    cluster_joined_at_ns: int | None
    peer_id: str
    app_instance: str
    def __init__(self, *, app: str, name: str, session_id: str, ...) -> None: ...

class PeerSnapshot:
    peer_id: str
    info: ParticipantInfo
    first_seen_ns: int

class ClusterDoc:
    peer_count: int
    cluster_name: str

class ClusterRuntime:
    def peers(self) -> list[PeerSnapshot]: ...
    def shutdown(self) -> None: ...

class _Cluster:
    ParticipantInfo: type[ParticipantInfo]
    PeerSnapshot: type[PeerSnapshot]
    ClusterDoc: type[ClusterDoc]
    ClusterRuntime: type[ClusterRuntime]
    def load_doc(self, path: str) -> ClusterDoc: ...
    def spawn(self, seed: bytes, doc: ClusterDoc, participant_provider, *,
              listen_addresses: list[str] | None = None,
              agent_version: str | None = None,
              enable_mdns: bool = True) -> ClusterRuntime: ...

cluster: _Cluster
```

---

## Reusing `auki-identity-py`'s recipe vs. exposing the same primitives

`auki-network-py` does **not** re-expose `load_or_mint_seed` / `Wallet` / `app_instance.derive` — those live in `auki-identity-py` already. A consumer (Boosterapp's sidecar) imports both:

```python
import auki_identity   # data primitives (peer_id, seed, app_instance)
import auki_network    # network layer (cluster.spawn, peers)
```

Per-component naming makes this natural. If a future consumer wants a single "everything" import, that's a `from auki_sdk import *` umbrella package decision — out of scope here.

---

## Single-task tokio runtime for the cluster

Today `cluster_tokio_runtime()` is `tokio::runtime::Runtime::new()` — multi-thread by default. The `auki-network` `ClusterRuntime` it backs only spawns one task per `cluster.spawn` call. A typical BoosterApp process has one cluster runtime (one libp2p peer per process), so the multi-thread runtime's worker pool is heavily under-utilized.

Could switch to single-thread (`tokio::runtime::Builder::new_current_thread()`) to drop the worker pool — saves a few threads of memory. Not pursued today: multi-thread is the simplest path that makes `Handle::try_current()` succeed inside `ClusterRuntime::spawn`, and the memory footprint is negligible vs. a Python process.

If a process ever spawns many cluster runtimes (multi-tenant scenario, not on the table for ansuz), we'd want to revisit and possibly share a single runtime across multiple ClusterRuntimes — already what `OnceLock<Runtime>` does today.

---

## Python version floor

`abi3-py38` matches `auki-identity-py`. Same reasoning: defer until we have a concrete reason to drop 3.8 (likely an underlying dep dropping it). Boosterapp's K1 runs Python 3.10; `abi3-py38` covers it.
