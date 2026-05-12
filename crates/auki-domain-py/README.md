# `auki-domain-py`

PyO3 bindings for [`auki-domain`](../auki-domain).

## What this crate is

The post-v0.0.33 entry point Python daemons (BoosterApp, Sentinel) use to construct a `ClusterRuntime` through Discovery. `auki-network-py`'s `cluster.spawn` Python function was removed in [auki-sdk v0.0.33](https://github.com/aukilabs/auki-sdk/releases/tag/v0.0.33) — see the [PR B changelog entry](../auki-network/changelog.md). This crate is the replacement.

## Surface

```python
from auki_domain import init_domain, DomainAlreadyExists, DiscoveryUnreachable

handle = init_domain(
    wallet_seed=wallet_seed,          # 32 bytes — parent wallet seed
    peer_seed=peer_seed,              # 32 bytes — peer/v1 derived seed
    discovery_url="http://discovery.lan:8080",
    domain_name="Vinland",            # reserved singleton; or "my-team-domain"
    addresses=["/ip4/192.168.9.72/tcp/4001"],
    participant_provider=lambda: my_participant_info,
    listen_addresses=["/ip4/0.0.0.0/tcp/4001"],  # optional
    expected_app_id="boosterapp",                # optional
    note="k1-walker",                             # optional
)

print(handle.identity)             # "Vinland" or "<wallet_id>/my-team-domain"
for peer in handle.peers():
    print(peer["peer_id"], peer["info"]["name"])
handle.shutdown()
```

## Architecture

Discovery is **mandatory**. There's no static-`cluster.json` fallback path in v0.0.33+ — the libp2p allow-list is populated from `ClusterDoc.peers` returned by Discovery's `register` call, and that's the only sanctioned way to obtain a `ClusterRuntime`. The Rust SDK enforces this through `ClusterRuntime::from_swarm` being `#[doc(hidden)] pub` and `init_domain` being the public path.

`init_domain` is async on the Rust side; this wrapper `block_on`s on a process-wide tokio runtime so the Python caller stays sync-shaped. Same pattern as `auki-network-py`'s `DiscoveryClient`.

The `participant_provider` callable is invoked on every inbound `/auki/cluster/0.0.1` request. It returns a Python object whose attributes are duck-typed to a `ParticipantInfo` shape (`app`, `name`, `session_id`, `session_clock_id`, `session_clock_hash`, `session_now_ns`, `cluster_joined_at_ns`, `peer_id`, `app_instance`). Daemons that already return an `auki_network.cluster.ParticipantInfo(...)` instance can keep doing so — the duck-typing reads attributes that pyclass exposes via `#[getter]`s.

## What's NOT in this PR

- **`stream_provider` Python callable.** The wrapper passes `auki_network::stream_runtime::decline_all_streams()` so producer-side stream support is degraded vs. the pre-v0.0.33 `cluster.spawn` surface. Wiring a Python callable through requires reusing `auki-network-py`'s `build_stream_provider` (currently `pub(crate)`) or duplicating ~500 lines of `PyStreamDecision` / `PyAcceptInfo` / `PyDeclineReason` pyclass plumbing across the two PyO3 crates (the lib-name collision between `auki-network`'s `auki_network` lib and `auki-network-py`'s `auki_network` lib blocks a direct dep). Filed in [`parking_lot.md`](parking_lot.md) as the immediate follow-up. BoosterApp + Sentinel can `init_domain` and run peer-list logic today, but can't accept inbound stream subscriptions until the follow-up lands.

- **`handle.open_stream(...)` consumer-side methods.** Park (the consumer) is Rust-side and uses `auki-network`'s Rust API directly. No Python daemon consumes streams today. Filed in the same follow-up.

- **`handle.update_cluster_doc(new_doc)` for SSE-driven membership refresh.** `init_domain` only performs the initial create-and-register; the daemon should subscribe to Discovery's SSE stream and feed fresh `ClusterDoc`s in so the libp2p allow-list stays in sync. The `ClusterDoc` Python pyclass would need to be reachable from this crate — same lib-name-collision blocker as `stream_provider`. Filed as a follow-up. In the meantime, the local allow-list reflects cluster membership at `init_domain`-time; peers that join after won't be dialable until the daemon restarts. Parking-lot item ["ClusterRuntime owns its SSE subscription internally"](../auki-network/parking_lot.md) (filed at the time PR B landed) is the SDK-side tightening that resolves this.

## Building

This is a maturin-built crate; standard `maturin develop` / `maturin build` workflow per `pyproject.toml`. Tests run via `cargo test -p auki-domain-py` (Rust unit tests with `pyo3 = auto-initialize`) and via `pytest python_tests/` once the wheel is installed.

```bash
# Rust-only test sweep:
cargo test -p auki-domain-py

# Python smoke test (requires the wheel installed):
pip install maturin
maturin develop -m crates/auki-domain-py/Cargo.toml
pytest crates/auki-domain-py/python_tests/
```

## Daemon-side cascade

Per the [v0.0.33 root changelog](../../changelog.md):

> Daemon-side cascade (handled in each daemon repo as follow-ups):
> - **BoosterApp** — drop `--cluster-doc` CLI flag and `AUKI_CLUSTER_DOC` env var; route boot through `init_domain` (or T12's algorithm once `fetch_latest` + `join_domain` land).
> - **Sentinel** — same as BoosterApp.

BoosterApp's [scripts/parking_lot.md #10](https://github.com/aukilabs/boosterapp/blob/develop/scripts/parking_lot.md) is the daemon-side T12 task. Sentinel has the parallel one in its own repo.
