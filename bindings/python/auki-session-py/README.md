# auki-session-py

Transport-neutral in-process Python surface for the Auki SDK's session lifecycle — opening sessions, registering sensors and clocks, writing and listing sensor logs and pose logs. PyO3 bindings over the `auki-session` Rust crate.

This is the **source-of-truth API** for control-plane operations. Both the [HTTP Control API](../../docs/control-api.md) (frozen at SDK release v0.0.23) and the forthcoming libp2p control protocols (`/auki/control/info/0.0.1`, `/auki/control/sensor_logs/0.0.1`, …) are thin wrappers over this surface — every consumer-facing operation maps to a method here.

## Status

Scaffolding only. No functional code yet. The shape below reflects design decisions made during the May 7, 2026 walkthrough; see [`parking_lot.md`](parking_lot.md) for the two open questions (`payload` encoding contract, libp2p control-plane design timing) that gate first implementation.

## Surface

```python
from pathlib import Path
from auki_session import Session, SensorSpec, ClockSpec

with Session.open(Path("/var/lib/boosterapp"),
                  app_id="boosterapp",
                  app_instance="k1-001") as session:
    # SDK minted a fresh UUIDv4; learn it after open() returns.
    print(session.session_id)

    # Register sensor + clock schemas. Idempotent — same content hash
    # writes to the same <hash>.json file under registries/.
    camera = session.register_sensor(SensorSpec("camera/left", {
        "data_type": "camera",
        "width": 1920, "height": 1080,
        "frame_rate_hz": 30,
        "frame_id": "K1-001/head_left_cam_optical",
        # …intrinsics…
    }))
    clock = session.session_clock  # session-monotonic; or register_clock(...)

    # Open a sensor log. retention_ns = backward window kept on disk
    # (0 = no eviction). duration_ns = forward auto-stop cap (0 = run
    # indefinitely). Same semantics as the Control API spec.
    with session.start_sensor_log(camera, clock,
                                  retention_ns=30_000_000_000,  # 30s pre-roll
                                  duration_ns=0) as log:
        for frame in subscribe_to_camera():
            log.append(frame.ts_ns, frame.payload_bytes)
            # ↑ blocks on I/O; producers handle backpressure with a
            # bounded queue on their side (grimsby pattern).
```

## API reference

### `Session`

```python
class Session:
    @classmethod
    def open(cls, app_root: Path, *,
             app_id: str,
             app_instance: str,
             session_id: Optional[str] = None) -> "Session": ...
    """Open a new session at <app_root>/<session_id>/. SDK mints a
    UUIDv4 if session_id is None; supplied session_id is validated for
    filesystem-safety and used as-is. Raises FileExistsError on UUID
    collision (extraordinarily rare; usually indicates the integrator
    passed a non-unique session_id)."""

    @classmethod
    def list(cls, app_root: Path, *,
             started_after_ns: Optional[int] = None,
             started_before_ns: Optional[int] = None) -> Iterator["SessionInfo"]: ...
    """Enumerate every session directory under app_root. Read-only — does
    not require the daemon to have a live session of its own."""

    @property
    def session_id(self) -> str: ...
    @property
    def app_id(self) -> str: ...
    @property
    def app_instance(self) -> str: ...
    @property
    def session_clock(self) -> "ClockRef": ...
    @property
    def session_root(self) -> Path: ...

    # Registries — content-addressed; idempotent across sessions of the
    # same app. Returns a ref the integrator threads into log opens.
    def register_sensor(self, spec: "SensorSpec") -> "SensorRef": ...
    def register_clock(self, spec: "ClockSpec") -> "ClockRef": ...
    def register_frame(self, spec: "FrameSpec") -> "FrameRef": ...

    # Logs — start_*  is the only way to open one; list_*  spans every
    # session on disk by default, filterable down to one. Cross-session
    # listing is what the Control API's `GET /api/sensor_logs` wraps.
    def start_sensor_log(self, sensor: "SensorRef", clock: "ClockRef", *,
                         retention_ns: int = 0,
                         duration_ns: int = 0) -> "SensorLog": ...
    def start_pose_log(self, source: "PoseSource", clock: "ClockRef", *,
                       retention_ns: int = 0,
                       duration_ns: int = 0) -> "PoseLog": ...
    def list_sensor_logs(self, *,
                         session_id: Optional[str] = None,
                         sensor: Optional["SensorRef"] = None,
                         clock: Optional["ClockRef"] = None,
                         started_after_ns: Optional[int] = None,
                         started_before_ns: Optional[int] = None) -> Iterator["SensorLogInfo"]: ...
    def list_pose_logs(self, **filters) -> Iterator["PoseLogInfo"]: ...
    def get_sensor_log(self, sensor_log_id: str) -> "SensorLogInfo": ...

    # TimeTransform log — explicit handle, returning the single
    # per-session TT log. Codifies "one TT log per session" at the type
    # level (intentional, not an oversight).
    def time_transform(self) -> "TimeTransformLog": ...

    def __enter__(self) -> "Session": ...
    def __exit__(self, *exc): ...  # flushes + closes every open log
```

