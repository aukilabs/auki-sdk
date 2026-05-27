# #216 Schema & API Placement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the v1 schema and crate boundaries for [#216](https://github.com/aukilabs/auki-sdk/issues/216): registry entries with `peer_id`, manifests with `source_peer_id`/`writer_peer_id`, new catalog row shape, and the new `auki-session` crate hosting the declarative app API.

**Architecture:** Coordinated breaking change across `auki-registry`, `auki-manifests`, `auki-network`, `auki-domain`, plus a new `auki-session` crate. No backwards compatibility — clean cut. Protocol bump to `0.2.0` across the three libp2p endpoints. Phases land sequentially, each ending in a commit checkpoint.

**Tech Stack:** Rust 2024 workspace, JCS-canonical JSON via `auki-jcs`, content hashing via `auki-hash`, libp2p via `auki-network`, PyO3 bindings.

**Source spec:** [docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md](../specs/2026-05-27-216-schema-and-api-placement-design.md)

---

## File Structure

**Created:**
- `crates/auki-session/Cargo.toml`
- `crates/auki-session/src/lib.rs` — public surface
- `crates/auki-session/src/session.rs` — `Session` + `SessionInner`
- `crates/auki-session/src/registry_store.rs` — local registry storage helpers
- `crates/auki-session/src/log_specs.rs` — `HeadSpec`, `SensorLogSpec`, `PoseLogSpec`, etc.
- `crates/auki-session/src/log_handles.rs` — `*LogHandle` types + write/seal
- `crates/auki-session/src/materialization.rs` — `materialize_remote_log` + `MaterializedLogHandle`
- `crates/auki-session/tests/end_to_end.rs` — materialization smoke
- `bindings/python/auki-session-py/Cargo.toml`
- `bindings/python/auki-session-py/src/lib.rs`
- `bindings/python/auki-session-py/python_tests/test_session.py`

**Locked test fixtures (each as one JSON file):**
- `crates/auki-registry/tests/locked/sensor_camera.json`
- `crates/auki-registry/tests/locked/sensor_point_cloud.json`
- `crates/auki-registry/tests/locked/sensor_audio.json`
- `crates/auki-registry/tests/locked/sensor_joint_encoders.json`
- `crates/auki-registry/tests/locked/clock_monotonic.json`
- `crates/auki-registry/tests/locked/clock_utc.json`
- `crates/auki-registry/tests/locked/frame_ros_body.json`
- `crates/auki-registry/tests/locked/frame_ros_optical.json`
- `crates/auki-registry/tests/locked/frame_opengl.json`
- `crates/auki-registry/tests/locked/frame_unity.json`
- `crates/auki-registry/tests/locked/detector_object_detection.json`
- `crates/auki-manifests/tests/locked/sensor_log_origin.json`
- `crates/auki-manifests/tests/locked/sensor_log_materialized.json`
- `crates/auki-manifests/tests/locked/pose_log_rigid.json`
- `crates/auki-manifests/tests/locked/pose_log_movable.json`
- `crates/auki-manifests/tests/locked/time_transform_log.json`
- `crates/auki-manifests/tests/locked/detection_log.json`
- `crates/auki-network/tests/locked/catalog_row_live_rolling_camera.json`
- `crates/auki-network/tests/locked/catalog_row_live_fixed_pose.json`
- `crates/auki-network/tests/locked/catalog_row_sealed_camera.json`
- `crates/auki-network/tests/locked/catalog_row_sealed_one_sample_pose.json`
- `crates/auki-network/tests/locked/catalog_row_materialization.json`
- `crates/auki-network/tests/locked/stream_request.json`

**Modified:**
- `Cargo.toml` (workspace) — add `auki-session` + `auki-session-py` members
- `crates/auki-registry/src/lib.rs` — `peer_id` on entries, `RegistryRef`, `LogRef`, `SensorBody` body update, charset validation
- `crates/auki-registry/src/storage.rs` (new helper module or extend lib.rs) — disk path now includes `peer_id` segment
- `crates/auki-manifests/src/lib.rs` — source/writer split, `RegistryRef`/`LogRef` adoption in all four manifests + builders
- `crates/auki-network/src/resources_protocol.rs` — new `ResourceEntry`, `Kind` closed enum, `Head`/`Extent`/`Available` blocks, drop the three legacy resource structs
- `crates/auki-network/src/stream_protocol.rs` — new `StreamRequest` (source_peer_id + resource_id + ReadFrom), drop legacy fields
- `crates/auki-network/src/lib.rs` — `RESOURCES_PROTOCOL` / `REGISTRIES_PROTOCOL` / `STREAM_PROTOCOL` constants → `0.2.0`
- `crates/auki-domain/src/lib.rs` — re-orient as internal: stop re-exporting wire types as app surface; remove `stream_manifest` public-builder
- `crates/auki-domain/src/cluster_manager.rs` — take a `SessionHandle` for catalog production
- `crates/auki-logs/src/lib.rs` — `HeadSpec`-aware `Log::new_with_head` constructor
- `bindings/python/auki-registry-py/src/lib.rs` — mirror new shapes
- `bindings/python/auki-manifests-py/src/lib.rs` — mirror new shapes
- `bindings/python/auki-domain-py/src/lib.rs` — deprecate / re-route through `auki-session-py`

---

## Phase 1 — auki-registry foundation

Add `peer_id` to the four entry types, introduce `RegistryRef` and `LogRef` shared types, switch `SensorBody`'s nested frame ref to `RegistryRef`, update on-disk paths, regenerate locked fixtures.

### Task 1.1: Add `RegistryRef` and `LogRef` shared types

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`
- Test: `crates/auki-registry/src/lib.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add at the bottom of `crates/auki-registry/src/lib.rs`:

```rust
#[cfg(test)]
mod ref_tests {
    use super::*;
    use auki_identity::PeerId;

    #[test]
    fn registry_ref_round_trips_canonical_json() {
        let r = RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "head_left_rgb".to_string(),
            hash: "abc123".to_string(),
        };
        let json = auki_jcs::to_canonical_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"hash":"abc123","id":"head_left_rgb","peer_id":"galbot"}"#
        );
        let r2: RegistryRef = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn log_ref_round_trips_canonical_json() {
        let r = LogRef {
            source_peer_id: PeerId::from_string("galbot").unwrap(),
            resource_id: "head_left_rgb".to_string(),
        };
        let json = auki_jcs::to_canonical_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"resource_id":"head_left_rgb","source_peer_id":"galbot"}"#
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-registry ref_tests
```

Expected: compile error — `RegistryRef`/`LogRef` not defined.

- [ ] **Step 3: Add the types**

In `crates/auki-registry/src/lib.rs`, add (above the existing `SensorRegistryEntry` block):

```rust
use auki_identity::PeerId;

/// Reference to a registry entry by (peer_id, id, content hash).
/// Used wherever one registry record points at another or a manifest
/// points at a registry entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RegistryRef {
    pub peer_id: PeerId,
    pub id: String,
    pub hash: String,
}

/// Reference to a log by (source_peer_id, resource_id). Logs are not
/// content-addressed by a single hash — their manifests may differ
/// across materializing peers — so this carries only the canonical
/// identity tuple.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogRef {
    pub source_peer_id: PeerId,
    pub resource_id: String,
}
```

Make sure `auki-identity` is in `crates/auki-registry/Cargo.toml` `[dependencies]` (it likely already is — verify).

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p auki-registry ref_tests
```

Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-registry/src/lib.rs crates/auki-registry/Cargo.toml
git commit -m "feat(auki-registry): add RegistryRef and LogRef shared types"
```

### Task 1.2: Add `peer_id` to `SensorRegistryEntry`, switch frame refs to `RegistryRef`

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`

- [ ] **Step 1: Write failing test**

Add to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn sensor_entry_camera_canonical_has_peer_id_first() {
    let entry = SensorRegistryEntry {
        peer_id: PeerId::from_string("galbot").unwrap(),
        sensor_id: "head_left_rgb".to_string(),
        body: SensorBody::Camera(Camera {
            width: 1920,
            height: 1200,
            frame_rate_hz: 30,
            pixel_format: "rgb8".to_string(),
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            frame: RegistryRef {
                peer_id: PeerId::from_string("galbot").unwrap(),
                id: "head_left_camera_optical".to_string(),
                hash: "framehash".to_string(),
            },
        }),
    };
    let json = auki_jcs::to_canonical_string(&entry).unwrap();
    // JCS sorts alphabetically — body, peer_id, sensor_id at the top
    assert!(json.starts_with(r#"{"body":"#));
    assert!(json.contains(r#""peer_id":"galbot""#));
    assert!(json.contains(r#""sensor_id":"head_left_rgb""#));
    // Camera body's frame is now a RegistryRef
    assert!(json.contains(r#""frame":{"hash":"framehash","id":"head_left_camera_optical","peer_id":"galbot"}"#));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-registry sensor_entry_camera_canonical_has_peer_id_first
```

Expected: compile error — `SensorRegistryEntry` has no `peer_id` field and `Camera` doesn't take `frame: RegistryRef`.

- [ ] **Step 3: Update `SensorRegistryEntry` and `Camera`**

In `crates/auki-registry/src/lib.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorRegistryEntry {
    pub peer_id: PeerId,
    pub sensor_id: String,
    #[serde(flatten)]
    pub body: SensorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Camera {
    pub width: u32,
    pub height: u32,
    pub frame_rate_hz: u32,
    pub pixel_format: String,
    pub color_space: String,
    pub intrinsics_model: String,
    pub distortion_model: String,
    pub frame: RegistryRef,
}
```

- [ ] **Step 4: Update `PointCloud`'s frame ref the same way**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointCloud {
    pub fields: Vec<PointField>,
    pub point_step: u32,
    pub is_bigendian: bool,
    pub frame_rate_hz: u32,
    pub frame: RegistryRef,
}
```

Audio and JointEncoders don't have a frame reference today; leave them unchanged structurally, but verify and adjust if they do.

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p auki-registry
```

Expected: the new test passes; existing tests fail (will be fixed in Task 1.5 when fixtures regenerate). Note which tests now fail — they'll be cleaned up later.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-registry/src/lib.rs
git commit -m "feat(auki-registry): add peer_id to SensorRegistryEntry, switch frame refs to RegistryRef"
```

### Task 1.3: Add `peer_id` to `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add to `#[cfg(test)] mod tests`:

