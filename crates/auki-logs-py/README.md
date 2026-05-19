# auki-logs-py

PyO3 bindings for [`auki-logs`](../auki-logs)'s `Log<T>` framing primitive — opaque-bytes Python surface that lets Python producers and consumers participate in the SDK's segmented ring-buffer log on equal footing with Rust. It also creates SDK-owned retained stream sources that `auki-network-py` can serve with `StreamDecision.accept_source(source)`.

Filed as the **smallest viable Python binding to unblock [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4** (specifically the ESL detector, which is Python). Higher-level abstractions (`Session`, registry helpers) live elsewhere ([`auki-session-py`](../auki-session-py)) and grow independently.

## Surface

```python
import auki_logs

# 1. Open or create a write-side log.
manifest = {
    "segment_duration_ns": 1_000_000_000,
    "retention_ns": 60_000_000_000,
    "kind": "detection",
    "detector_id": "aukilabs/esl/v1",
    # ... rest of the build_detection_log_manifest fields
}
output = auki_logs.Log.open("/path/to/detection_log", manifest)

# 2. Tail an input sensor log (yields entries as they arrive).
tail = auki_logs.Log.tail("/path/to/input_sensor_log")
for entry in tail:                               # blocking iterator
    ts: int = entry.timestamp_ns
    payload: bytes = entry.payload               # prost-encoded; decode it yourself
    detections = run_esl(payload)                # your detector
    output.append(ts, serialize(detections))     # opaque bytes back

output.close()  # or use `with auki_logs.Log.open(...) as log:`
```

Retained producer sources:

```python
from auki_network import cluster

camera_log = auki_logs.Log.open("/path/to/camera_log", camera_manifest)
source = camera_log.stream_source(
    sensor_id="robot/head_left_cam",
    sensor_hash="...",
    clock_id="robot/clock",
    clock_hash="...",
    payload_kind="camera",
    frame_id="robot/head_left_cam_optical",
    frame_hash="...",
)

decision = cluster.StreamDecision.accept_source(source)
```

## Encoding stance

**Opaque bytes only.** The Python surface mirrors the Rust crate's encoder-agnostic stance — there's no Python equivalent of `auki_logs::LogPayload`. Callers pass `bytes` for `payload`; readers receive `bytes`. Decoding the prost bytes (e.g. into `auki_datatypes` types) is the Python consumer's job, typically via `betterproto`-generated dataclasses once those land — see [Step 9 of the `auki-datatypes` migration sprint](../auki-datatypes/src/sprint.md). For phase-2 detector work today, the consumer hand-rolls prost or builds a small inline encoder.

This matches the Rust `auki-logs` crate's "framing only — pick your encoder" philosophy. Drift between Rust and Python is **bytewise** at the segment-file level; the `LogPayload` choice on each side stays a per-language concern.

`Log.stream_source(...)` is the retained-stream exception to "decode it yourself": it still stores opaque bytes, but packages the log path plus sensor/clock/frame metadata into a named SDK-internal capsule. `auki-network-py` unwraps that source, tails the log, decodes retained bytes by `payload_kind`, and maps them to the stream runtime internally.

## Surface — full reference

| Class / method | Notes |
|---|---|
| `Log.open(root: str, manifest: dict) -> Log` | Open or create a log directory. Manifest dict is JCS-canonicalized at write time; required keys: `segment_duration_ns` (> 0), `retention_ns` (≥ 0). |
| `Log.append(timestamp_ns: int, payload: bytes)` | Append an entry. Rolls segments + evicts retention-expired segments. |
| `Log.flush()` | Flush + fsync the current segment. |
| `Log.set_retention(retention_ns: int)` | Update the on-disk manifest's retention window atomically. |
| `Log.manifest() -> dict` | Inspect the active manifest. |
| `Log.stream_source(..., payload_kind: str, ...) -> StreamSource` | Create an SDK-owned retained stream source for `StreamDecision.accept_source(source)`. `payload_kind` is one of `camera`, `pointcloud`, `joint_encoders`, or `audio`. |
| `Log.close()` | Drop the writer (or use `with Log.open(...) as log:`). |
| `Log.read(root: str) -> LogReader` | Read snapshot — list segments and load entries. |
| `LogReader.manifest() -> dict` / `segment_starts() -> list[int]` / `entries() -> list[Entry]` | Read-side introspection. |
| `Log.tail(root: str) -> TailIter` | Yield newly-appended entries as they become readable. Starts at current EOF; tails forever (drop the iterator to stop). |
| `TailIter.with_poll_interval(ms: int) -> TailIter` | Override the poll cadence (default 10ms). Builder style. |
| `TailIter.try_next() -> Optional[Entry]` | Non-blocking — None if no entry ready. |
| `for entry in tail: ...` | Blocking — yields entries at the configured poll cadence. |
| `Entry.timestamp_ns: int` / `Entry.payload: bytes` | Read-only getters. |
| `StreamSource` | Read-only metadata object produced by `Log.stream_source(...)`; apps pass it to `auki_network.cluster.StreamDecision.accept_source(source)`. |

## Errors

| Rust error | Python exception |
|---|---|
| `Error::Io(_)` | `OSError` |
| `Error::Payload(_)` | `ValueError` (prefixed `payload:`) |
| `Error::Manifest(_)` | `ValueError` (prefixed `manifest:`) |
| `Error::Format(_)` | `ValueError` (prefixed `format:`) |

Calling a method on a closed `Log` raises `RuntimeError`.

## Install

```sh
cd crates/auki-logs-py
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest python_tests/
```

`maturin develop` installs the extension into the venv as `auki_logs`. The build is `abi3-py38` so a single wheel works on Python 3.8+.

## Versioning

The Python surface tracks the Rust `auki-logs` crate version. Changes to the Rust framing primitive that aren't backward-compatible at the on-disk level bump the SDK version; pure-additive Python wrappers don't.

## Status

**Read side, write side, tail iterator, and retained stream-source bridge are all here as of 2026-05-19.** The original 2026-05-09 bytes-level interface closes [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4. `Log.stream_source(...)` adds the producer path for retained sensor logs: apps keep writing opaque bytes, then hand the SDK-owned source to `auki-network-py` for stream serving.

See [`src/readme.md`](src/readme.md) for the implementation status and [`src/sprint.md`](src/sprint.md) for the next steps.
