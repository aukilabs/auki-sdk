# Parking lot — auki-network-py

Open questions specific to the Python bindings. Cross-cutting questions about the underlying primitives belong in [`auki-network/parking_lot.md`](../../../crates/auki-network/parking_lot.md).

---

## Logging routing — stderr vs `pyo3-log`

Default is stderr — Rust `tracing` → stderr → systemd → journald (matches `auki-identity-py`'s pattern). Filtering is via `RUST_LOG`. No `tracing` subscriber is installed by the crate today; a host process that installs one wins.

If a downstream consumer wants `tracing` events folded into Python's `logging` module (so they can filter / format alongside the rest of their app's logs), the route is a small `pyo3-log` integration. Defer until a real ask appears; BoosterApp's K1 sidecar uses `journalctl` and is fine with stderr.

---

## PyPI distribution

Same status as `auki-identity-py`: Git-only install today (`maturin develop` for development, `pip install git+...#subdirectory=` for downstream). Rust toolchain required at install time. Defer PyPI publication until BoosterApp actually wants it; same tradeoffs as the `auki-identity-py` parking lot.

---

## Type stubs (`auki_network.pyi`)

Improves IDE autocomplete in BoosterApp's sidecar and any future Python consumer. Worth doing once the surface stabilizes — current public surface is `DiscoveryClient`, `ClusterEntry`, `CreateClusterOutcome`, the `cluster.*` stream types, and `AudioFrame`. Not urgent for the initial PR.

---

## Reusing `auki-identity-py`'s recipe vs. exposing the same primitives

`auki-network-py` does **not** re-expose `load_or_mint_seed` / `Wallet` / `app_instance.derive` — those live in `auki-identity-py` already. A consumer (Boosterapp's sidecar) imports both:

```python
import auki_identity   # data primitives (peer_id, seed, app_instance)
import auki_network    # network layer (DiscoveryClient, AudioFrame)
```

Per-component naming makes this natural. If a future consumer wants a single "everything" import, that's a `from auki_sdk import *` umbrella package decision — out of scope here.

---

## Single-task tokio runtime for the cluster

Today the network crate uses `tokio::runtime::Runtime::new()` — multi-thread by default. A typical BoosterApp process has one network runtime per process, so the multi-thread runtime's worker pool is heavily under-utilized.

Could switch to single-thread (`tokio::runtime::Builder::new_current_thread()`) to drop the worker pool — saves a few threads of memory. Not pursued today: multi-thread is the simplest path that makes `Handle::try_current()` succeed inside spawn paths, and the memory footprint is negligible vs. a Python process.

If a process ever spawns many network runtimes (multi-tenant scenario, not on the table for v1), we'd want to revisit and possibly share a single runtime — already what the `OnceLock<Runtime>` pattern does today.

---

## Python version floor

`abi3-py38` matches `auki-identity-py`. Same reasoning: defer until we have a concrete reason to drop 3.8 (likely an underlying dep dropping it). Boosterapp's K1 runs Python 3.10; `abi3-py38` covers it.