```rust
#[test]
fn clock_entry_canonical_has_peer_id() {
    let entry = ClockRegistryEntry {
        peer_id: PeerId::from_string("galbot").unwrap(),
        clock_id: "session/sdk_clock".to_string(),
        body: ClockBody::MonotonicClock(MonotonicClock {
            unit: "ns".to_string(),
            epoch: "session_start".to_string(),
            scope: "process".to_string(),
        }),
    };
    let json = auki_jcs::to_canonical_string(&entry).unwrap();
    assert!(json.contains(r#""peer_id":"galbot""#));
}

#[test]
fn frame_entry_canonical_has_peer_id() {
    let entry = FrameRegistryEntry {
        peer_id: PeerId::from_string("galbot").unwrap(),
        frame_id: "head_left_camera_optical".to_string(),
        handedness: Handedness::RightHanded,
        axes: AxesMap::ros_optical(),
        units: Units::Meters,
    };
    let json = auki_jcs::to_canonical_string(&entry).unwrap();
    assert!(json.contains(r#""peer_id":"galbot""#));
}

#[test]
fn detector_entry_canonical_has_peer_id() {
    let entry = DetectorRegistryEntry {
        peer_id: PeerId::from_string("galbot").unwrap(),
        detector_id: "yolo_v8".to_string(),
        body: DetectorBody::ObjectDetection { /* fill per existing variant */ },
        output_types: vec!["bounding_box".into()],
    };
    let json = auki_jcs::to_canonical_string(&entry).unwrap();
    assert!(json.contains(r#""peer_id":"galbot""#));
}
```

