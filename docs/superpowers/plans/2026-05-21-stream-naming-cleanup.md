# Stream Naming Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the SDK stream, log-payload, and sensor registry vocabulary to `CameraFrame`, `DetectionFrame`, and `Camera` with no compatibility aliases or legacy tags.

**Architecture:** The proto payload schemas remain structurally identical but their message names become the public domain names. Registry JSON uses `"camera"` via `SensorBody::Camera` and the `Camera` registry body. Network and Python binding stream APIs expose only the new names while keeping the existing dispatch variant names.

**Tech Stack:** Rust workspace crates (`auki-datatypes`, `auki-registry`, `auki-network`, `auki-domain`, `auki-ros-adapter`), prost-generated protobuf code, PyO3 Python bindings, committed pure-Python generated datatypes, Cargo tests, pytest where available.

---

## File Structure

- `crates/auki-datatypes/proto/camera.proto`: ensure the public camera stream/log payload is `CameraFrame`.
- `crates/auki-datatypes/proto/detection.proto`: ensure the public detection stream/log payload is `DetectionFrame`.
- `crates/auki-datatypes/src/lib.rs`: update generated-module references, log-payload impls, locked vector tests.
- `bindings/python/auki-datatypes-py/auki_datatypes/auki/{camera,detection}.py`: update generated class names or regenerate.
- `crates/auki-registry/src/lib.rs`: use the `Camera` struct and `SensorBody::Camera`; update locked JSON tag to `"camera"`.
- `bindings/python/auki-registry-py/src/lib.rs`: expose `Camera` / `"camera"` and no legacy RGB-camera registry name.
- `crates/auki-network/src/{stream_protocol,stream_runtime,sensors_protocol,resources_protocol}.rs`: expose `CameraFrame` and `DetectionFrame`; update tests and docs.
- `bindings/python/auki-network-py/src/{lib,stream_types}.rs` plus `python_tests`: remove old Rust names and expose only `CameraFrame`.
- `crates/auki-domain/src/{cluster_manager,stream_manifest}.rs`: adjust sensor-kind and registry-body matching to `"camera"` / `SensorBody::Camera`.
- `crates/auki-ros-adapter/src/lib.rs`: builders return/register `CameraFrame` and `Camera`.
- Active `README.md`, `src/readme.md`, `src/sprint.md`, and component docs: replace current vocabulary; leave append-only changelog history intact.
- Park branch after SDK commit/tag/rev is usable: update Park imports and UI routing to `CameraFrame` / `"camera"` and remove legacy camera names/tags.

## Task 1: Datatypes Proto Payload Renames

**Files:**
- Modify: `crates/auki-datatypes/proto/camera.proto`
- Modify: `crates/auki-datatypes/proto/detection.proto`
- Modify: `crates/auki-datatypes/src/lib.rs`
- Modify: `bindings/python/auki-datatypes-py/auki_datatypes/auki/camera.py`
- Modify: `bindings/python/auki-datatypes-py/auki_datatypes/auki/detection.py`

- [ ] **Step 1: Write failing Rust API tests**

Add or rename tests in `crates/auki-datatypes/src/lib.rs` so they construct `CameraFrame` and `DetectionFrame` directly:

```rust
use super::camera::{CameraFrame, DynamicIntrinsics};
use super::detection::DetectionFrame;

#[test]
fn camera_frame_round_trips() {
    let entry = CameraFrame {
        dynamic_intrinsics: None,
        frame: vec![0xff, 0xd8, 0xff],
    };
    let bytes = entry.encode_to_vec();
    let decoded = CameraFrame::decode(&*bytes).expect("decode");
    assert_eq!(decoded, entry);
}

#[test]
fn detection_frame_round_trips() {
    let entry = DetectionFrame {
        data: vec![1, 2, 3],
        sensor_hash: "sensor-hash".into(),
        r#type: "portal".into(),
    };
    let bytes = entry.encode_to_vec();
    let decoded = DetectionFrame::decode(&*bytes).expect("decode");
    assert_eq!(decoded, entry);
}
```

- [ ] **Step 2: Run the focused datatypes test and verify failure**

Run: `cargo test -p auki-datatypes camera_frame detection_frame -- --nocapture`

Expected: fail before implementation because `CameraFrame` / `DetectionFrame` are not generated yet.

- [ ] **Step 3: Rename proto message names and Rust references**

Ensure `camera.proto` defines `message CameraFrame` and `detection.proto` defines `message DetectionFrame`, then update comments, field ledgers, `src/lib.rs` imports, `impl_log_payload!`, helper names, and tests to use those names everywhere.

- [ ] **Step 4: Update committed Python datatypes**

Update the generated Python classes so `auki_datatypes.auki.camera.CameraFrame` and `auki_datatypes.auki.detection.DetectionFrame` are the exported names. Update `bindings/python/auki-datatypes-py/tests/test_locked_vectors.py` to import and instantiate the new names.

- [ ] **Step 5: Run datatypes verification**

Run: `cargo test -p auki-datatypes`

Expected: all datatypes tests pass with updated locked names and unchanged wire-byte expectations except for class/type names.

