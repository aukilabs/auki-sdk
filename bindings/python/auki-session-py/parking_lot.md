# Parking lot — auki-session-py

Open questions for the auki-session-py crate. Cross-cutting questions that involve other crates (auki-session, auki-network, etc.) live in the [root `parking_lot.md`](../../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../../CLAUDE.md) for the workflow.

---

## Propagate: `payload: bytes` encoding contract — decided protobuf via [`auki-datatypes`](../../../crates/auki-datatypes)

Resolved 2026-05-07: segment payloads on disk are **protobuf-encoded**. The original A-vs-B framing (Python pass-through vs SDK-serialized JCS-JSON) is obsolete — protobuf via shared `.proto` files in [`auki-datatypes`](../../../crates/auki-datatypes) collapses both options into one path. `.proto` files are the canonical schema; both Rust (`prost`) and Python (`betterproto`) generate typed code from them. Cross-language conformance is enforced by locked-vector tests pinning specific message → wire-bytes pairings.

The natural split that falls out: manifests + registry entries + signing payloads stay JCS-canonical JSON via [`auki-jcs`](../../../crates/auki-jcs) (curl-debuggable, signed, hashed); segment payloads (the bulk data) become protobuf. Two encodings, no overlap on the wire — different files, different concerns.

When this crate's first implementation lands (after `auki-datatypes` migrates at least `PinholeCameraLogEntry`), propagate by: (a) wiring `betterproto` codegen through maturin's pre-build hook; (b) typing `SensorLog.append(ts_ns, entry: PinholeCameraLogEntry)` with the betterproto-generated dataclass (not raw `bytes`); (c) cross-language locked-vector test pinning that the Python encoder produces byte-identical bytes to the Rust prost encoder for a fixed input. See [`auki-datatypes/src/sprint.md`](../../../crates/auki-datatypes/src/sprint.md) for the Rust-side migration sequence this propagation depends on.

## libp2p control-plane protocols — design timing

Nils 2026-05-07: "Don't design those libp2p control protocols speculatively now. They'll come once the in-process SDK API (`auki-session-py`, in design) is settled — both transports adapt that." Translation: `/auki/control/info/0.0.1`, `/auki/control/sensor_logs/0.0.1`, etc. land **after** this crate stabilizes; the in-process API is the source-of-truth, and both transports (HTTP frozen at v0.0.23, libp2p forthcoming) become thin wrappers. Same length-prefixed-JSON framing pattern as `/auki/stream/0.1.0`. Don't draft these until this crate's [`src/readme.md`](src/readme.md) shows real implementation, not just the spec.

---

## Resolved 2026-05-07 — design walkthrough decisions (Propagate when first implementation lands)

The May 7 sketch from booster claude raised eight design questions; six are pinned. They live here as a reminder until the implementation propagates them into the relevant Rust shapes + Python module + tests.

- **`session_id` minting** — SDK mints UUIDv4; optional `session_id: Optional[str] = None` kwarg as escape hatch for deterministic / pre-known IDs (test harnesses, replay tooling). Cross-cutting Propagate task lives in [root parking_lot](../../../parking_lot.md) since it also touches `auki-session` and the `auki-registry` / `auki-time-transforms` manifest tables.
- **`start_pose_log` symmetry** — add `start_pose_log(source, clock, *, retention_ns, duration_ns) -> PoseLog` and `list_pose_logs(...)` peer to `list_sensor_logs(...)`. Pose logs are first-class.
- **`SensorLog` is single-writer, no internal locking** — one writer per log, threading is the producer's problem. SDK does NOT acquire an internal lock; concurrent `append()` from multiple threads is undefined behavior. Document in the `SensorLog` docstring.
- **`SensorLog.append` to a stopped log raises** — typed `LogStopped` exception (Python) / `Error::LogStopped` (Rust). Never silent drop.
- **`session.time_transform() -> TimeTransformLog`** — explicit handle returning the single per-session TT log. Codifies "one TT log per session" at the type level; docstring marks the enforcement as deliberate.
- **`SensorLog.append` is blocking on I/O** — producers handle backpressure via the grimsby `asyncio.Queue(maxsize=1)` idiom. No `append_nowait` variant.