(Adjust the body construction per the existing `DetectorBody` enum shape — read it first; the assertion is the load-bearing part.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-registry clock_entry_canonical_has_peer_id frame_entry_canonical_has_peer_id detector_entry_canonical_has_peer_id
```

Expected: compile errors.

- [ ] **Step 3: Add `peer_id` to all three remaining entry types**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockRegistryEntry {
    pub peer_id: PeerId,
    pub clock_id: String,
    #[serde(flatten)]
    pub body: ClockBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRegistryEntry {
    pub peer_id: PeerId,
    pub frame_id: String,
    pub handedness: Handedness,
    pub axes: AxesMap,
    pub units: Units,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorRegistryEntry {
    pub peer_id: PeerId,
    pub detector_id: String,
    #[serde(flatten)]
    pub body: DetectorBody,
    pub output_types: Vec<String>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-registry clock_entry_canonical_has_peer_id frame_entry_canonical_has_peer_id detector_entry_canonical_has_peer_id
```

Expected: all three pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-registry/src/lib.rs
git commit -m "feat(auki-registry): add peer_id to clock/frame/detector entries"
```

### Task 1.4: Update disk-path helpers to include `peer_id` segment

**Files:**
- Modify: `crates/auki-registry/src/lib.rs` (or `crates/auki-layout/src/lib.rs` — verify which one owns the path helpers)
- Test: same file's `#[cfg(test)] mod tests`

- [ ] **Step 1: Read existing path helpers**

```bash
grep -n "registries/" crates/auki-registry/src/lib.rs crates/auki-layout/src/lib.rs
```

Note the existing helper function names (e.g., `sensor_path`, `clock_path`).

- [ ] **Step 2: Write failing test**

In `crates/auki-registry/src/lib.rs` tests:

```rust
#[test]
fn sensor_path_includes_peer_id_segment() {
    use std::path::PathBuf;
    let root = PathBuf::from("/tmp/auki");
    let path = sensor_path(&root, &PeerId::from_string("galbot").unwrap(), "head_left_rgb", "abc123");
    assert_eq!(
        path,
        PathBuf::from("/tmp/auki/registries/sensors/galbot/head_left_rgb/abc123.json")
    );
}

#[test]
fn clock_path_includes_peer_id_segment() {
    use std::path::PathBuf;
    let root = PathBuf::from("/tmp/auki");
    let path = clock_path(&root, &PeerId::from_string("galbot").unwrap(), "session/sdk_clock", "abc123");
    // forward slash in clock_id stays as a path separator
    assert_eq!(
        path,
        PathBuf::from("/tmp/auki/registries/clocks/galbot/session/sdk_clock/abc123.json")
    );
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p auki-registry sensor_path_includes_peer_id_segment clock_path_includes_peer_id_segment
```

Expected: compile error or signature mismatch.

- [ ] **Step 4: Update path helpers**

Update `sensor_path`, `clock_path`, `frame_path`, `detector_path` to accept `&PeerId` and prepend it as a segment:

```rust
pub fn sensor_path(root: &Path, peer_id: &PeerId, sensor_id: &str, hash: &str) -> PathBuf {
    root.join("registries").join("sensors").join(peer_id.as_str()).join(sensor_id).join(format!("{hash}.json"))
}

pub fn clock_path(root: &Path, peer_id: &PeerId, clock_id: &str, hash: &str) -> PathBuf {
    root.join("registries").join("clocks").join(peer_id.as_str()).join(clock_id).join(format!("{hash}.json"))
}

pub fn frame_path(root: &Path, peer_id: &PeerId, frame_id: &str, hash: &str) -> PathBuf {
    root.join("registries").join("frames").join(peer_id.as_str()).join(frame_id).join(format!("{hash}.json"))
}

pub fn detector_path(root: &Path, peer_id: &PeerId, detector_id: &str, hash: &str) -> PathBuf {
    root.join("registries").join("detectors").join(peer_id.as_str()).join(detector_id).join(format!("{hash}.json"))
}
```

Drop any prior id-mangling logic (`__` substitution for slashes) — leave slashes as path separators.

- [ ] **Step 5: Update call-sites in the crate's write/read helpers**

Walk every call to the path helpers in `auki-registry/src/lib.rs` and add the `peer_id` arg by pulling it from the entry being written/read. Compile-driven discovery is fine; the compiler errors point at each call site.

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p auki-registry sensor_path_includes_peer_id_segment clock_path_includes_peer_id_segment
```

Expected: both pass.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-registry/src/lib.rs crates/auki-layout/src/lib.rs
git commit -m "feat(auki-registry): disk paths include peer_id segment"
```

### Task 1.5: Regenerate locked JSON fixtures

**Files:**
- Create: `crates/auki-registry/tests/locked/*.json` (11 fixtures)
- Modify: `crates/auki-registry/tests/locked_json.rs` (new or existing) — round-trip harness

- [ ] **Step 1: Write the round-trip harness**

Create `crates/auki-registry/tests/locked_json.rs`:

```rust
use auki_registry::*;
use auki_identity::PeerId;
use std::fs;
use std::path::Path;

fn assert_locked<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    fixture: &str,
    value: T,
) {
    let path = Path::new("tests/locked").join(fixture);
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing fixture {fixture}"));
    let actual = auki_jcs::to_canonical_string(&value).unwrap();
    assert_eq!(actual, expected.trim_end(), "fixture {fixture} drifted");
    let parsed: T = serde_json::from_str(&actual).unwrap();
    assert_eq!(parsed, value, "fixture {fixture} round-trip mismatch");
}

#[test]
fn sensor_camera_locked() {
    let entry = SensorRegistryEntry {
        peer_id: PeerId::from_string("galbot").unwrap(),
        sensor_id: "head_left_rgb".to_string(),
        body: SensorBody::Camera(Camera {
            width: 1920,
            height: 1200,
            frame_rate_hz: 30,
            pixel_format: "rgb8".to_string(),
            color_space: "srgb".to_string(),
            intrinsics_model: "pinhole".to_string(),
            distortion_model: "brown_conrady".to_string(),
            frame: RegistryRef {
                peer_id: PeerId::from_string("galbot").unwrap(),
                id: "head_left_camera_optical".to_string(),
                hash: "framehash".to_string(),
            },
        }),
    };
    assert_locked("sensor_camera.json", entry);
}

// Repeat for: sensor_point_cloud, sensor_audio, sensor_joint_encoders,
// clock_monotonic, clock_utc, frame_ros_body, frame_ros_optical,
// frame_opengl, frame_unity, detector_object_detection
```

Write each test function. Use the spec's canonical examples for field values where possible.

- [ ] **Step 2: Run tests to verify they fail (fixtures missing)**

```bash
cargo test -p auki-registry --test locked_json
```

Expected: each test fails with "missing fixture …".

- [ ] **Step 3: Add an xtask to dump fixtures**

Add `cargo xtask regen-registry-fixtures` (or a one-off `bin/` target) that writes each test's canonical-JSON output to the fixture path:

In `crates/auki-registry/src/bin/regen_fixtures.rs` (or similar):

```rust
use auki_registry::*;
use auki_identity::PeerId;
use std::fs;

fn main() {
    let cases: Vec<(&str, String)> = vec![
        ("sensor_camera.json", {
            let entry = SensorRegistryEntry { /* same as test */ };
            auki_jcs::to_canonical_string(&entry).unwrap()
        }),
        // ... all 11
    ];

    for (name, json) in cases {
        let path = format!("crates/auki-registry/tests/locked/{name}");
        fs::create_dir_all("crates/auki-registry/tests/locked").unwrap();
        fs::write(&path, json + "\n").unwrap();
        println!("wrote {name}");
    }
}
```

- [ ] **Step 4: Run regen**

```bash
cargo run -p auki-registry --bin regen_fixtures
```

Expected: 11 JSON files written under `crates/auki-registry/tests/locked/`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p auki-registry --test locked_json
```

Expected: all 11 fixtures pass round-trip.

- [ ] **Step 6: Visual diff & commit**

```bash
git diff --stat crates/auki-registry/tests/locked/
git add crates/auki-registry/tests/locked/ crates/auki-registry/tests/locked_json.rs crates/auki-registry/src/bin/regen_fixtures.rs
git commit -m "test(auki-registry): regenerate locked JSON fixtures for peer_id schema"
```

### Task 1.6: ID charset enforcement

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn sensor_id_rejects_arrow_at_whitespace() {
    let bad_ids = ["foo>bar", "foo@bar", "foo bar", "foo\tbar"];
    for bad in bad_ids {
        let result = SensorRegistryEntry::validate_id(bad);
        assert!(result.is_err(), "id {bad:?} should be rejected");
    }
}

#[test]
fn sensor_id_allows_slash_underscore_dash() {
    for good in ["foo/bar", "foo_bar", "foo-bar", "a/b/c", "head_left_rgb"] {
        let result = SensorRegistryEntry::validate_id(good);
        assert!(result.is_ok(), "id {good:?} should be allowed");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-registry sensor_id_rejects sensor_id_allows
```

Expected: compile error — `validate_id` not defined.

- [ ] **Step 3: Add validation**

In `crates/auki-registry/src/lib.rs`:

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryIdError {
    #[error("registry id contains disallowed character {0:?}")]
    DisallowedChar(char),
    #[error("registry id is empty")]
    Empty,
}

fn validate_registry_id(id: &str) -> Result<(), RegistryIdError> {
    if id.is_empty() {
        return Err(RegistryIdError::Empty);
    }
    for c in id.chars() {
        if c == '>' || c == '@' || c.is_whitespace() {
            return Err(RegistryIdError::DisallowedChar(c));
        }
    }
    Ok(())
}

impl SensorRegistryEntry {
    pub fn validate_id(id: &str) -> Result<(), RegistryIdError> {
        validate_registry_id(id)
    }
}

// Same helper on ClockRegistryEntry / FrameRegistryEntry / DetectorRegistryEntry
```

Add `thiserror` to `Cargo.toml` if not present.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-registry sensor_id_rejects sensor_id_allows
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-registry/src/lib.rs crates/auki-registry/Cargo.toml
git commit -m "feat(auki-registry): reject '>', '@', whitespace in registry ids"
```

### Phase 1 checkpoint

```bash
cargo test -p auki-registry
```

Expected: all `auki-registry` tests green. Downstream crates (`auki-manifests`, `auki-network`) won't compile yet — that's fixed in Phase 2/3.

---

## Phase 2 — auki-manifests: source/writer split

Each manifest gains `source_peer_id` + `writer_peer_id`, switches cross-references to `RegistryRef` / `LogRef`, and the builder API is updated. Locked fixtures regenerate.

### Task 2.1: `SensorLogManifest` with source/writer split + `RegistryRef`

**Files:**
- Modify: `crates/auki-manifests/src/lib.rs`

- [ ] **Step 1: Write failing test**

Add to `crates/auki-manifests/src/lib.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn sensor_log_manifest_origin_canonical() {
    use auki_registry::{RegistryRef};
    use auki_identity::PeerId;

    let m = SensorLogManifest {
        source_peer_id: PeerId::from_string("galbot").unwrap(),
        writer_peer_id: PeerId::from_string("galbot").unwrap(),
        app_id: "galbot-control-plane".to_string(),
        session_id: "01HV-galbot-session".to_string(),
        sensor: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "head_left_rgb".to_string(),
            hash: "sensorhash".to_string(),
        },
        clock: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "session/sdk_clock".to_string(),
            hash: "clockhash".to_string(),
        },
        frame: Some(RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "head_left_camera_optical".to_string(),
            hash: "framehash".to_string(),
        }),
        segment_duration_ns: 1_000_000_000,
        retention_ns: 5_000_000_000,
    };
    let json = auki_jcs::to_canonical_string(&m).unwrap();
    assert!(json.contains(r#""source_peer_id":"galbot""#));
    assert!(json.contains(r#""writer_peer_id":"galbot""#));
    assert!(json.contains(r#""sensor":{"hash":"sensorhash","id":"head_left_rgb","peer_id":"galbot"}"#));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-manifests sensor_log_manifest_origin_canonical
```

Expected: compile error.

- [ ] **Step 3: Update the struct**

In `crates/auki-manifests/src/lib.rs`:

```rust
use auki_identity::PeerId;
use auki_registry::{RegistryRef, LogRef};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub sensor: RegistryRef,
    pub clock: RegistryRef,
    pub frame: Option<RegistryRef>,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}
```

Add `auki-registry` to `crates/auki-manifests/Cargo.toml` `[dependencies]` if absent.

- [ ] **Step 4: Update `build_sensor_log_manifest` signature**

```rust
pub fn build_sensor_log_manifest(
    source_peer_id: &PeerId,
    writer_peer_id: &PeerId,
    app_id: &str,
    session_id: &str,
    sensor: RegistryRef,
    clock: RegistryRef,
    frame: Option<RegistryRef>,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value {
    let m = SensorLogManifest {
        source_peer_id: source_peer_id.clone(),
        writer_peer_id: writer_peer_id.clone(),
        app_id: app_id.to_string(),
        session_id: session_id.to_string(),
        sensor,
        clock,
        frame,
        segment_duration_ns: segment_duration.as_nanos() as i64,
        retention_ns: retention.as_nanos() as i64,
    };
    serde_json::to_value(&m).unwrap()
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p auki-manifests sensor_log_manifest_origin_canonical
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-manifests/src/lib.rs crates/auki-manifests/Cargo.toml
git commit -m "feat(auki-manifests): SensorLogManifest gains source/writer peer ids + RegistryRef"
```

### Task 2.2: `PoseLogManifest` with source/writer split

**Files:**
- Modify: `crates/auki-manifests/src/lib.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn pose_log_manifest_movable_canonical() {
    use auki_registry::RegistryRef;
    use auki_identity::PeerId;

    let m = PoseLogManifest {
        source_peer_id: PeerId::from_string("galbot").unwrap(),
        writer_peer_id: PeerId::from_string("galbot").unwrap(),
        app_id: "galbot-control-plane".to_string(),
        session_id: "01HV".to_string(),
        from_frame: RegistryRef {
            peer_id: PeerId::from_string("park").unwrap(),
            id: "world".to_string(),
            hash: "fromhash".to_string(),
        },
        to_frame: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "base_link".to_string(),
            hash: "tohash".to_string(),
        },
        clock: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "session/sdk_clock".to_string(),
            hash: "clockhash".to_string(),
        },
        source: PoseSource::Ros2Tf { publishers: vec!["robot_state_publisher".into()] },
        writer_mode: PoseWriterMode::Movable,
        expected_rate_hz: 30,
        segment_duration_ns: 1_000_000_000,
        retention_ns: 60_000_000_000,
    };
    let json = auki_jcs::to_canonical_string(&m).unwrap();
    assert!(json.contains(r#""source_peer_id":"galbot""#));
    assert!(json.contains(r#""writer_mode":"movable""#));
    // from_frame uses Park's peer_id — cross-peer reference works
    assert!(json.contains(r#""from_frame":{"hash":"fromhash","id":"world","peer_id":"park"}"#));
}
```

(Adapt `PoseSource::Ros2Tf` to the current variant shape — read it first.)

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-manifests pose_log_manifest_movable_canonical
```

Expected: compile error.

- [ ] **Step 3: Update `PoseLogManifest` and builder**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoseLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id: String,
    pub session_id: String,
    pub from_frame: RegistryRef,
    pub to_frame: RegistryRef,
    pub clock: RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,
    pub expected_rate_hz: u32,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}

pub fn build_pose_log_manifest(
    source_peer_id: &PeerId,
    writer_peer_id: &PeerId,
    app_id: &str,
    session_id: &str,
    from_frame: RegistryRef,
    to_frame: RegistryRef,
    clock: RegistryRef,
    source: PoseSource,
    writer_mode: PoseWriterMode,
    expected_rate_hz: u32,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value { /* analogous to sensor builder */ }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p auki-manifests pose_log_manifest_movable_canonical
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-manifests/src/lib.rs
git commit -m "feat(auki-manifests): PoseLogManifest gains source/writer peer ids + RegistryRef"
```

### Task 2.3: `TimeTransformLogManifest` and `DetectionLogManifest`

**Files:**
- Modify: `crates/auki-manifests/src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn time_transform_log_manifest_canonical() {
    use auki_registry::RegistryRef;
    use auki_identity::PeerId;

    let m = TimeTransformLogManifest {
        source_peer_id: PeerId::from_string("galbot").unwrap(),
        writer_peer_id: PeerId::from_string("galbot").unwrap(),
        app_id: "ctrl".to_string(),
        session_id: "01HV".to_string(),
        from_clock: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "session/sdk_clock".to_string(),
            hash: "fromhash".to_string(),
        },
        to_clock: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "wall_clock".to_string(),
            hash: "tohash".to_string(),
        },
        source: TimeTransformSource::Heartbeat,
        segment_duration_ns: 60_000_000_000,
        retention_ns: 3_600_000_000_000,
    };
    let json = auki_jcs::to_canonical_string(&m).unwrap();
    assert!(json.contains(r#""source_peer_id":"galbot""#));
    assert!(json.contains(r#""from_clock":{"hash":"fromhash""#));
}

#[test]
fn detection_log_manifest_canonical() {
    use auki_registry::{RegistryRef, LogRef};
    use auki_identity::PeerId;

    let m = DetectionLogManifest {
        source_peer_id: PeerId::from_string("galbot").unwrap(),
        writer_peer_id: PeerId::from_string("galbot").unwrap(),
        app_id: "ctrl".to_string(),
        session_id: "01HV".to_string(),
        detector: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "yolo_v8".to_string(),
            hash: "dethash".to_string(),
        },
        input_log: LogRef {
            source_peer_id: PeerId::from_string("galbot").unwrap(),
            resource_id: "head_left_rgb".to_string(),
        },
        input_sensor: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "head_left_rgb".to_string(),
            hash: "sensorhash".to_string(),
        },
        clock: RegistryRef {
            peer_id: PeerId::from_string("galbot").unwrap(),
            id: "session/sdk_clock".to_string(),
            hash: "clockhash".to_string(),
        },
        segment_duration_ns: 1_000_000_000,
        retention_ns: 60_000_000_000,
    };
    let json = auki_jcs::to_canonical_string(&m).unwrap();
    assert!(json.contains(r#""input_log":{"resource_id":"head_left_rgb","source_peer_id":"galbot"}"#));
    assert!(json.contains(r#""detector":{"hash":"dethash""#));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-manifests time_transform_log_manifest_canonical detection_log_manifest_canonical
```

- [ ] **Step 3: Update both structs + builders**

Apply the same source/writer + RegistryRef adoption as Tasks 2.1/2.2.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-manifests time_transform_log_manifest_canonical detection_log_manifest_canonical
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-manifests/src/lib.rs
git commit -m "feat(auki-manifests): TimeTransform + Detection manifests gain source/writer split"
```

### Task 2.4: Regenerate locked manifest fixtures

**Files:**
- Create: `crates/auki-manifests/tests/locked/sensor_log_origin.json`
- Create: `crates/auki-manifests/tests/locked/sensor_log_materialized.json`
- Create: `crates/auki-manifests/tests/locked/pose_log_rigid.json`
- Create: `crates/auki-manifests/tests/locked/pose_log_movable.json`
- Create: `crates/auki-manifests/tests/locked/time_transform_log.json`
- Create: `crates/auki-manifests/tests/locked/detection_log.json`
- Modify: `crates/auki-manifests/tests/locked_json.rs`
- Create: `crates/auki-manifests/src/bin/regen_fixtures.rs`

- [ ] **Step 1: Write the harness**

Mirror Task 1.5's structure:

```rust
// crates/auki-manifests/tests/locked_json.rs
use auki_manifests::*;
use auki_registry::{RegistryRef, LogRef};
use auki_identity::PeerId;
use std::fs;
use std::path::Path;

fn assert_locked<T>(fixture: &str, value: T)
where T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug {
    let path = Path::new("tests/locked").join(fixture);
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing fixture {fixture}"));
    let actual = auki_jcs::to_canonical_string(&value).unwrap();
    assert_eq!(actual, expected.trim_end());
    let parsed: T = serde_json::from_str(&actual).unwrap();
    assert_eq!(parsed, value);
}

#[test] fn sensor_log_origin_locked()       { assert_locked("sensor_log_origin.json",       /* value */); }
#[test] fn sensor_log_materialized_locked() { assert_locked("sensor_log_materialized.json", /* value */); }
#[test] fn pose_log_rigid_locked()          { assert_locked("pose_log_rigid.json",          /* value */); }
#[test] fn pose_log_movable_locked()        { assert_locked("pose_log_movable.json",        /* value */); }
#[test] fn time_transform_log_locked()      { assert_locked("time_transform_log.json",      /* value */); }
#[test] fn detection_log_locked()           { assert_locked("detection_log.json",           /* value */); }
```

For the materialized fixture, set `source_peer_id = "galbot"`, `writer_peer_id = "park"`, `app_id = "park-vis"`. For the rigid pose fixture, set `writer_mode = Rigid`, `expected_rate_hz = 0`. The other fixtures use the origin pattern.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-manifests --test locked_json
```

Expected: all six fail with "missing fixture".

- [ ] **Step 3: Build the regen bin**

Mirror `crates/auki-registry/src/bin/regen_fixtures.rs`.

- [ ] **Step 4: Run regen**

```bash
cargo run -p auki-manifests --bin regen_fixtures
```

Expected: six JSON files in `crates/auki-manifests/tests/locked/`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p auki-manifests --test locked_json
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-manifests/tests/locked/ crates/auki-manifests/tests/locked_json.rs crates/auki-manifests/src/bin/regen_fixtures.rs
git commit -m "test(auki-manifests): locked JSON fixtures for source/writer manifests"
```

### Phase 2 checkpoint

```bash
cargo test -p auki-manifests
```

Expected: all green. `auki-network` consumers of these types still broken — Phase 3.

---

## Phase 3 — auki-network: new wire shape

New `ResourceEntry` with `source_peer_id`/`writer_peer_id`/`kind` closed enum/`state`/`head`/`extent`/`available`/`manifest`. New `StreamRequest`. Protocol version constants bumped to `0.2.0`.

### Task 3.1: Protocol version constants

**Files:**
- Modify: `crates/auki-network/src/lib.rs`

- [ ] **Step 1: Read existing constants**

```bash
grep -n "0\.0\.1\|0\.1\.0" crates/auki-network/src/lib.rs crates/auki-network/src/*.rs
```

- [ ] **Step 2: Write failing test**

```rust
// crates/auki-network/src/lib.rs at bottom
#[cfg(test)]
mod protocol_id_tests {
    use super::*;
    #[test]
    fn protocols_bumped_to_v0_2_0() {
        assert_eq!(RESOURCES_PROTOCOL,  "/auki/resources/0.2.0");
        assert_eq!(REGISTRIES_PROTOCOL, "/auki/registries/0.2.0");
        assert_eq!(STREAM_PROTOCOL,     "/auki/stream/0.2.0");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo test -p auki-network protocols_bumped_to_v0_2_0
```

- [ ] **Step 4: Update the constants**

```rust
pub const RESOURCES_PROTOCOL:  &str = "/auki/resources/0.2.0";
pub const REGISTRIES_PROTOCOL: &str = "/auki/registries/0.2.0";
pub const STREAM_PROTOCOL:     &str = "/auki/stream/0.2.0";
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p auki-network protocols_bumped_to_v0_2_0
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/src/lib.rs
git commit -m "feat(auki-network): bump protocol ids to 0.2.0 for #216 schema"
```

### Task 3.2: `Kind` closed enum

**Files:**
- Modify: `crates/auki-network/src/resources_protocol.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn kind_serializes_as_snake_case_string() {
    assert_eq!(serde_json::to_string(&Kind::Camera).unwrap(), r#""camera""#);
    assert_eq!(serde_json::to_string(&Kind::PointCloud).unwrap(), r#""point_cloud""#);
    assert_eq!(serde_json::to_string(&Kind::Audio).unwrap(), r#""audio""#);
    assert_eq!(serde_json::to_string(&Kind::JointEncoders).unwrap(), r#""joint_encoders""#);
    assert_eq!(serde_json::to_string(&Kind::Pose).unwrap(), r#""pose""#);
    assert_eq!(serde_json::to_string(&Kind::TimeTransform).unwrap(), r#""time_transform""#);
    assert_eq!(serde_json::to_string(&Kind::Detection).unwrap(), r#""detection""#);
}

#[test]
fn kind_rejects_unknown_string() {
    let result: Result<Kind, _> = serde_json::from_str(r#""foobar""#);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-network kind_serializes kind_rejects
```

- [ ] **Step 3: Add the enum**

In `crates/auki-network/src/resources_protocol.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Camera,
    PointCloud,
    Audio,
    JointEncoders,
    Pose,
    TimeTransform,
    Detection,
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p auki-network kind_serializes kind_rejects
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/resources_protocol.rs
git commit -m "feat(auki-network): add closed Kind enum for resource catalog rows"
```

### Task 3.3: `Head`, `Extent`, `Available` blocks + `State` discriminator

**Files:**
- Modify: `crates/auki-network/src/resources_protocol.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn head_rolling_canonical() {
    let h = Head::Rolling { retention_ns: 5_000_000_000 };
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"{"kind":"rolling","retention_ns":5000000000}"#);
}

#[test]
fn head_fixed_canonical() {
    let h = Head::Fixed { started_at_ns: 1733836800000000000 };
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"{"kind":"fixed","started_at_ns":1733836800000000000}"#);
}

#[test]
fn extent_canonical() {
    let e = Extent { start_at_ns: 100, finish_at_ns: 200 };
    let json = auki_jcs::to_canonical_string(&e).unwrap();
    assert_eq!(json, r#"{"finish_at_ns":200,"start_at_ns":100}"#);
}

#[test]
fn available_canonical() {
    let a = Available { bytes: 3_000_000_000, entries: 900, duration_ns: 5_000_000_000 };
    let json = auki_jcs::to_canonical_string(&a).unwrap();
    assert_eq!(json, r#"{"bytes":3000000000,"duration_ns":5000000000,"entries":900}"#);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-network head_rolling head_fixed extent_canonical available_canonical
```

- [ ] **Step 3: Add types**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Head {
    Rolling { retention_ns: i64 },
    Fixed   { started_at_ns: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extent {
    pub start_at_ns: i64,
    pub finish_at_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub bytes: u64,
    pub entries: u64,
    pub duration_ns: i64,
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-network head_rolling head_fixed extent_canonical available_canonical
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/src/resources_protocol.rs
git commit -m "feat(auki-network): add Head/Extent/Available blocks for catalog rows"
```

### Task 3.4: `ResourceEntry` and `ManifestSummary`, drop legacy types

**Files:**
- Modify: `crates/auki-network/src/resources_protocol.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn live_rolling_camera_row_canonical() {
    use auki_registry::RegistryRef;
    use auki_identity::PeerId;
    let pid = |s| PeerId::from_string(s).unwrap();

    let row = ResourceEntry {
        source_peer_id: pid("galbot"),
        writer_peer_id: pid("galbot"),
        resource_id: "head_left_rgb".to_string(),
        kind: Kind::Camera,
        state: "live".to_string(),
        head: Some(Head::Rolling { retention_ns: 5_000_000_000 }),
        extent: None,
        available: Available { bytes: 3_000_000_000, entries: 900, duration_ns: 5_000_000_000 },
        manifest: ManifestSummary::Sensor {
            sensor: RegistryRef { peer_id: pid("galbot"), id: "head_left_rgb".into(),       hash: "sh".into() },
            clock:  RegistryRef { peer_id: pid("galbot"), id: "session/sdk_clock".into(),   hash: "ch".into() },
            frame:  Some(RegistryRef { peer_id: pid("galbot"), id: "head_left_camera_optical".into(), hash: "fh".into() }),
        },
    };
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(json["source_peer_id"], "galbot");
    assert_eq!(json["kind"], "camera");
    assert_eq!(json["state"], "live");
    assert!(json["head"].is_object());
    assert!(json.get("extent").map_or(true, |v| v.is_null()));
    assert_eq!(json["available"]["entries"], 900);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-network live_rolling_camera_row_canonical
```

- [ ] **Step 3: Define `ResourceEntry` and `ManifestSummary`**

Delete the existing `SensorStreamResource`, `TransformEdgeResource`, `PoseStreamResource` structs. Replace with:

```rust
use auki_registry::{RegistryRef, LogRef};
use auki_identity::PeerId;
use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceEntry {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub resource_id: String,
    pub kind: Kind,
    pub state: String, // open string per spec; v1: "live" | "sealed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<Head>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extent: Option<Extent>,
    pub available: Available,
    pub manifest: ManifestSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ManifestSummary {
    Sensor {
        sensor: RegistryRef,
        clock: RegistryRef,
        frame: Option<RegistryRef>,
    },
    Pose {
        from_frame: RegistryRef,
        to_frame: RegistryRef,
        clock: RegistryRef,
        writer_mode: PoseWriterMode,
        source: PoseSource,
        expected_rate_hz: u32,
    },
    TimeTransform {
        from_clock: RegistryRef,
        to_clock: RegistryRef,
        source: TimeTransformSource,
    },
    Detection {
        detector: RegistryRef,
        input_log: LogRef,
        input_sensor: RegistryRef,
        clock: RegistryRef,
    },
}
```

Note: untagged enum means JSON shape determines the variant. The catalog row's `kind` field still indicates which variant to expect, but parsing falls through untagged matching. Verify this works in tests; if it doesn't, swap to a `kind`-driven external tag.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p auki-network live_rolling_camera_row_canonical
```

- [ ] **Step 5: Update `ResourcesRequest` and `ResourcesResponse`**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcesRequest {
    /// Open-string filter; empty means "all kinds I produce or materialize".
    pub kinds: Vec<Kind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourcesResponse {
    pub resources: Vec<ResourceEntry>,
}
```

Drop `include_sensor_entries` / `include_frame_entries` / `include_clock_entries` — registry-entry embedding is gone (registry refs in the manifest summary are sufficient).

- [ ] **Step 6: Run full crate tests**

```bash
cargo test -p auki-network
```

Expected: existing tests that referenced the deleted resource structs fail to compile — delete or rewrite them.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-network/src/resources_protocol.rs
git commit -m "feat(auki-network): replace ResourceEntry with #216 schema; drop legacy resource types"
```

### Task 3.5: `StreamRequest` and `ReadFrom`

**Files:**
- Modify: `crates/auki-network/src/stream_protocol.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn stream_request_canonical() {
    use auki_identity::PeerId;
    let r = StreamRequest {
        source_peer_id: PeerId::from_string("galbot").unwrap(),
        resource_id: "head_left_rgb".to_string(),
        from: ReadFrom::FromTimestamp(1733836800000000000),
    };
    let json = auki_jcs::to_canonical_string(&r).unwrap();
    assert!(json.contains(r#""source_peer_id":"galbot""#));
    assert!(json.contains(r#""resource_id":"head_left_rgb""#));
    assert!(json.contains(r#""from":{"from_timestamp":1733836800000000000}"#));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p auki-network stream_request_canonical
```

- [ ] **Step 3: Define types**

In `crates/auki-network/src/stream_protocol.rs`:

```rust
use auki_identity::PeerId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRequest {
    pub source_peer_id: PeerId,
    pub resource_id: String,
    pub from: ReadFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFrom {
    Latest,
    FromStart,
    FromTimestamp(i64),
}
```

Drop any existing `sensor_id`/`resource_id`/etc field permutations on the old StreamRequest. Existing call-sites will fail to compile — fix in the next step.

- [ ] **Step 4: Update stream runtime call-sites**

```bash
grep -n "StreamRequest" crates/auki-network/src/
```

For each, replace old field access with the new shape. The full sweep is mechanical; let the compiler drive.

- [ ] **Step 5: Run tests**

```bash
cargo test -p auki-network stream_request_canonical
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/src/stream_protocol.rs crates/auki-network/src/stream_runtime.rs
git commit -m "feat(auki-network): replace StreamRequest with (source_peer_id, resource_id, ReadFrom)"
```

### Task 3.6: Locked wire fixtures

**Files:**
- Create: `crates/auki-network/tests/locked/catalog_row_live_rolling_camera.json`
- Create: `crates/auki-network/tests/locked/catalog_row_live_fixed_pose.json`
- Create: `crates/auki-network/tests/locked/catalog_row_sealed_camera.json`
- Create: `crates/auki-network/tests/locked/catalog_row_sealed_one_sample_pose.json`
- Create: `crates/auki-network/tests/locked/catalog_row_materialization.json`
- Create: `crates/auki-network/tests/locked/stream_request.json`
- Modify: `crates/auki-network/tests/locked_json.rs` (new)
- Create: `crates/auki-network/src/bin/regen_fixtures.rs`

- [ ] **Step 1–6:** mirror Tasks 1.5 / 2.4. Each row uses the spec's exact example values. The materialization row has `source_peer_id="galbot"`, `writer_peer_id="park"`.

- [ ] **Final commit**

```bash
git add crates/auki-network/tests/locked/ crates/auki-network/tests/locked_json.rs crates/auki-network/src/bin/regen_fixtures.rs
git commit -m "test(auki-network): locked wire fixtures for #216 catalog rows + stream request"
```

### Phase 3 checkpoint

```bash
cargo test -p auki-network
cargo build --workspace
```

Expected: `auki-network` tests green. Workspace build may still fail on `auki-domain` (Phase 5) or downstream binaries — that's expected.

---

## Phase 4 — `auki-session` (new crate)

### Task 4.1: Workspace + crate skeleton

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `crates/auki-session/Cargo.toml`
- Create: `crates/auki-session/src/lib.rs`

- [ ] **Step 1: Add to workspace members**

In root `Cargo.toml`:

```toml
[workspace]
members = [
    # ... existing
    "crates/auki-session",
]
```

- [ ] **Step 2: Create the crate manifest**

`crates/auki-session/Cargo.toml`:

```toml
[package]
name = "auki-session"
version = "0.0.0"
edition = "2024"

[dependencies]
auki-identity   = { path = "../auki-identity" }
auki-registry   = { path = "../auki-registry" }
auki-manifests  = { path = "../auki-manifests" }
auki-network    = { path = "../auki-network" }
auki-domain     = { path = "../auki-domain" }
auki-datatypes  = { path = "../auki-datatypes" }
auki-logs       = { path = "../auki-logs" }
auki-jcs        = { path = "../auki-jcs" }
auki-hash       = { path = "../auki-hash" }
auki-time       = { path = "../auki-time" }
serde           = { workspace = true, features = ["derive"] }
serde_json      = { workspace = true }
tokio           = { workspace = true, features = ["sync", "fs"] }
thiserror       = { workspace = true }
parking_lot     = { workspace = true }
ulid            = { workspace = true }
```

Adjust to match the workspace's actual dependency declarations; copy from a similar crate like `auki-domain` for consistency.

- [ ] **Step 3: Empty lib**

`crates/auki-session/src/lib.rs`:

```rust
//! Session — per-process declarative API for the Auki SDK.
//!
//! Apps construct a [`Session`], register their sensors / clocks / frames /
//! detectors and the logs they own, then join a domain to advertise them.
//! See `docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md`.

mod session;
mod registry_store;
mod log_specs;
mod log_handles;
mod materialization;

pub use session::Session;
pub use registry_store::RegistryStore;
pub use log_specs::{HeadSpec, SensorLogSpec, PoseLogSpec, TimeTransformLogSpec, DetectionLogSpec};
pub use log_handles::{SensorLogHandle, PoseLogHandle, TimeTransformLogHandle, DetectionLogHandle, MaterializedLogHandle};
pub use materialization::MaterializationError;
```

- [ ] **Step 4: Verify compile**

```bash
cargo build -p auki-session
```

Expected: error — `mod session` not found. Tasks 4.2+ add each module.

- [ ] **Step 5: Stub each module with `pub struct Foo;` placeholders so the crate compiles**

Create `crates/auki-session/src/session.rs` containing `pub struct Session;`. Same for the other modules — minimal stubs.

- [ ] **Step 6: Verify compile**

```bash
cargo build -p auki-session
```

Expected: green.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/auki-session/
git commit -m "feat(auki-session): scaffold new crate"
```

### Task 4.2: `Session::new`, `with_storage_root`, internal state

**Files:**
- Modify: `crates/auki-session/src/session.rs`

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use auki_identity::PeerId;
    use std::path::PathBuf;

    #[test]
    fn session_new_carries_peer_app_and_generates_session_id() {
        let s = Session::new(PeerId::from_string("galbot").unwrap(), "galbot-ctrl");
        assert_eq!(s.peer_id().as_str(), "galbot");
        assert_eq!(s.app_id(), "galbot-ctrl");
        assert!(!s.session_id().is_empty());
    }

    #[test]
    fn with_storage_root_sets_root() {
        let s = Session::new(PeerId::from_string("p").unwrap(), "a")
            .with_storage_root(PathBuf::from("/tmp/auki"));
        assert_eq!(s.storage_root(), &PathBuf::from("/tmp/auki"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-session
```

- [ ] **Step 3: Implement**

In `crates/auki-session/src/session.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use auki_identity::PeerId;
use auki_registry::*;
use std::collections::HashMap;

pub struct Session {
    pub(crate) inner: Arc<RwLock<SessionInner>>,
}

pub(crate) struct SessionInner {
    pub(crate) peer_id: PeerId,
    pub(crate) app_id: String,
    pub(crate) session_id: String,
    pub(crate) storage_root: PathBuf,
    pub(crate) sensors:   crate::registry_store::RegistryStore<SensorRegistryEntry>,
    pub(crate) clocks:    crate::registry_store::RegistryStore<ClockRegistryEntry>,
    pub(crate) frames:    crate::registry_store::RegistryStore<FrameRegistryEntry>,
    pub(crate) detectors: crate::registry_store::RegistryStore<DetectorRegistryEntry>,
    pub(crate) sensor_logs:    HashMap<(PeerId, String), Arc<crate::log_handles::SensorLogState>>,
    pub(crate) pose_logs:      HashMap<(PeerId, String), Arc<crate::log_handles::PoseLogState>>,
    pub(crate) time_logs:      HashMap<(PeerId, String), Arc<crate::log_handles::TimeTransformLogState>>,
    pub(crate) detection_logs: HashMap<(PeerId, String), Arc<crate::log_handles::DetectionLogState>>,
}

impl Session {
    pub fn new(peer_id: PeerId, app_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SessionInner {
                peer_id,
                app_id: app_id.into(),
                session_id: ulid::Ulid::new().to_string(),
                storage_root: PathBuf::from("."),
                sensors:   crate::registry_store::RegistryStore::default(),
                clocks:    crate::registry_store::RegistryStore::default(),
                frames:    crate::registry_store::RegistryStore::default(),
                detectors: crate::registry_store::RegistryStore::default(),
                sensor_logs:    HashMap::new(),
                pose_logs:      HashMap::new(),
                time_logs:      HashMap::new(),
                detection_logs: HashMap::new(),
            })),
        }
    }

    pub fn with_storage_root(self, root: PathBuf) -> Self {
        self.inner.write().storage_root = root;
        self
    }

    pub fn peer_id(&self) -> PeerId { self.inner.read().peer_id.clone() }
    pub fn app_id(&self) -> String { self.inner.read().app_id.clone() }
    pub fn session_id(&self) -> String { self.inner.read().session_id.clone() }
    pub fn storage_root(&self) -> PathBuf { self.inner.read().storage_root.clone() }
}
```

Stub `RegistryStore` and the `*LogState` types so this compiles. They'll get fleshed out in 4.3 and later.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-session
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-session/src/session.rs crates/auki-session/src/registry_store.rs crates/auki-session/src/log_handles.rs
git commit -m "feat(auki-session): Session struct, constructor, basic accessors"
```

### Task 4.3: `Session::register_sensor` / `register_clock` / `register_frame` / `register_detector`

**Files:**
- Modify: `crates/auki-session/src/session.rs`
- Modify: `crates/auki-session/src/registry_store.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn register_sensor_returns_registry_ref_with_self_peer_id() {
    let s = Session::new(PeerId::from_string("galbot").unwrap(), "app")
        .with_storage_root(tempfile::tempdir().unwrap().path().to_path_buf());

    let body = SensorBody::Camera(Camera { /* fill from spec */ });
    let r = s.register_sensor("head_left_rgb", body).unwrap();

    assert_eq!(r.peer_id.as_str(), "galbot");
    assert_eq!(r.id, "head_left_rgb");
    assert!(!r.hash.is_empty());

    // Re-registering the same sensor with same body is idempotent (same hash)
    let r2 = s.register_sensor("head_left_rgb", body).unwrap();
    assert_eq!(r.hash, r2.hash);
}

