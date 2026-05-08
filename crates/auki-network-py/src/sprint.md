# Sprint — auki-network-py

Current work and next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now (cluster + grimsby + Vinland Batch 2 + Dagaz Batch 2 — landed)

The crate ships three Python sub-modules:

- **`auki_network.cluster`** — ansuz cluster runtime + types (initial release).
- **`auki_network.cluster.{StreamRequest, AcceptInfo, JpegFrame, PointCloudFrame, ...}`** — grimsby `Stream<T>` surface (deliverable #4 / v0.0.17) lifted by [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) Batch 2 (v0.0.21) to multi-`T` dispatch. `cluster.StreamDecision.accept(info, source)` produces a JPEG substream; the new `accept_pointcloud(info, source)` produces a CDR-encoded `PointCloud2` substream. Consumer side: `runtime.open_stream(peer_id, sensor_id)` opens JPEG; `runtime.open_pointcloud_stream(peer_id, sensor_id)` opens PointCloud. Each substream stays mono-`T` end-to-end.
- **`auki_network.discovery`** — Vinland Batch 2 REST client wrapping `auki_network::discovery_client` (shipped in v0.0.19). Sync-shaped `DiscoveryClient(url)` with `register` / `fetch` / `deregister`; three typed Python exceptions (`DiscoveryUnreachable`, `DiscoveryRejected`, `DiscoveryClockError`). Pattern A bridge — each method `block_on`s on the existing process-wide `cluster_tokio_runtime()`.

The crate ships the full cluster-layer Python surface and nothing else:

- `cluster.ParticipantInfo(*, app, name, ...)` — typed `#[pyclass]` with kwargs-only `#[new]`, getters for all 9 wire-shape fields, `__repr__`, `__eq__`. Wire-shape contract pinned by `auki_network::participant::ParticipantInfo` (one schema, two transports — `/api/info` HTTP + `/auki/cluster/0.0.1` libp2p).
- `cluster.PeerSnapshot` — typed read-only view, runtime-emitted via `runtime.peers()`. Three fields: `peer_id: str`, `info: ParticipantInfo`, `first_seen_ns: int`.
- `cluster.ClusterDoc` — opaque handle returned by `cluster.load_doc`. `peer_count` and `cluster_name` getters for sanity-checks.
- `cluster.ClusterRuntime` — opaque handle returned by `cluster.spawn`. `peers()` from any thread (lock-light); `shutdown()` consumes (post-shutdown calls raise `RuntimeError`).
- `cluster.load_doc(path) -> ClusterDoc` — wraps `auki_network::cluster_doc::load` with full `LoadError` mapping (`OSError` / `ValueError`).
- `cluster.spawn(seed, doc, participant_provider, *, listen_addresses=None, agent_version=None, enable_mdns=True) -> ClusterRuntime` — boots an `auki_network::cluster_runtime::ClusterRuntime` against a process-wide tokio runtime owned in `OnceLock<Runtime>`. Wrapper-side validation for `seed` length and listen-multiaddr strings; default listen on TCP+QUIC `0.0.0.0`; default `agent_version` of `auki-network-py/<version>`. The Python `participant_provider` callable is wrapped to catch exceptions + non-`ParticipantInfo` returns, log via `tracing::warn!`, and signal `None` to the runtime (drops the reply channel cleanly).

Built via `maturin` (PEP 517 backend declared in `pyproject.toml`). PyO3 0.22 with the `Bound<...>` API; `abi3-py38`; `crate-type = ["cdylib", "rlib"]`.

Tests: 40 Rust-side smoke tests (`cargo test -p auki-network-py`); 44 Python-side tests (`pytest python_tests/`) or 51 with `DISCOVERY_BIN=/path/to/discovery` set, enabling the Vinland-Batch-2 live integration tests in `python_tests/test_discovery.py`.

## Next

In priority order:

1. **Type stubs (`auki_network.pyi`).** Improves IDE autocomplete in BoosterApp's sidecar and any future Python consumer. Surface is stable; ~80 lines of `.pyi` (cluster + grimsby + Vinland + Dagaz). Tracked in [`parking_lot.md`](../parking_lot.md).

2. **Layer 2 capability advertisement.** When `auki-network` ships a `/auki/capabilities/1.0.0` request-response protocol (the State of SDK doc names this as "the biggest single gap on the networking side"), the wrapper exposes it through `runtime.fetch_capabilities(peer_id)` or similar. Sibling to `discovery_client`'s shape — sync-blocking call with `block_on` on the shared tokio runtime.

## Smaller follow-ups

- **`pyo3-log` integration** — route Rust `tracing` events into Python's `logging` module if a downstream consumer wants filtering / formatting alongside their own logs. Defer until a real ask. Currently stderr → systemd → journald via `tracing-subscriber`'s default fmt subscriber.
- **PyPI publication.** Same status as `auki-identity-py`'s parking lot — defer until BoosterApp wants it. Single coordinated wheel matrix when we ship.
- **`runtime.listen_addresses() -> list[str]`** — introspection of the bound addresses post-spawn. Today the two-runtime Python E2E test pre-commits to fixed loopback ports because the wrapper has no introspection API. Add only if real consumers want OS-chosen ports + dynamic cluster-doc generation.
- **`__version__` attribute.** Standard Python convention; trivial to add.

## Open items

See [`../parking_lot.md`](../parking_lot.md). Nothing blocking — all forward-looking trade-offs.

## Out of scope by design

- **Async / asyncio.** Synchronous Python API. The internal tokio runtime is hidden; consumers don't `await` anything. If a future asyncio-native consumer wants `await runtime.next_peer_event()`, that's a separate API design.
- **`from_swarm` constructor.** The Rust `ClusterRuntime::from_swarm` accepts a pre-built `Swarm<Behaviour>` — useful for tests that need to learn bound addresses before composing the cluster doc. Not exposed to Python because Python can't construct a libp2p `Swarm`. The brief explicitly skipped this.
- **WASM.** Native Python extension only.
- **Re-exposing `auki-identity-py`'s primitives.** Per-component naming means consumers import both packages.
