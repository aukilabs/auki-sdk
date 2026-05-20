# `auki-session-py/src/`

Implementation status of `auki-session-py`. Spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs). It is currently an **empty `#[pymodule]`** — the crate compiles and produces a loadable Python extension named `auki_session`, but the module exports nothing.

## What's not here yet

Everything in the [outer `README.md`](../README.md) surface section:

- `Session`, `SessionInfo`, `SensorLog`, `PoseLog`, `TimeTransformLog` — no PyClasses.
- `SensorRef` / `ClockRef` / `FrameRef` / `SensorSpec` / `ClockSpec` / `FrameSpec` — no dataclasses.
- `LogStopped` / `SchemaHashMismatch` — no exception types.
- No registry round-trip (`register_sensor` / `register_clock` / `register_frame`).
- No log open/append/stop (`start_sensor_log` / `start_pose_log`).
- No cross-session listing (`Session.list` / `list_sensor_logs` / `list_pose_logs`).
- No TimeTransform handle (`session.time_transform`).
- No tests.

First implementation gates on the [`payload` encoding contract](../parking_lot.md) decision.

## Public surface (current)

```rust
#[pymodule]
fn auki_session(_py: Python<'_>, _m: &Bound<'_, PyModule>) -> PyResult<()> { Ok(()) }
```

That's the entirety of the Python-visible surface today.

## Tests

None. `cargo test -p auki-session-py` runs zero tests; `pytest bindings/python/auki-session-py/python_tests/` has no `python_tests/` directory yet.

## Consumers

None in the workspace; none downstream. BoosterApp's existing Python sidecar will be the first consumer — once first implementation lands.