#[test]
fn register_sensor_rejects_invalid_id() {
    let s = Session::new(PeerId::from_string("p").unwrap(), "a")
        .with_storage_root(tempfile::tempdir().unwrap().path().to_path_buf());

    let body = SensorBody::Camera(Camera { /* fill */ });
    let result = s.register_sensor("bad>id", body);
    assert!(matches!(result, Err(SessionError::InvalidId(_))));
}
```

Add `tempfile` to `[dev-dependencies]`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-session register_sensor
```

- [ ] **Step 3: Define `SessionError`**

In `crates/auki-session/src/lib.rs` (or `src/error.rs`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid registry id: {0}")]
    InvalidId(#[from] auki_registry::RegistryIdError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("duplicate log {0}/{1}")]
    DuplicateLog(String, String),
    #[error("materialization: {0}")]
    Materialization(#[from] crate::materialization::MaterializationError),
}

pub type Result<T> = std::result::Result<T, SessionError>;
```

Re-export.

- [ ] **Step 4: Implement `register_sensor`**

```rust
impl Session {
    pub fn register_sensor(&self, sensor_id: &str, body: SensorBody) -> Result<RegistryRef> {
        SensorRegistryEntry::validate_id(sensor_id)?;
        let inner = self.inner.read();
        let entry = SensorRegistryEntry {
            peer_id: inner.peer_id.clone(),
            sensor_id: sensor_id.to_string(),
            body,
        };
        let canon = auki_jcs::to_canonical_string(&entry).unwrap();
        let hash = auki_hash::hash_string(&canon);
        let path = auki_registry::sensor_path(&inner.storage_root, &inner.peer_id, sensor_id, &hash);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, &canon)?;
        Ok(RegistryRef { peer_id: inner.peer_id.clone(), id: sensor_id.to_string(), hash })
    }
}
```

`auki_hash::hash_string` — confirm the actual symbol name in the existing crate; substitute if needed.

- [ ] **Step 5: Implement the other three `register_*` methods (clock/frame/detector)**

Same pattern; different entry struct and path helper.

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p auki-session register_sensor
```

- [ ] **Step 7: Commit**

```bash
git add crates/auki-session/src/
git commit -m "feat(auki-session): register_* methods for the 4 registry types"
```

### Task 4.4: Log specs (`HeadSpec`, `SensorLogSpec`, `PoseLogSpec`, etc.)

**Files:**
- Modify: `crates/auki-session/src/log_specs.rs`

- [ ] **Step 1: Define spec types**

```rust
use std::time::Duration;
use auki_registry::{RegistryRef, LogRef};
use auki_manifests::{PoseSource, PoseWriterMode, TimeTransformSource};

#[derive(Debug, Clone)]
pub enum HeadSpec {
    Rolling { retention_ns: i64 },
    Fixed,
}

#[derive(Debug, Clone)]
pub struct SensorLogSpec {
    pub sensor: RegistryRef,
    pub clock: RegistryRef,
    pub frame: Option<RegistryRef>,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct PoseLogSpec {
    pub from_frame: RegistryRef,
    pub to_frame: RegistryRef,
    pub clock: RegistryRef,
    pub source: PoseSource,
    pub writer_mode: PoseWriterMode,
    pub expected_rate_hz: u32,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct TimeTransformLogSpec {
    pub from_clock: RegistryRef,
    pub to_clock: RegistryRef,
    pub source: TimeTransformSource,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}

#[derive(Debug, Clone)]
pub struct DetectionLogSpec {
    pub detector: RegistryRef,
    pub input_log: LogRef,
    pub input_sensor: RegistryRef,
    pub clock: RegistryRef,
    pub head: HeadSpec,
    pub segment_duration: Duration,
    pub retention: Duration,
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/auki-session/src/log_specs.rs
git commit -m "feat(auki-session): log spec types"
```

### Task 4.5: `Session::register_sensor_log` (TDD)

**Files:**
- Modify: `crates/auki-session/src/session.rs`
- Modify: `crates/auki-session/src/log_handles.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn register_sensor_log_creates_handle_with_correct_log_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Session::new(PeerId::from_string("galbot").unwrap(), "app")
        .with_storage_root(tmp.path().to_path_buf());

    let sensor = s.register_sensor("head_left_rgb", /* Camera body */).unwrap();
    let clock  = s.register_clock("session/sdk_clock", /* MonotonicClock body */).unwrap();
    let frame  = s.register_frame("head_left_camera_optical", /* FrameDef */).unwrap();

    let handle = s.register_sensor_log(SensorLogSpec {
        sensor: sensor.clone(),
        clock,
        frame: Some(frame),
        head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
        segment_duration: Duration::from_secs(1),
        retention: Duration::from_secs(5),
    }).unwrap();

    assert_eq!(handle.log_ref().source_peer_id.as_str(), "galbot");
    assert_eq!(handle.log_ref().resource_id, "head_left_rgb");
    assert_eq!(handle.resource_id(), "head_left_rgb");

    // Manifest persisted to disk
    let manifest_path = tmp.path().join("logs/galbot/head_left_rgb/manifest.json");
    assert!(manifest_path.exists());
}

#[test]
fn register_sensor_log_rejects_duplicate_resource_id() {
    /* same setup + second register_sensor_log returns DuplicateLog */
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p auki-session register_sensor_log
```

- [ ] **Step 3: Implement `register_sensor_log`**

```rust
impl Session {
    pub fn register_sensor_log(&self, spec: SensorLogSpec) -> Result<SensorLogHandle> {
        let resource_id = spec.sensor.id.clone();
        let mut inner = self.inner.write();
        let key = (inner.peer_id.clone(), resource_id.clone());
        if inner.sensor_logs.contains_key(&key) {
            return Err(SessionError::DuplicateLog(inner.peer_id.as_str().to_string(), resource_id));
        }
        let manifest = SensorLogManifest {
            source_peer_id: inner.peer_id.clone(),
            writer_peer_id: inner.peer_id.clone(),
            app_id: inner.app_id.clone(),
            session_id: inner.session_id.clone(),
            sensor: spec.sensor.clone(),
            clock: spec.clock,
            frame: spec.frame,
            segment_duration_ns: spec.segment_duration.as_nanos() as i64,
            retention_ns: spec.retention.as_nanos() as i64,
        };
        let manifest_dir = inner.storage_root
            .join("logs").join(inner.peer_id.as_str()).join(&resource_id);
        std::fs::create_dir_all(&manifest_dir)?;
        std::fs::write(
            manifest_dir.join("manifest.json"),
            auki_jcs::to_canonical_string(&manifest).unwrap(),
        )?;
        // (Allocating the backing Log<CameraFrame> is deferred to a later task.
        // Track state for catalog production now.)
        let state = Arc::new(SensorLogState {
            log_ref: LogRef { source_peer_id: inner.peer_id.clone(), resource_id: resource_id.clone() },
            manifest,
            head_spec: spec.head,
            sealed: false,
            // entries/bytes tracking: stub
        });
        inner.sensor_logs.insert(key, state.clone());
        Ok(SensorLogHandle { state })
    }
}
```

Define `SensorLogState` and `SensorLogHandle::log_ref()/resource_id()` in `log_handles.rs`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p auki-session register_sensor_log
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-session/src/
git commit -m "feat(auki-session): register_sensor_log writes manifest, returns handle"
```

### Task 4.6: `register_pose_log` / `register_time_transform_log` / `register_detection_log`

**Files:**
- Modify: `crates/auki-session/src/session.rs`
- Modify: `crates/auki-session/src/log_handles.rs`

- [ ] **Step 1: Write failing tests**

One per log kind. Resource_id format check is the load-bearing part:

```rust
#[test]
fn register_pose_log_resource_id_is_from_arrow_to() {
    /* setup */
    let h = s.register_pose_log(PoseLogSpec {
        from_frame: world_ref,
        to_frame:   base_link_ref,
        clock,
        source: PoseSource::Manual,
        writer_mode: PoseWriterMode::Movable,
        expected_rate_hz: 30,
        head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
        segment_duration: Duration::from_secs(1),
        retention: Duration::from_secs(5),
    }).unwrap();
    assert_eq!(h.resource_id(), "world->base_link");
}

#[test]
fn register_time_transform_log_resource_id_format() {
    /* "<from_clock>-><to_clock>" */
}

#[test]
fn register_detection_log_resource_id_format() {
    /* "<detector>@<input_sensor>" */
}
```

- [ ] **Step 2: Run tests to verify they fail**

- [ ] **Step 3: Implement the three methods, mirroring Task 4.5**

The only differences: how `resource_id` is computed, and the manifest type.

- [ ] **Step 4: Run tests to verify they pass**

- [ ] **Step 5: Commit**

```bash
git add crates/auki-session/src/
git commit -m "feat(auki-session): register_pose_log / register_time_transform_log / register_detection_log"
```

### Task 4.7: `Session::catalog`

**Files:**
- Modify: `crates/auki-session/src/session.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn catalog_returns_a_row_per_registered_log() {
    let s = /* setup with 1 sensor log + 1 pose log */;
    let rows = s.catalog();
    assert_eq!(rows.len(), 2);
    let camera_row = rows.iter().find(|r| r.kind == Kind::Camera).unwrap();
    assert_eq!(camera_row.source_peer_id.as_str(), "galbot");
    assert_eq!(camera_row.writer_peer_id.as_str(), "galbot");
    assert_eq!(camera_row.resource_id, "head_left_rgb");
    assert_eq!(camera_row.state, "live");
    assert!(matches!(camera_row.head, Some(Head::Rolling { retention_ns: 5_000_000_000 })));
}
```

- [ ] **Step 2: Run test to verify it fails**

- [ ] **Step 3: Implement `catalog`**

```rust
impl Session {
    pub fn catalog(&self) -> Vec<ResourceEntry> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for state in inner.sensor_logs.values() {
            out.push(state.to_resource_entry());
        }
        for state in inner.pose_logs.values()      { out.push(state.to_resource_entry()); }
        for state in inner.time_logs.values()      { out.push(state.to_resource_entry()); }
        for state in inner.detection_logs.values() { out.push(state.to_resource_entry()); }
        out
    }
}
```

Each `*LogState::to_resource_entry()` builds the row from manifest + current write state. For v1, `Available` can be a stub (0 / 0 / 0) until the backing log is wired up — flag this as a follow-up but don't block on it.

- [ ] **Step 4: Run test to verify it passes**

- [ ] **Step 5: Commit**

```bash
git add crates/auki-session/src/
git commit -m "feat(auki-session): catalog() produces resource entries from registered logs"
```

### Task 4.8: `Session::materialize_remote_log` (sketch + stub)

**Files:**
- Modify: `crates/auki-session/src/materialization.rs`

- [ ] **Step 1: Define types**

```rust
use std::time::Duration;
use auki_registry::LogRef;

