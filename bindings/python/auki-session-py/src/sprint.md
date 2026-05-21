# Sprint — auki-session-py

Current work and the next steps to close the gap between [`src/readme.md`](readme.md) (what's implemented) and the outer [`README.md`](../README.md) (the spec).

## Now

Crate scaffolding only — empty `#[pymodule]`, no PyClasses, no registry methods, no logs. See [`src/readme.md`](readme.md) for the precise current surface.

## Blocked on

[`payload: bytes` encoding contract](../parking_lot.md) — the cross-language wire format for non-Rust producers. First implementation gates here; without a decision, `SensorLog.append` / `PoseLog.append` can't be specified end-to-end (and `auki-jcs` Python bindings may or may not be a prerequisite, depending on which option wins).

## Next, in order

1. **Resolve the `payload` encoding question.** Pick option A (Python encodes, SDK pass-through) or option B (typed Python objects, SDK serializes). See [`parking_lot.md`](../parking_lot.md). Without this, every other step is a blocked guess.

2. **Wrap `auki-session` Rust path helpers** as a sanity-check `#[pyfunction]` set — `sensor_entry_path`, `clock_entry_path`, `frame_entry_path`, `session_root`, `sensorlog_path`, `poselog_path`, `timetransform_log_path`, `id_to_segment`. Pure-function, zero state. Useful as a smoke test that the maturin / abi3-py38 build pipeline works before introducing stateful types.

3. **Stand up `Session` (the stateful handle).** First crack at `Session.open(app_root, *, app_id, app_instance, session_id=None)` minting UUIDv4, `mkdir`ing `<app_root>/<session_id>/`, exposing `session_id` / `session_root` / `app_id` / `app_instance`. No registries, no logs yet — just the session lifecycle. This requires a Rust `auki_session::Session` struct first; that lands in `auki-session` (likely as a separate change).

4. **Registries.** `register_sensor` / `register_clock` / `register_frame` returning typed refs. Wraps `auki-registry` writes via the path helpers. Test: re-registering identical content is idempotent (same hash, same file); re-registering with drift returns `SchemaHashMismatch`.

5. **Sensor + pose logs.** `start_sensor_log` / `start_pose_log` returning `SensorLog` / `PoseLog`. `append` (blocking, `LogStopped` on stopped log, no internal locking), `update`, `stop`, context-manager protocol. Tests: round-trip write/read, stopped-log raises, retention/duration behavior.

6. **Cross-session listing.** `Session.list` / `list_sensor_logs` / `list_pose_logs` walking the on-disk layout. Backs the Control API's `GET /api/sensor_logs`.

7. **TimeTransform log.** `session.time_transform()` returning `TimeTransformLog`. Wraps `auki-time` writes.

## Out-of-band

- libp2p control-plane protocols (`/auki/control/...`) — explicitly deferred until this crate stabilizes. Don't draft speculatively. See [`parking_lot.md`](../parking_lot.md).
- HTTP Control API daemon implementation — not in this crate's scope; daemons (BoosterApp, Sentinel) write their own HTTP layer that wraps this surface. Same eventual story for libp2p.