### `SensorLog` / `PoseLog`

Same shape, separate types so you can't pass a `PoseLog` where a `SensorLog` is expected.

```python
class SensorLog:  # PoseLog identical except `sensor` → `source`
    @property
    def sensor_log_id(self) -> str: ...
    @property
    def session_id(self) -> str: ...
    @property
    def sensor(self) -> "SensorRef": ...   # or `source: PoseSource` on PoseLog
    @property
    def clock(self) -> "ClockRef": ...
    @property
    def retention_ns(self) -> int: ...
    @property
    def duration_ns(self) -> int: ...
    @property
    def started_at_ns(self) -> int: ...
    @property
    def stopped_at_ns(self) -> Optional[int]: ...

    def append(self, ts_ns: int, payload: bytes) -> None: ...
    """One log entry. Blocking on I/O — producers needing backpressure
    use the grimsby asyncio.Queue(maxsize=1) idiom on their side. The
    SDK does NOT acquire an internal lock; if you call append() from
    two threads concurrently, behavior is undefined. One writer per
    log. Raises LogStopped if the log has been stopped (via stop(),
    __exit__, daemon shutdown, or hitting its duration_ns cap)."""

    def update(self, *, retention_ns: Optional[int] = None,
               duration_ns: Optional[int] = None) -> None: ...
    """Mutate the log's configuration in place — backs the Control API
    PATCH endpoint. Identity fields (sensor, clock, session_id) are
    immutable; mutating any of them is semantically a different log."""

    def stop(self) -> None: ...
    """Sets stopped_at_ns and closes the underlying file handles. Idempotent."""

    def __enter__(self) -> "SensorLog": ...
    def __exit__(self, *exc): ...  # auto-stops
```

### Refs and specs

```python
@dataclass(frozen=True)
class SensorRef:
    sensor_id: str
    sensor_hash: str  # XXH3-128 hex of the registry entry

@dataclass(frozen=True)
class ClockRef:
    clock_id: str
    clock_hash: str

@dataclass(frozen=True)
class FrameRef:
    frame_id: str
    frame_hash: str

@dataclass
class SensorSpec:
    sensor_id: str
    schema: dict   # JSON-serializable; SDK content-hashes it via auki-jcs

@dataclass
class ClockSpec:
    clock_id: str
    schema: dict

@dataclass
class FrameSpec:
    frame_id: str
    schema: dict   # FrameRegistryEntry shape: handedness, axes, units
```

### Exceptions

```python
class LogStopped(RuntimeError): ...
"""Raised by SensorLog.append / PoseLog.append when the log is stopped.
Never silent drop — silent drops are corruption-of-intent."""

class SchemaHashMismatch(RuntimeError): ...
"""Raised when a re-registration of an existing (sensor_id, clock_id, frame_id)
produces a different hash than the on-disk entry — i.e. schema drift."""
```

## Design decisions

These are pinned and shouldn't be revisited without explicit discussion.

| Decision | Resolution | Why |
|---|---|---|
| `session_id` minting | SDK mints UUIDv4; optional `session_id=None` kwarg as escape hatch | Centralizes the one place that has to get UUIDv4 right. Optional kwarg covers test harnesses, replay tooling. |
| Pose-log symmetry | `start_pose_log(...)` + `list_pose_logs(...)` peer to the sensor-log methods | Pose logs are first-class on disk; the in-process API mirrors that. |
| Concurrent writes to a log | One writer per log, no internal locking | Threading is the producer's problem. Mirrors Rust's `&mut self` on `Log<T>::append`. |
| Write to a stopped log | Raise `LogStopped` | Silent drops are the worst kind of corruption-of-intent. |
| TimeTransform log access | Explicit `session.time_transform()` returning a single handle | Codifies "one TT log per session" at the type level. |
| `append` blocking semantics | Blocking on I/O | Simple semantics, ergonomic. Producers needing backpressure use a bounded queue (grimsby pattern). |

## Open questions

Tracked in [`parking_lot.md`](parking_lot.md). Two items gating first implementation:

1. `payload: bytes` encoding contract — which serializer becomes the cross-language wire format for non-Rust producers.
2. libp2p control-plane protocol design timing — when to start drafting `/auki/control/...`. Directive from Nils (2026-05-07): wait until this surface settles.

## Build

```sh
maturin develop --release    # install into the active venv
cargo test -p auki-session-py
pytest bindings/python/auki-session-py/python_tests/
```

abi3-py38 wheel; works on every Python 3.8+ minor without rebuild.