#[derive(Debug, thiserror::Error)]
pub enum MaterializationError {
    #[error("remote catalog row not found: {0:?}")]
    NotFound(LogRef),
    #[error("connection: {0}")]
    Connection(String),
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
    // ... backing local Log<T> reference
}

impl Session {
    pub async fn materialize_remote_log(
        &self,
        log_ref: LogRef,
        retention: Duration,
        segment_duration: Duration,
    ) -> crate::Result<MaterializedLogHandle> {
        // 1. Fetch remote catalog row for log_ref (via auki-domain when available)
        // 2. Extract canonical fields from row.manifest
        // 3. Open /auki/stream/0.2.0 against the serving peer
        // 4. Write local manifest with source_peer_id = remote, writer_peer_id = self
        // 5. Spawn ingest task
        todo!("full materialization arrives in a follow-up plan")
    }
}
```

The full materialization design is deferred per the spec — this task lands the surface signature only so app code can wire against it.

- [ ] **Step 2: Commit**

```bash
git add crates/auki-session/src/materialization.rs
git commit -m "feat(auki-session): materialize_remote_log surface (deferred body)"
```

### Phase 4 checkpoint

```bash
cargo test -p auki-session
cargo build --workspace
```

Expected: `auki-session` tests green. Workspace may still fail on Python bindings — Phase 6.

---

## Phase 5 — auki-domain refactor

Move from app-facing to internal-only. `auki-domain` continues to own libp2p + protocol implementations, but apps no longer construct its public types directly.

### Task 5.1: `auki-domain` takes a `Session` handle

**Files:**
- Modify: `crates/auki-domain/src/cluster_manager.rs`
- Modify: `crates/auki-domain/src/lib.rs`

- [ ] **Step 1: Read existing cluster_manager API**

```bash
grep -n "pub fn\|pub async fn" crates/auki-domain/src/cluster_manager.rs
```

- [ ] **Step 2: Add a `SessionHandle` trait**

In `crates/auki-domain/src/lib.rs`:

```rust
use std::sync::Arc;
use auki_network::resources_protocol::ResourceEntry;

