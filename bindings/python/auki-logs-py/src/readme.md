# `auki-logs-py/src/`

PyO3 bindings for `auki-logs`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). Wraps the Rust `Log<T>` framing primitive monomorphized to opaque bytes (`Log<RawBytes>`), exposing it as a Python `auki_logs` extension module via PyO3. It also exposes `StreamSource`, an SDK-owned retained-source metadata object consumed by `auki-network-py`.

## Public surface

```python
class Entry:
    timestamp_ns: int  # read-only getter
    payload: bytes     # read-only getter

class LogReader:
    def manifest(self) -> dict
    def segment_starts(self) -> list[int]
    def entries(self) -> list[Entry]

class TailIter:
    def with_poll_interval(self, ms: int) -> TailIter
    def try_next(self) -> Optional[Entry]
    # __iter__ / __next__ — blocking iterator protocol

class Log:
    @staticmethod
    def open(root: str, manifest: dict) -> Log
    def append(self, timestamp_ns: int, payload: bytes) -> None
    def flush(self) -> None
    def set_retention(self, retention_ns: int) -> None
    def manifest(self) -> dict
    def stream_source(
        self,
        *,
        sensor_id: str,
        sensor_hash: str,
        clock_id: str,
        clock_hash: str,
        payload_kind: str,
        frame_id: Optional[str] = None,
        frame_hash: Optional[str] = None,
    ) -> StreamSource
    def close(self) -> None
    # __enter__ / __exit__ — context-manager protocol
    @staticmethod
    def read(root: str) -> LogReader
    @staticmethod
    def tail(root: str) -> TailIter

class StreamSource:
    root: str
    sensor_id: str
    sensor_hash: str
    clock_id: str
    clock_hash: str
    payload_kind: str
    frame_id: str
    frame_hash: str
```

## Encoding stance

**Opaque bytes only.** The Python surface doesn't expose `LogPayload`; the framing primitive stays out of the encoder's way (mirrors the Rust crate's stance). Internally, `lib.rs` uses a tiny `RawBytes(Vec<u8>)` newtype with an identity-encoding `LogPayload` impl to satisfy the generic `Log<T>` bound.

This means a Python writer's bytes land on disk byte-for-byte the same as a Rust writer's bytes — cross-language byte equality is determined by what the producer hands to `append()`, not by any wrapper logic.

## Retained stream source bridge

`Log.stream_source(...)` creates a frozen `StreamSource` object with log path, stream manifest metadata, and a `payload_kind` discriminator (`camera`, `pointcloud`, `joint_encoders`, or `audio`). Apps pass this object directly to `auki_network.cluster.StreamDecision.accept_source(source)`.

The object exposes read-only metadata for inspection and an underscore-prefixed `_stream_source_capsule()` method for the sibling PyO3 bridge. The capsule name is `auki_logs_py::stream_source::v1`; `auki-network-py` validates that exact name before unboxing the Rust `RetainedStreamSource` payload.

The logs binding still does not decode per-frame bytes. It owns the retained source identity; `auki-network-py` owns payload-kind dispatch, retained-log tailing, and prost decoding into stream runtime frames.

## Manifest seam

The manifest is a Python `dict` on the Python side and `serde_json::Value` on the Rust side. The seam round-trips via Python's stdlib `json` module: dict → `json.dumps(...)` → `serde_json::from_str(...)` on entry, and the reverse on exit. Avoids hand-coding a pydict-to-serde-json walker; the cost is one Python call per `open` / `manifest()` (negligible for ~1KB manifests).

## Errors

| Rust error | Python exception |
|---|---|
| `Error::Io(_)` | `OSError` |
| `Error::Payload(_)` | `ValueError` (prefix `payload:`) |
| `Error::Manifest(_)` | `ValueError` (prefix `manifest:`) |
| `Error::Format(_)` | `ValueError` (prefix `format:`) |

Methods called on a closed log raise `RuntimeError`. Methods called on a consumed `TailIter` raise `RuntimeError` (defensive — currently unreachable since the iterator never voluntarily releases its inner state).

## GIL handling

Long-blocking calls (`Log.open`, `append`, `flush`, `read`, `entries`, `tail.next` blocking) all release the GIL via `Python::allow_threads(...)`. Non-blocking calls (`try_next`, getters, `manifest`) hold the GIL — they're cheap.

## Tests

### Rust-side (`cargo test -p auki-logs-py`, 3 tests)

| Test | Asserts |
|------|---------|
| `raw_bytes_round_trips_through_log` | Two appends + read returns same bytes (identity-encoding `LogPayload` impl is correct). |
| `raw_bytes_empty_payload_round_trips` | Empty payload survives the round-trip. |
| `raw_bytes_can_read_what_prost_wrote` | Stub — full cross-encoder seam test deferred (would need a circular path-dep through `auki-datatypes`). |

### Python-side (`pytest python_tests/`, 17 tests)

Surface tests, retained-source metadata/capsule tests, round-trip tests, tail tests, and an end-to-end "fake detector loop" smoke test. See [`python_tests/test_logs.py`](../python_tests/test_logs.py) for the full set.

## Consumers in this workspace

- [`detectors`](https://github.com/aukilabs/detectors) (downstream) — phase-2 ESL detector tails an input sensor log and appends to an output detection log via this binding. Closes blocker #4 at the bytes level.

## Out of scope

- **`betterproto`-generated `auki-datatypes` Python types.** Tracked as Step 9 of the [`auki-datatypes` migration sprint](../../../../crates/auki-datatypes/src/sprint.md). Until those land, Python detector authors hand-roll prost or use a custom encoder.
- **Higher-level `Session` abstraction** — lives in [`auki-session-py`](../../auki-session-py), which wraps the future Rust `auki-session` runtime crate (separate from the existing layout-only `auki-layout`).
- **Path + manifest helpers in Python** — currently Rust-only ([`auki-layout`](../../../../crates/auki-layout) and [`auki-manifests`](../../../../crates/auki-manifests)). Companion `auki-layout-py` + `auki-manifests-py` crates are filed as a follow-on PR; for v1 Python users hand-construct the path string and manifest dict.
