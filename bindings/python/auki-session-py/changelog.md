# Changelog — auki-session-py

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's codex · May 20, HKT, 2026

**Package relocated to `bindings/python/auki-session-py`.** The Python package moved from `crates/auki-session-py` to `bindings/python/auki-session-py` with no package-name, module-name, or runtime behavior changes. Cargo workspace membership and local path dependencies now point at the new location.

### broodsugar's claude · May 7, 19:00 HKT, 2026

**Crate scaffolding.** New crate `auki-session-py` — transport-neutral in-process Python surface that the [HTTP Control API](../../docs/control-api.md) (frozen at v0.0.23) and the forthcoming libp2p control protocols (`/auki/control/...`) both wrap. No functional code yet; this entry lands the layout (`README.md` aspirational spec, `parking_lot.md`, `changelog.md`, `src/readme.md`, `src/sprint.md`, `src/lib.rs` empty PyO3 entry) so the six design decisions resolved during the May 7 walkthrough have a home.

**Resolved decisions encoded in [`README.md`](README.md):** `session_id` minting (SDK-mints with `Optional[str] = None` escape-hatch kwarg); `start_pose_log` / `list_pose_logs` peer to the sensor-log methods (pose logs are first-class); `SensorLog` single-writer with no internal locking (concurrent `append()` from two threads is undefined); writes to a stopped log raise typed `LogStopped`; `session.time_transform()` returns the single per-session TT log explicitly; `SensorLog.append` is blocking on I/O (producers handle backpressure via the grimsby `Queue(maxsize=1)` idiom).

**Open questions in [`parking_lot.md`](parking_lot.md):** `payload: bytes` encoding contract (which serializer becomes the cross-language wire format) and libp2p control-plane protocol design timing (Nils directive 2026-05-07: wait until this crate stabilizes). First implementation gates on the `payload` decision.

**Build skeleton:** `Cargo.toml` mirrors `auki-network-py`'s shape (cdylib + rlib, abi3-py38, `extension-module` feature gated); `pyproject.toml` for maturin; `src/lib.rs` is an empty `#[pymodule]`. `cargo check -p auki-session-py` clean. Workspace `Cargo.toml` updated to include the new member. Will land in v0.0.24 (the v0.0.23 cut is the spec rewrite; this scaffolding follows separately).