/// Internal handle the Domain calls into for catalog + manifest data.
/// Implemented by `auki_session::Session`.
pub trait SessionHandle: Send + Sync {
    fn catalog(&self) -> Vec<ResourceEntry>;
}
```

- [ ] **Step 3: Wire the resources_protocol handler to call the handle**

In `cluster_manager.rs`, replace the existing catalog assembly (which read auki-domain's own state) with a call into `self.session.catalog()`.

- [ ] **Step 4: Add a test using a fake SessionHandle**

```rust
struct FakeSession(Vec<ResourceEntry>);
impl SessionHandle for FakeSession {
    fn catalog(&self) -> Vec<ResourceEntry> { self.0.clone() }
}

#[tokio::test]
async fn resources_protocol_returns_session_catalog() {
    // Spin up two ClusterManagers in-process; one with a fake session that
    // has one camera row; the other opens /auki/resources/0.2.0 against it
    // and asserts the row comes back.
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p auki-domain
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-domain/
git commit -m "feat(auki-domain): cluster manager takes a SessionHandle for catalog production"
```

### Task 5.2: `Session::join_domain` constructs and owns the Domain

**Files:**
- Modify: `crates/auki-session/src/session.rs`

- [ ] **Step 1: Implement `SessionHandle` for `Session`**

```rust
// In auki-session, behind a feature or just at the top of session.rs:
impl auki_domain::SessionHandle for Session {
    fn catalog(&self) -> Vec<ResourceEntry> { Session::catalog(self) }
}
```

- [ ] **Step 2: Add `join_domain` / `leave_domain`**

```rust
impl Session {
    pub async fn join_domain(&mut self, config: auki_domain::ClusterConfig) -> Result<()> {
        let handle: Arc<dyn auki_domain::SessionHandle> = Arc::new(self.clone());
        let domain = auki_domain::Domain::join(config, handle).await?;
        self.domain = Some(domain);
        Ok(())
    }

    pub async fn leave_domain(&mut self) -> Result<()> {
        if let Some(d) = self.domain.take() {
            d.leave().await?;
        }
        Ok(())
    }
}
```

(The exact `Domain::join` signature lives in `auki-domain` — match it. Adjust if the existing API uses a `ClusterMembership` instead.)

- [ ] **Step 3: Test (integration)**

Skip a full libp2p test here — the end-to-end behavior is covered in Phase 7's smoke test. A compile check is enough for this task.

- [ ] **Step 4: Commit**

```bash
git add crates/auki-session/src/session.rs
git commit -m "feat(auki-session): join_domain wires Session into auki-domain"
```

### Phase 5 checkpoint

```bash
cargo test -p auki-domain -p auki-session
cargo build --workspace
```

Expected: both green; workspace builds.

---

## Phase 6 — Python bindings

### Task 6.1: `auki-registry-py` mirrors new shapes

**Files:**
- Modify: `bindings/python/auki-registry-py/src/lib.rs`
- Modify: `bindings/python/auki-registry-py/python_tests/test_registry.py`

- [ ] **Step 1: Update the Python `SensorRegistryEntry` constructor**

In `bindings/python/auki-registry-py/src/lib.rs`, find the existing PyO3 class and add `peer_id` as a required constructor arg. Switch the `Camera` body's `frame_id`/`frame_hash` pair to a `RegistryRef` argument.

- [ ] **Step 2: Add `RegistryRef` and `LogRef` Python classes**

```rust
#[pyclass]
#[derive(Clone)]
pub struct RegistryRef {
    pub peer_id: String,
    pub id: String,
    pub hash: String,
}

#[pymethods]
impl RegistryRef {
    #[new]
    fn new(peer_id: String, id: String, hash: String) -> Self {
        Self { peer_id, id, hash }
    }
}

#[pyclass]
#[derive(Clone)]
pub struct LogRef {
    pub source_peer_id: String,
    pub resource_id: String,
}

#[pymethods]
impl LogRef {
    #[new]
    fn new(source_peer_id: String, resource_id: String) -> Self {
        Self { source_peer_id, resource_id }
    }
}
```

- [ ] **Step 3: Mirror in Python tests**

```python
# python_tests/test_registry.py
from auki_registry import SensorRegistryEntry, Camera, RegistryRef

def test_sensor_camera_constructs():
    frame = RegistryRef(peer_id="galbot", id="head_left_camera_optical", hash="fh")
    body  = Camera(width=1920, height=1200, frame_rate_hz=30,
                   pixel_format="rgb8", color_space="srgb",
                   intrinsics_model="pinhole", distortion_model="brown_conrady",
                   frame=frame)
    e = SensorRegistryEntry(peer_id="galbot", sensor_id="head_left_rgb", body=body)
    assert e.peer_id == "galbot"
```

- [ ] **Step 4: Run tests**

```bash
cd bindings/python/auki-registry-py && maturin develop && pytest python_tests/
```

- [ ] **Step 5: Commit**

```bash
git add bindings/python/auki-registry-py/
git commit -m "feat(auki-registry-py): expose peer_id + RegistryRef/LogRef in Python"
```

### Task 6.2: `auki-manifests-py` mirrors new shapes

**Files:**
- Modify: `bindings/python/auki-manifests-py/src/lib.rs`
- Modify: `bindings/python/auki-manifests-py/python_tests/test_manifests.py`

- [ ] **Step 1: Update each manifest class**

Add `source_peer_id`, `writer_peer_id`, and switch refs to use the Python `RegistryRef` / `LogRef` from `auki-registry-py`.

- [ ] **Step 2: Update Python tests**

```python
from auki_manifests import SensorLogManifest
from auki_registry import RegistryRef

def test_sensor_log_manifest_origin():
    m = SensorLogManifest(
        source_peer_id="galbot",
        writer_peer_id="galbot",
        app_id="ctrl",
        session_id="01HV",
        sensor=RegistryRef("galbot", "head_left_rgb", "sh"),
        clock= RegistryRef("galbot", "session/sdk_clock", "ch"),
        frame= RegistryRef("galbot", "head_left_camera_optical", "fh"),
        segment_duration_ns=1_000_000_000,
        retention_ns=5_000_000_000,
    )
    assert m.source_peer_id == "galbot"
```

- [ ] **Step 3: Run tests**

```bash
cd bindings/python/auki-manifests-py && maturin develop && pytest python_tests/
```

- [ ] **Step 4: Commit**

```bash
git add bindings/python/auki-manifests-py/
git commit -m "feat(auki-manifests-py): mirror source/writer split + RegistryRef adoption"
```

### Task 6.3: New `auki-session-py` package

**Files:**
- Create: `bindings/python/auki-session-py/Cargo.toml`
- Create: `bindings/python/auki-session-py/pyproject.toml`
- Create: `bindings/python/auki-session-py/src/lib.rs`
- Create: `bindings/python/auki-session-py/python_tests/test_session.py`
- Modify: `Cargo.toml` (workspace) — add binding crate

- [ ] **Step 1: Workspace + package skeleton**

Add to root `Cargo.toml`:

```toml
members = [
    # ... existing
    "bindings/python/auki-session-py",
]
```

`bindings/python/auki-session-py/Cargo.toml`:

```toml
[package]
name = "auki-session-py"
version = "0.0.0"
edition = "2024"

[lib]
name = "auki_session_py"
crate-type = ["cdylib"]

[dependencies]
auki-session = { path = "../../../crates/auki-session" }
auki-identity = { path = "../../../crates/auki-identity" }
auki-registry = { path = "../../../crates/auki-registry" }
auki-manifests = { path = "../../../crates/auki-manifests" }
pyo3 = { workspace = true, features = ["extension-module"] }
pyo3-async-runtimes = { workspace = true }
tokio = { workspace = true }
```

`bindings/python/auki-session-py/pyproject.toml`: mirror `auki-domain-py`.

- [ ] **Step 2: Implement `Session` PyO3 wrapper**

```rust
use pyo3::prelude::*;
use auki_session::Session as RustSession;
use auki_identity::PeerId;

#[pyclass]
pub struct Session(RustSession);

#[pymethods]
impl Session {
    #[new]
    fn new(peer_id: &str, app_id: &str) -> PyResult<Self> {
        let pid = PeerId::from_string(peer_id).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(RustSession::new(pid, app_id)))
    }

    fn register_sensor(&self, sensor_id: &str, body: PyObject /* TODO: typed body */) -> PyResult<RegistryRef> {
        // ... bridge body and call self.0.register_sensor
        todo!()
    }

    // Same for register_clock / frame / detector

    fn register_sensor_log<'py>(&self, py: Python<'py>, spec: PyObject) -> PyResult<SensorLogHandle> {
        // ... bridge SensorLogSpec
        todo!()
    }

    // join_domain / leave_domain async wrappers via pyo3-async-runtimes
}

