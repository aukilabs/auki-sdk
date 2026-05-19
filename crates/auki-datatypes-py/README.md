# auki-datatypes-py

Python betterproto bindings for [`auki-datatypes`](../auki-datatypes) — dataclass-shaped types for the Auki SDK's shared cross-language data types. Lets a Python consumer encode / decode every log payload and stream wire shape that ships in the SDK without hand-rolling prost.

Filed as [Step 9 of the `auki-datatypes` migration sprint](../auki-datatypes/src/sprint.md). Closes the typed-message half of [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #4 — the bytes-level half [shipped in `auki-logs-py`](../auki-logs-py).

## Surface

```python
import auki_datatypes as adt

# Step 1 — Pinhole camera log entry
frame = adt.camera.PinholeCameraLogEntry(
    dynamic_intrinsics=adt.camera.DynamicIntrinsics(fx=1234.5, fy=1234.5, cx=272.0, cy=244.0, distortion_coefficients=[]),
    frame=jpeg_bytes,
)
on_disk = bytes(frame)
decoded = adt.camera.PinholeCameraLogEntry().parse(on_disk)

# Step 8 — Detection log entry (the ESL detector's output type)
det = adt.detection.DetectionLogEntry(data=esl_payload_bytes)
on_disk = bytes(det)

# Step 5 — Pose
st = adt.pose.SpatialTransform(
    translation=adt.pose.Vec3(x=1.0, y=2.0, z=3.0),
    orientation=adt.pose.Quat(x=0.0, y=0.0, z=0.0, w=1.0),
)
```

Every Rust `auki_datatypes::<name>::<Type>` has a matching `auki_datatypes.<name>.<Type>` in Python. The submodules are: `audio`, `camera`, `detection`, `frame_stream`, `joint_encoders`, `joint_encoders_stream`, `point_cloud`, `pose`, `stream`, `time_transform`. Pointcloud samples use `adt.point_cloud.PointCloudFrame(point_count=..., data=...)`; the former separate stream-only pointcloud package is gone.

## Cross-language byte equality

Each `bytes(message)` call produces wire bytes byte-identical to what Rust's `prost::Message::encode_to_vec` produces for the same input. **Verified by [`tests/test_locked_vectors.py`](tests/test_locked_vectors.py)** — every locked vector from `auki-datatypes/src/lib.rs` has a matching Python test that pins the same hex bytes. If betterproto's encoder ever drifts from prost's, those tests trip immediately.

This is the property that makes `auki-logs-py` + `auki-datatypes-py` a real cross-language SDK rather than two separate-but-similar libraries — a Rust producer's segment files are byte-for-byte readable from Python, and vice versa.

## End-to-end with `auki-logs-py`

```python
import auki_datatypes as adt
import auki_layout
import auki_logs
import auki_manifests

# Caller (Park / Boosterapp) sets up:
session = auki_layout.session_root("/app", session_id)
output_path = auki_layout.detection_log_path(session, "aukilabs/esl/v1", input_log_id)
output_manifest = auki_manifests.build_detection_log_manifest(...)
output = auki_logs.Log.open(output_path, output_manifest)

# Detector loop:
input_path = auki_layout.sensorlog_path(session, input_log_id)
for entry in auki_logs.Log.tail(input_path):
    # Decode the input log's PinholeCameraLogEntry from opaque bytes
    camera_frame = adt.camera.PinholeCameraLogEntry().parse(entry.payload)
    # Run the ESL detector on the JPEG bytes
    detections = run_esl(camera_frame.frame)
    # Encode the detector output as a DetectionLogEntry
    payload = bytes(adt.detection.DetectionLogEntry(data=serialize(detections)))
    output.append(entry.timestamp_ns, payload)
```

That's the entire ESL phase-2 loop in Python. Four crates (`auki-datatypes-py`, `auki-layout-py`, `auki-logs-py`, `auki-manifests-py`), every primitive native, no string templating, no dict hand-rolling, no hand-rolled prost.

## Install

For consumers:

```sh
cd crates/auki-datatypes-py
python -m venv .venv && source .venv/bin/activate
pip install -e .
pip install -e .[test] && pytest tests/  # cross-language byte-equality
```

For contributors who edit `.proto` files in [`auki-datatypes/proto/`](../auki-datatypes/proto/):

```sh
pip install -e .[regen]
brew install protobuf  # or your distro's equivalent — needs `protoc` on PATH
./regen.sh
pytest tests/                # confirm wire compat with locked Rust vectors
```

The `[regen]` extras pull in `betterproto[compiler]` and `grpcio-tools`. End consumers don't need them — only contributors regenerating the bindings.

## Why betterproto over `protobuf`

`google.protobuf` (the official Python protobuf library) generates classes that aren't dataclasses, with non-Pythonic API (set fields via `msg.field = value` but `msg.field = obj` for messages doesn't work; needs `msg.field.CopyFrom(obj)`). `betterproto` generates plain dataclasses — `Msg(field=value, nested=Nested(x=1))` works as expected, and `bytes(msg)` / `Msg().parse(bytes)` give clean serialization symmetry. The wire format is identical between the two; only the Python API changes.

Pinned to `betterproto==1.2.5`. Bumping requires re-running `regen.sh` and re-locking the cross-language vectors (the locked-vector tests trip if betterproto's encoder changes).

## Status

Crate landed 2026-05-09 — Step 9 of the [`auki-datatypes` migration sprint](../auki-datatypes/src/sprint.md). Generated files committed alongside the regen workflow. **All seven on-disk Rust locked vectors plus the JointEncoders vector match byte-for-byte in Python.**

See [`pyproject.toml`](pyproject.toml) for the package metadata, [`regen.sh`](regen.sh) for the codegen workflow, [`tests/`](tests/) for the cross-language tests.
