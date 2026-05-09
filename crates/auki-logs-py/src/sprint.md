# Sprint — auki-logs-py

Current work and next steps. Spec: [outer `README.md`](../README.md).

## Now

`auki-logs-py` landed 2026-05-09 to close [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4 at the bytes level. The Python surface is the smallest viable wrapper around `auki_logs::Log<T>` monomorphized to opaque bytes — `Log.open`, `append`, `flush`, `set_retention`, `read`, `tail` (blocking iterator) + `try_next` (non-blocking) all map cleanly. GIL released on every blocking call. Errors mapped to `OSError` / `ValueError` per the rest of the SDK's Python conventions.

Closes blocker #4 for ESL **at the bytes level**. The detector author hand-rolls prost or builds a small inline encoder for `DetectionLogEntry` and `PinholeCameraLogEntry` until the `betterproto` Python codegen lands.

## Next

1. **`betterproto` codegen for `auki-datatypes`** (Step 9 of the migration sprint). Lets Python consumers `from auki_datatypes import DetectionLogEntry` and skip hand-rolling prost. The detector loop becomes:
   ```python
   from auki_datatypes import DetectionLogEntry, PinholeCameraLogEntry
   for entry in auki_logs.Log.tail(input_path):
       camera_frame = PinholeCameraLogEntry().parse(entry.payload)
       detections = run_esl(camera_frame)
       output.append(entry.timestamp_ns, bytes(DetectionLogEntry(data=serialize(detections))))
   ```
   Lives in a new `auki-datatypes-py` crate per the [per-component naming decision](../../parking_lot.md). Locked-vector cross-language test: betterproto encoder produces byte-identical bytes to Rust prost for a fixed input.

2. **Companion `auki-layout-py` + `auki-manifests-py` crates.** Tiny pure-function wrappers — `detection_log_path(...)`, `build_detection_log_manifest(...)`, `sensorlog_path(...)`, etc. as `#[pyfunction]`s. Sibling slice; doesn't gate this crate. Fixes the divergence risk where Python users currently hand-construct `<session>/detection_logs/<detector_id>__<input_log_id>` and the manifest dict in case the format changes Rust-side.

3. **Type stubs** (`auki_logs.pyi`) for IDE support. Filed as a low-priority follow-up — the surface is small enough that doc strings cover most cases.

## Out-of-band

- **No `Session` here** — that lives in [`auki-session-py`](../../auki-session-py), which wraps the future Rust `auki-session` runtime crate. This crate is the framing primitive only.
- **No registry helpers here** — those live in a future `auki-registry-py`.
- **PyO3 0.22 + abi3-py38** to match `auki-network-py`'s pattern. Bumping the PyO3 version is a workspace-wide concern.