#[pymodule]
fn auki_session_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Session>()?;
    Ok(())
}
```

This task lands the skeleton; method-by-method bridging is incremental. Get `Session::new` and `register_sensor` working end-to-end first.

- [ ] **Step 3: Python smoke test**

```python
# python_tests/test_session.py
import auki_session_py as auki

def test_session_new():
    s = auki.Session(peer_id="galbot", app_id="ctrl")
    assert s is not None
```

- [ ] **Step 4: Build + test**

```bash
cd bindings/python/auki-session-py && maturin develop && pytest python_tests/
```

- [ ] **Step 5: Commit**

```bash
git add bindings/python/auki-session-py/ Cargo.toml
git commit -m "feat(auki-session-py): scaffold Python binding"
```

### Task 6.4: Cross-language parity: Python regenerates same fixtures

**Files:**
- Modify: `bindings/python/auki-registry-py/python_tests/test_locked_json.py`
- Modify: `bindings/python/auki-manifests-py/python_tests/test_locked_json.py`

- [ ] **Step 1: Mirror the Rust locked fixtures**

For each Rust fixture, load it from `tests/locked/` (vendor or path-relative), construct the same value through the Python API, and assert byte-equal canonical JSON.

```python
import json, pathlib
from auki_registry import SensorRegistryEntry, Camera, RegistryRef
import auki_jcs_py as jcs

