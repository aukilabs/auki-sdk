# auki-network-py

PyO3 bindings for `auki-network`'s cluster layer — lets a Python process participate in an [ansuz](https://www.notion.so/3565c8e96592809fb674f769d826c1de) cluster as a libp2p peer.

The Python sidecar in [BoosterApp](https://github.com/aukilabs/boosterapp) runs `rclpy` for ROS ingestion; without bindings it can't be a libp2p peer. This crate is the bridge.

## Surface

```python
import auki_network as auki

# 1. Read and parse the discovery doc.
doc = auki.cluster.load_doc("/var/lib/boosterapp/registries/cluster_registries/cluster.json")

# 2. Build a ParticipantInfo from session-local state. The provider
#    closure below returns a fresh one on every call — `session_now_ns`
#    must be current per reply, not stale at spawn time.
def participant_provider() -> auki.cluster.ParticipantInfo:
    return auki.cluster.ParticipantInfo(
        app="boosterapp", name="k1-walker",
        session_id=current_session_id,
        session_clock_id=current_clock_id,
        session_clock_hash=current_clock_hash,
        session_now_ns=current_session_clock_value(),
        cluster_joined_at_ns=first_peer_ns_or_None,
        peer_id=our_peer_id,
        app_instance=our_app_instance,
    )

# 3. Spawn the cluster runtime. Lazily creates a process-wide tokio
#    runtime on first call.
#
#    Note: `seed` is the *peer* seed, not the wallet seed. For the
#    typical wallet-rooted pattern, derive the peer wallet first and
#    hand its `.seed()` here — otherwise the swarm's PeerId won't
#    match the wallet-derived peer identity in cluster.json.
import auki_identity
wallet = auki_identity.Wallet.from_seed(
    auki_identity.load_or_mint_seed("/var/lib/boosterapp/identity.seed")
)
peer = wallet.derive_child("peer/v1")  # canonical "peer/v1" label

runtime = auki.cluster.spawn(
    seed=peer.seed(),                  # 32-byte derived peer seed (NOT wallet seed)
    doc=doc,
    participant_provider=participant_provider,
    listen_addresses=None,             # default: TCP+QUIC on 0.0.0.0, OS-chosen ports
    agent_version=None,                # default: "auki-network-py/<crate-version>"
    enable_mdns=True,                  # default: True; matches SwarmConfig::default
)
# runtime's libp2p PeerId == peer.peer_id(), by construction.

# 4. Read the live peer state from any thread (HTTP handler, control loop, ...).
for peer in runtime.peers():
    print(peer.peer_id, peer.info.app, peer.info.name, peer.first_seen_ns)

# 5. Clean shutdown — consumes the runtime; subsequent peers() / shutdown() raise RuntimeError.
runtime.shutdown()
```

### `Stream<T>` (grimsby + Dagaz Batch 2)

Same `runtime` participates in `/auki/stream/1.0.0` substreams over the same swarm — no second libp2p stack. The producer's `stream_provider` callable returns a typed `StreamDecision`, the consumer opens a typed subscription:

```python
# Producer side — one callable, two T's. Each substream stays mono-T.
def stream_provider(req: auki.cluster.StreamRequest) -> auki.cluster.StreamDecision:
    if req.sensor_id == "head_left_cam":
        async def jpeg_source():
            async for jpeg_bytes in jpeg_fanout.subscribe():
                yield auki.cluster.ProducerFrame(
                    timestamp_ns=session_clock_now_ns(),
                    payload=auki.cluster.JpegFrame(jpeg_bytes),
                )
        return auki.cluster.StreamDecision.accept(
            info=auki.cluster.AcceptInfo(sensor_hash="...", clock_id="...", clock_hash="..."),
            source=jpeg_source(),
        )
    if req.sensor_id == "head/pointcloud":
        async def pc_source():
            async for cdr_bytes in pointcloud_fanout.subscribe():
                yield auki.cluster.ProducerFrame(
                    timestamp_ns=session_clock_now_ns(),
                    payload=auki.cluster.PointCloudFrame(cdr_bytes),
                )
        return auki.cluster.StreamDecision.accept_pointcloud(
            info=auki.cluster.AcceptInfo(sensor_hash="...", clock_id="...", clock_hash="..."),
            source=pc_source(),
        )
    return auki.cluster.StreamDecision.decline(
        auki.cluster.DeclineReason.sensor_not_found(),
    )

runtime = auki.cluster.spawn(
    seed=peer.seed(), doc=doc,
    participant_provider=participant_provider,
    stream_provider=stream_provider,         # Dagaz Batch 2 — multi-T
)

# Consumer side — pick the open method that matches the substream T.
sub = runtime.open_stream(peer_id=their_peer_id, sensor_id="head_left_cam")
for frame in sub.frames():
    handle_jpeg(frame.timestamp_ns, frame.seq, frame.payload.bytes)

sub_pc = runtime.open_pointcloud_stream(peer_id=their_peer_id, sensor_id="head/pointcloud")
for frame in sub_pc.frames():
    handle_cdr_pointcloud2(frame.timestamp_ns, frame.seq, frame.payload.bytes)
```

Stream-end signals raise typed exceptions (`StreamEndOfStream(reason)`, `StreamConnectionLost`, `StreamProtocolError(detail)`); see [`src/readme.md`](src/readme.md) for the full surface.

## Provider performance contract

The `participant_provider` callable runs **on the cluster runtime's only worker task**. It's invoked once per inbound `/auki/cluster/1.0.0` request — the wrapper acquires the GIL, calls it, converts the result, and hands the answer to the runtime. While the GIL is held, the runtime's task is blocked on the wrapper.

Brief contention (sub-100ms) is fine. Sustained contention (I/O, contended locks beyond a brief copy) measurably impacts cluster responsiveness. **Build the `ParticipantInfo` from cached state** — read your session clock value, copy a few strings, return. No HTTP calls, no database queries, no waiting on producer threads.

## Provider error handling

The wrapper distinguishes three cases coming back from the Python callable:

1. Returns a `cluster.ParticipantInfo` → the runtime sends it to the requester.
2. Returns Python `None` → the runtime drops the reply channel; the requester sees a request timeout. Use this to signal *transient* unavailability (session clock not yet bound, sidecar mid-startup).
3. Raises an exception OR returns anything else → wrapper logs via `tracing::warn!`, then drops the reply (same effect as `None`). Runtime stays alive; future requests still work.

Cases 2 and 3 are intentional escapes; the runtime never panics on a misbehaving provider.

## Install

The crate builds as a native Python extension via [maturin](https://www.maturin.rs/).

**Editable / development build** — clone the SDK and `maturin develop`:

```bash
git clone https://github.com/aukilabs/auki-sdk.git
cd auki-sdk/crates/auki-network-py
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
python -c "import auki_network; print(dir(auki_network.cluster))"
```

**Pip-from-Git (subdirectory)** — pin a tag (the crate has been in tagged releases since v0.0.14):

```bash
pip install "git+https://github.com/aukilabs/auki-sdk.git@v0.0.22#subdirectory=crates/auki-network-py"
```

A Rust toolchain (`rustup`) and a working C compiler are required at install time. PyPI publication is parked — same status as `auki-identity-py`.

## What this is *not*

- **Not a discovery service.** Peers come from the static `cluster.json`; no DHT, no gossip.
- **Not a session manager.** The session clock and `session_id` story belongs to the consumer; the wrapper just plumbs whatever the provider returns.
- **Not the `cluster_joined_at_ns` setter.** That field on the *local* `ParticipantInfo` is the consumer's responsibility — read `runtime.peers()`, set on outbound info when it first goes non-empty.
- **Not a trust-extender.** Inbound from peers not in `cluster.json` is dropped silently by the underlying `auki-network` runtime — no Python-visible event.
- **Not WASM.** Native Python extension; `auki-network`'s WASM-friendly subset stays in Rust.
- **Not async.** Synchronous Python API. The tokio runtime is internal; consumers don't `await` anything.

## Why per-component naming

This crate is `auki-network-py`, not part of an umbrella `auki-py`. Per-component matches the existing [`auki-identity-py`](../auki-identity-py) pattern. Future bindings (`auki-logs-py`, `auki-session-py`, `auki-registry-py`) will retire the Python sidecar's reimplementation of SDK log-writing semantics one component at a time. Decided through-Nils between SDK Claude and BoosterApp Claude on 2026-05-05.

A consumer that wants both identity and network primitives imports both:

```python
import auki_identity   # data primitives: load_or_mint_seed, Wallet, app_instance.derive
import auki_network    # network layer: cluster.spawn / load_doc / ParticipantInfo / ClusterRuntime
```

## Errors

| Function | Raises | When |
|---|---|---|
| `cluster.load_doc(path)` | `OSError` | Filesystem error reading the file. |
| `cluster.load_doc(path)` | `ValueError` | JSON syntax error, missing required field, unsupported schema version, invalid `peer_id` string, invalid multiaddr string. The error message names the offending value where applicable. |
| `cluster.ParticipantInfo(...)` | `ValueError` | `peer_id` does not parse as a libp2p PeerId. |
| `cluster.spawn(seed, ..., listen_addresses=..., ...)` | `ValueError` | `len(seed) != 32`, or any string in `listen_addresses` does not parse as a multiaddr. |
| `cluster.spawn(...)` | `RuntimeError` | Underlying swarm build failed (transport stack assembly, `listen_on` rejection). |
| `runtime.peers()` (post-shutdown) | `RuntimeError` | The runtime has been shut down. |
| `runtime.shutdown()` (called twice) | `RuntimeError` | The runtime has already been shut down. |

## Logging

Rust `tracing` events go to stderr by default — systemd → journald captures them on the K1, same pattern as `auki-identity-py`. Filter via `RUST_LOG`:

```bash
RUST_LOG=warn               # default — quiet healthy cluster
RUST_LOG=info               # peer connect/disconnect events
RUST_LOG=auki_network=debug # full cluster runtime trace
```

Routing into Python's `logging` module (so consumers can format alongside their own logs) is a follow-up — see [`parking_lot.md`](parking_lot.md). Defer until a real ask.

## Tests

Two layers, both green:

```bash
# Rust-side (links a real Python interpreter via pyo3's auto-initialize
# dev-feature; doesn't need maturin):
cargo test -p auki-network-py
# → 40 passed (cluster + grimsby + Vinland Batch 2 discovery + Dagaz Batch 2 PointCloud)

# Python-side end-to-end (needs maturin):
maturin develop
pytest python_tests/
# → 44 passed, 7 skipped (51 with DISCOVERY_BIN=/path/to/discovery set)
```

The Python suite includes a **two-runtime discovery test** — two `cluster.spawn` instances against a stub `cluster.json` listing both peers' fixed loopback addresses, polled until both see each other. Cross-language analog of `auki_network::cluster_runtime::tests::two_runtimes_discover_each_other_via_cluster_doc`.

## Versioning

Pre-1.0 alongside the rest of the SDK. Wire-shape contract pinned in [`auki_network::participant::ParticipantInfo`](../auki-network/src/participant.rs); Python API surface pinned via the `test_module_exposes_only_documented_apis` Python test. New surface requires a deliberate decision and a changelog bump.

## Implementation status

See [`src/readme.md`](src/readme.md) for the per-file map and [`src/sprint.md`](src/sprint.md) for what's queued.