## Task 2: Registry Camera Rename

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`
- Modify: `crates/auki-registry/{README.md,changelog.md,src/readme.md}`
- Modify: `bindings/python/auki-registry-py/src/lib.rs`

- [ ] **Step 1: Write failing registry vocabulary test**

Update the locked sensor JSON test to expect:

```json
{"color_space":"BT.709","distortion_model":"plumb_bob","frame_hash":"e0d40e7b526e04f15f83f75897f53825","frame_id":"K1-AABBCCDDEEFF/head_left_cam_optical","frame_rate_hz":20,"height":488,"intrinsics_model":"pinhole","pixel_format":"YUV_NV12","sensor_id":"K1-AABBCCDDEEFF/head_left_cam","type":"camera","width":544}
```

Update Rust construction sites to use `SensorBody::Camera(Camera { ... })`.

- [ ] **Step 2: Run focused registry test and verify failure**

Run: `cargo test -p auki-registry sensor_registry_entry_json_is_locked -- --nocapture`

Expected: fail before implementation because serialization still emits the legacy RGB-camera registry tag.

- [ ] **Step 3: Rename registry types and JSON tag**

Use `Camera` and `SensorBody::Camera` only. Keep `#[serde(tag = "type", rename_all = "snake_case")]`; the variant should serialize as `"camera"` without a compatibility override.

- [ ] **Step 4: Update Python registry binding surface**

Expose `Camera` and accept/return `"camera"` in `auki-registry-py`. Remove legacy RGB-camera names from current public docs/tests.

- [ ] **Step 5: Run registry verification**

Run: `cargo test -p auki-registry`

Expected: all registry tests pass and locked JSON/hash fixtures intentionally update to the new tag.

## Task 3: Network Stream API Rename

**Files:**
- Modify: `crates/auki-network/src/stream_protocol.rs`
- Modify: `crates/auki-network/src/stream_runtime.rs`
- Modify: `crates/auki-network/src/{sensors_protocol,resources_protocol}.rs`
- Modify: `crates/auki-network/src/readme.md`
- Modify: `bindings/python/auki-network-py/src/{lib.rs,stream_types.rs,readme.md,sprint.md}`
- Modify: `bindings/python/auki-network-py/python_tests/{test_basic.py,test_streams.py}`

- [ ] **Step 1: Write failing network vocabulary tests**

Update stream-protocol tests to use `CameraFrame` and `DetectionFrame`; update Python tests to assert `hasattr(cluster, "CameraFrame")` and that legacy camera payload aliases are absent.

- [ ] **Step 2: Run focused network test and verify failure**

Run: `cargo test -p auki-network stream_protocol::tests::camera_frame --features swarm -- --nocapture`

Expected: fail before implementation because `CameraFrame` is not exported by `stream_protocol`.

- [ ] **Step 3: Rename stream re-exports and dispatch internals**

`stream_protocol` should re-export `CameraFrame`, `DetectionFrame`, `PointCloudFrame`, `JointEncodersFrame`, and `AudioFrame`. `stream_runtime::StreamDispatch::AcceptCamera` uses `SourceStream<CameraFrame>`. `AcceptDetection` uses `SourceStream<DetectionFrame>`.

- [ ] **Step 4: Update sensor/resource catalog strings**

Use `"camera"` in sensor catalog docs, fixtures, and resource examples. Keep `"point_cloud"`, `"audio"`, and `"joint_encoders"` unchanged.

- [ ] **Step 5: Update Python network binding**

Rename internal Rust aliases and PyO3 classes to `CameraFrame`. Add `DetectionFrame` if detection streams are exposed through Python; otherwise update docs to make the missing Python detection stream exposure explicit as an implementation gap.

- [ ] **Step 6: Run network verification**

Run: `cargo test -p auki-network --features swarm`

Expected: all network tests pass with the new stream names.

## Task 4: Domain, ROS Adapter, Docs, And Workspace Sweep

**Files:**
- Modify: `crates/auki-domain/src/{cluster_manager.rs,stream_manifest.rs}`
- Modify: `crates/auki-ros-adapter/src/lib.rs`
- Modify: active docs listed by `rg`
- Modify: changelogs at every touched component and parent level

- [ ] **Step 1: Update domain and adapter compile surfaces**

Replace legacy camera registry names/tags and legacy log-entry payload names with `SensorBody::Camera`, `"camera"`, `CameraFrame`, and `DetectionFrame`.

- [ ] **Step 2: Run compile-focused tests**

Run: `cargo test -p auki-domain -p auki-ros-adapter`

Expected: both crates compile and tests pass after upstream tasks.

- [ ] **Step 3: Update active docs**

Run the forbidden-name search from the completion checklist across active docs, crates, bindings, and examples.

Update active specs/status docs, examples, and non-history tests. Leave append-only changelog historical entries untouched.

- [ ] **Step 4: Run workspace verification**

Run: `cargo test --workspace --features swarm`

Expected: workspace tests pass, or document any feature-combination limitation and run the equivalent per-crate commands.

- [ ] **Step 5: Commit SDK implementation**

Run:

```bash
git add crates bindings README.md docs changelog.md
git commit -m "refactor: standardize stream frame naming"
```

## Task 5: Park Integration

**Files:**
- Modify in Park worktree: `Cargo.toml`, `Cargo.lock`, `README.md`, `src/README.md`, stream consumers, recordings, UI type routing docs/tests.

- [ ] **Step 1: Point Park at the fixed SDK revision**

After the SDK implementation commit is available from a Git ref, update Park's SDK dependency from the temporary v0.0.50 workaround to that revision or the next SDK tag.

- [ ] **Step 2: Update Park imports and routing**

Replace legacy camera payload names with `CameraFrame`; use `"camera"` in active UI routing and docs. Keep point cloud, audio, joint encoders, and world behavior unchanged.

- [ ] **Step 3: Run Park verification**

Run: `cargo check` and `cargo test` in the Park worktree.

Expected: Park compiles and tests pass against the renamed SDK surface.