FIXTURE_DIR = pathlib.Path(__file__).parent.parent.parent.parent.parent / "crates/auki-registry/tests/locked"

def test_sensor_camera_matches_rust_fixture():
    expected = (FIXTURE_DIR / "sensor_camera.json").read_text().rstrip()
    frame = RegistryRef("galbot", "head_left_camera_optical", "framehash")
    body  = Camera(width=1920, height=1200, frame_rate_hz=30,
                   pixel_format="rgb8", color_space="srgb",
                   intrinsics_model="pinhole", distortion_model="brown_conrady",
                   frame=frame)
    e = SensorRegistryEntry(peer_id="galbot", sensor_id="head_left_rgb", body=body)
    actual = jcs.to_canonical_string(e)
    assert actual == expected
```

(Substitute `auki_jcs_py` with whatever the workspace exposes for Python-side JCS canonicalization. If it doesn't exist, use Python's `json` with `sort_keys=True, separators=(",", ":")` for the test.)

- [ ] **Step 2: Run tests**

```bash
cd bindings/python/auki-registry-py && pytest python_tests/test_locked_json.py
cd bindings/python/auki-manifests-py && pytest python_tests/test_locked_json.py
```

- [ ] **Step 3: Commit**

```bash
git add bindings/python/auki-registry-py/python_tests/ bindings/python/auki-manifests-py/python_tests/
git commit -m "test(python-bindings): cross-language parity against Rust locked fixtures"
```

### Phase 6 checkpoint

```bash
cargo test --workspace
for crate in auki-registry-py auki-manifests-py auki-session-py; do
    (cd bindings/python/$crate && maturin develop && pytest python_tests/)
done
```

Expected: all green.

---

## Phase 7 — Materialization smoke + cleanup

### Task 7.1: End-to-end smoke test

**Files:**
- Create: `crates/auki-session/tests/end_to_end.rs`

- [ ] **Step 1: Write the test**

```rust
use auki_session::{Session, SensorLogSpec, HeadSpec};
use auki_registry::*;
use auki_identity::PeerId;
use std::time::Duration;

#[tokio::test]
async fn galbot_session_writes_log_then_park_materializes() {
    let tmp = tempfile::tempdir().unwrap();

    // Galbot side
    let galbot = Session::new(PeerId::from_string("galbot").unwrap(), "galbot-ctrl")
        .with_storage_root(tmp.path().join("galbot"));

    let sensor = galbot.register_sensor("head_left_rgb", /* Camera */).unwrap();
    let clock  = galbot.register_clock("session/sdk_clock", /* MonotonicClock */).unwrap();
    let frame  = galbot.register_frame("head_left_camera_optical", /* FrameDef */).unwrap();

    let _log = galbot.register_sensor_log(SensorLogSpec {
        sensor: sensor.clone(),
        clock, frame: Some(frame),
        head: HeadSpec::Rolling { retention_ns: 5_000_000_000 },
        segment_duration: Duration::from_secs(1),
        retention: Duration::from_secs(5),
    }).unwrap();

    // Verify Galbot's manifest on disk
    let manifest_path = tmp.path().join("galbot/logs/galbot/head_left_rgb/manifest.json");
    let manifest_str = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest_str.contains(r#""source_peer_id":"galbot""#));
    assert!(manifest_str.contains(r#""writer_peer_id":"galbot""#));

    // Galbot's catalog
    let catalog = galbot.catalog();
    assert_eq!(catalog.len(), 1);
    let row = &catalog[0];
    assert_eq!(row.source_peer_id.as_str(), "galbot");
    assert_eq!(row.writer_peer_id.as_str(), "galbot");
    assert_eq!(row.resource_id, "head_left_rgb");

    // Park side — materialization stub
    // The full async materialization is deferred; assert the surface compiles.
    // Implementation will exercise this path when materialization is wired.
}
```

- [ ] **Step 2: Run**

```bash
cargo test -p auki-session --test end_to_end
```

- [ ] **Step 3: Commit**

```bash
git add crates/auki-session/tests/end_to_end.rs
git commit -m "test(auki-session): end-to-end smoke covering write + catalog + manifest"
```

### Task 7.2: Update `dataproducts.md`

**Files:**
- Modify: `dataproducts.md`

- [ ] **Step 1: Replace `parking_lot.md` references with the spec link**

```bash
grep -n "parking_lot\|216" dataproducts.md
```

- [ ] **Step 2: Drop the "Shipped v0" historical section once schema is live**

Walk the doc and replace stream-first wording with log-first wording per the spec. The current branch (`docs/210-rewrite-dataproducts`) already includes the bulk of this — verify against the locked schema.

- [ ] **Step 3: Commit**

```bash
git add dataproducts.md
git commit -m "docs(dataproducts): align with #216 locked schema"
```

### Task 7.3: Final workspace sanity

- [ ] **Step 1: Full workspace test**

```bash
cargo test --workspace
```

- [ ] **Step 2: Full clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 3: Format**

```bash
cargo fmt --all
```

- [ ] **Step 4: Run Python bindings**

```bash
for crate in auki-registry-py auki-manifests-py auki-session-py; do
    (cd bindings/python/$crate && maturin develop && pytest python_tests/) || exit 1
done
```

- [ ] **Step 5: Final commit if any formatting changes**

```bash
git status
git add -p  # selectively stage formatting fixes
git commit -m "chore: cargo fmt + clippy cleanups"
```

### Task 7.4: Open PR

- [ ] **Step 1: Push branch**

```bash
git push -u origin HEAD
```

- [ ] **Step 2: Open PR**

```bash
gh pr create --title "feat: SDK as robot data plane (#216 schema + auki-session)" --body "$(cat <<'EOF'
## Summary
- Adds peer-ownership fields across registries, manifests, and the resource catalog
- Introduces `auki-session` as the declarative app-facing crate
- Bumps protocols to `/auki/{resources,registries,stream}/0.2.0`
- Drops legacy `SensorStreamResource` / `TransformEdgeResource` / `PoseStreamResource`

Closes #216

## Test plan
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] Python binding tests pass (`auki-registry-py`, `auki-manifests-py`, `auki-session-py`)
- [ ] End-to-end smoke (`crates/auki-session/tests/end_to_end.rs`) green
- [ ] Locked JSON fixtures match between Rust and Python sides

## Migration notes
Clean cut — no backwards compatibility. Consumers (Park, Boosterapp) must wipe their on-disk registry/log caches and rebuild against this SDK version.

xoxo Broodsugar's exocortex
EOF
)"
```

- [ ] **Step 3: Move the issue card on the SDK Kanban from In Progress → In Review** (manually; the automation isn't reliable per the CLAUDE.md guidance).

---

## Self-review pass

Spec sections vs plan tasks:
- §1 (Catalog row shape) → Tasks 3.2, 3.3, 3.4, 3.6 ✓
- §2 (Registry entries) → Tasks 1.1–1.6 ✓
- §3 (Manifests) → Tasks 2.1–2.4 ✓
- §4 (auki-session) → Tasks 4.1–4.8 ✓
- §5 (Domain protocol) → Tasks 5.1, 5.2; full materialization deferred ✓
- §6 (resource_id derivation) → Task 4.6 (per-type formats), Task 1.6 (charset) ✓
- §7 (Testing & migration) → Tasks 1.5, 2.4, 3.6, 6.4, 7.1 ✓

No `TBD` / `TODO` strings except: Task 4.8's `todo!("full materialization arrives in a follow-up plan")` — that's intentional and spec'd as a deferred sub-project.

Type consistency: `RegistryRef`, `LogRef`, `SessionError`, `SessionHandle`, `Head::Rolling/Fixed`, `Kind` enum variants stay byte-equal across tasks. Tracked.
