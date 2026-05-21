# Clean Retained Stream Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an SDK-owned producer source path so Python apps accept retained log streams with `Log.stream_source(...)` and `StreamDecision.accept_source(source)`.

**Architecture:** `auki-logs-py` creates a first-class retained stream source object that owns log path, stream manifest metadata, and payload kind. `auki-network-py` consumes that source through a named PyCapsule bridge, decodes retained log bytes into the typed stream payload internally, and maps to the existing runtime dispatch arms as implementation detail. The public Python app surface uses `CameraFrame`, `PointCloudFrame`, `JointEncodersFrame`, and `AudioFrame`; no retained-log entry alias is added.

**Tech Stack:** Rust, PyO3, auki-logs segmented tail iterator, prost payload decoding, auki-network stream runtime, pytest.

---

### Task 1: Add Retained Source Python Shape

**Files:**
- Modify: `crates/auki-logs-py/python_tests/test_logs.py`
- Modify: `crates/auki-logs-py/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add tests proving `Log.stream_source(...)` exists, carries `payload_kind="camera"` metadata, builds manifest fields, and exposes an SDK-internal named capsule without exposing stream factories to apps.

- [ ] **Step 2: Run red test**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-logs-py/python_tests/test_logs.py -q`
Expected: FAIL because `Log.stream_source` is missing.

- [ ] **Step 3: Implement minimal retained source object**

Add `StreamSource` pyclass in `auki-logs-py` with read-only metadata getters and `Log.stream_source(...)`. Store source path and metadata in Rust, validate `payload_kind` is one of `camera`, `pointcloud`, `joint_encoders`, or `audio`, and create a named PyCapsule bridge for `auki-network-py`.

- [ ] **Step 4: Run green test**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-logs-py/python_tests/test_logs.py -q`
Expected: PASS for the new source-shape tests.

### Task 2: Add Generic Producer Accept API

**Files:**
- Modify: `crates/auki-network-py/python_tests/test_streams.py`
- Modify: `crates/auki-network-py/src/stream_types.rs`
- Modify: `crates/auki-network-py/Cargo.toml`

- [ ] **Step 1: Write failing tests**

Add tests proving `StreamDecision.accept_source(source)` exists, returns `kind == "accept_camera"` for a camera source, and keeps legacy retained-log entry aliases absent from the Python surface.

- [ ] **Step 2: Run red test**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-network-py/python_tests/test_streams.py -q`
Expected: FAIL because `accept_source` is missing.

- [ ] **Step 3: Implement accept_source bridge**

Depend on `auki-logs-py` rlib, unwrap the retained source capsule by exact name, build `StreamManifest` from source metadata, and dispatch internally to `AcceptCamera`, `AcceptPointCloud`, `AcceptJointEncoders`, or `AcceptAudio`.

- [ ] **Step 4: Run green test**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-network-py/python_tests/test_streams.py -q`
Expected: PASS for the generic producer accept tests.

### Task 3: Decode Retained Log Bytes Internally

**Files:**
- Modify: `crates/auki-network-py/src/stream_types.rs`
- Modify: `crates/auki-network-py/python_tests/test_streams.py`

- [ ] **Step 1: Write failing tests**

Add tests that append retained log bytes for camera, pointcloud, audio, and joint encoders, call `accept_source`, and prove no app code constructs manifests, decodes log bytes, or calls typed accept factories.

- [ ] **Step 2: Run red tests**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-network-py/python_tests/test_streams.py -q`
Expected: FAIL where decoding/lifecycle is not implemented.

- [ ] **Step 3: Implement retained source streams**

Use `auki_logs::Log::<RawBytes>::tail` from source path, decode each retained payload with prost according to `payload_kind`, map disk/wire field names where needed (`PointCloudLogEntry.data` to `PointCloudFrame.bytes`), and yield typed `SourceStream<T>` items. Use the SDK-owned source lifecycle; apps only hold the source object.

- [ ] **Step 4: Run green tests**

Run: `crates/auki-network-py/.venv/bin/python -m pytest crates/auki-network-py/python_tests/test_streams.py -q`
Expected: PASS for all retained-source tests.

### Task 4: Document And Verify

**Files:**
- Modify: `crates/auki-logs-py/README.md`
- Modify: `crates/auki-logs-py/src/readme.md`
- Modify: `crates/auki-logs-py/changelog.md`
- Modify: `crates/auki-network-py/README.md`
- Modify: `crates/auki-network-py/src/readme.md`
- Modify: `crates/auki-network-py/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update docs**

Document `Log.stream_source(...)` and `StreamDecision.accept_source(source)` as the recommended producer API. Remove typed accept factories from app-facing examples or mark them SDK-internal legacy.

- [ ] **Step 2: Run verification**

Run:
`cargo check -p auki-logs-py -p auki-network-py -p auki-domain-py`
`crates/auki-network-py/.venv/bin/python -m pytest crates/auki-logs-py/python_tests/test_logs.py crates/auki-network-py/python_tests/test_streams.py -q`
`git diff --check`

Expected: all pass.
