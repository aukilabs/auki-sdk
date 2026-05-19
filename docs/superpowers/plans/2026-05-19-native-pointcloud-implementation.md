# Native Pointcloud Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ROS-CDR pointcloud stream contract with one native Auki `PointCloudFrame { point_count, data }` record used by both logs and live streams.

**Architecture:** Keep pointcloud interpretation in the Sensor Registry and sample data in `auki.point_cloud.PointCloudFrame`. Remove the stream-only pointcloud protobuf package and make `auki-network`, Python bindings, and the ROS adapter use `auki_datatypes::point_cloud::PointCloudFrame` directly. Validate the canonical XYZ layout at registry/helper boundaries while leaving the generic stream pump type-agnostic.

**Tech Stack:** Rust workspace crates, protobuf via `prost`, Python betterproto bindings in `auki-datatypes-py`, PyO3 bindings, pytest/maturin for Python surfaces.

---

## File Structure

- `crates/auki-datatypes/proto/point_cloud.proto`: replace `PointCloudLogEntry` with native shared `PointCloudFrame`.
- `crates/auki-datatypes/proto/point_cloud_stream.proto`: delete; stream pointclouds reuse `auki.point_cloud`.
- `crates/auki-datatypes/build.rs`: remove `proto/point_cloud_stream.proto`.
- `crates/auki-datatypes/src/lib.rs`: remove `point_cloud_stream`; implement `LogPayload` for `point_cloud::PointCloudFrame`; update tests.
- `crates/auki-datatypes-py/`: regenerate/update betterproto output and remove `point_cloud_stream`.
- `crates/auki-registry/src/lib.rs`: remove `PointCloud.is_bigendian`; add pointcloud layout validation.
- `crates/auki-registry-py/src/lib.rs`: remove `is_bigendian` from `point_cloud_sensor_entry`.
- `crates/auki-network/src/stream_protocol.rs`: re-export native `PointCloudFrame`; update locked stream vectors.
- `crates/auki-network/src/stream_runtime.rs`: update pointcloud stream fixtures/tests to use `point_count` and `data`.
- `crates/auki-domain/src/cluster_manager.rs`: update pointcloud resource payload hint to the native package path.
- `crates/auki-network-py/src/stream_types.rs`: expose `PointCloudFrame(point_count, data)` with `.point_count` and `.data`.
- `crates/auki-domain-py/src/lib.rs`: update pointcloud stream docs/tests; the method shape stays.
- `crates/auki-ros-adapter/src/lib.rs`: convert `PointCloud2Msg` into native `PointCloudFrame`.
- README/changelog/src-readme files under affected crates: document the intentional wire break and native invariant.

---

### Task 1: Native Pointcloud Protobuf And Rust Datatypes

**Files:**
- Modify: `crates/auki-datatypes/proto/point_cloud.proto`
- Delete: `crates/auki-datatypes/proto/point_cloud_stream.proto`
- Modify: `crates/auki-datatypes/build.rs`
- Modify: `crates/auki-datatypes/src/lib.rs`
- Test: `crates/auki-datatypes/src/lib.rs`

- [ ] **Step 1: Write the failing Rust locked-vector tests**

In `crates/auki-datatypes/src/lib.rs`, replace the pointcloud test import:

```rust
use super::point_cloud::PointCloudFrame;
```

Replace the old `PointCloudLogEntry` tests with:

```rust
fn native_point_cloud_frame() -> PointCloudFrame {
    PointCloudFrame {
        point_count: 2,
        data: vec![
            0x00, 0x00, 0x80, 0x3f,
            0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x40, 0x40,
            0x00, 0x00, 0x80, 0x40,
            0x00, 0x00, 0xa0, 0x40,
            0x00, 0x00, 0xc0, 0x40,
        ],
    }
}

#[test]
fn point_cloud_frame_serializes_to_locked_wire_bytes() {
    let bytes = native_point_cloud_frame().encode_to_vec();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(
        hex,
        "080212180000803f0000004000004040000080400000a0400000c040"
    );
}

#[test]
fn point_cloud_frame_hash_is_locked() {
    let bytes = native_point_cloud_frame().encode_to_vec();
    assert_eq!(
        auki_hash::hash_jcs_bytes(&bytes),
        "f629645289882067aece1781d34cec92"
    );
}

#[test]
fn point_cloud_frame_round_trips() {
    let entry = native_point_cloud_frame();
    let bytes = entry.encode_to_vec();
    let decoded = PointCloudFrame::decode(&*bytes).expect("decode");
    assert_eq!(decoded, entry);
}

#[test]
fn point_cloud_frame_log_payload_round_trips() {
    use auki_logs::LogPayload;
    let entry = native_point_cloud_frame();
    let bytes = LogPayload::encode(&entry);
    let decoded = <PointCloudFrame as LogPayload>::decode(&bytes).expect("decode");
    assert_eq!(decoded, entry);
}

#[test]
fn point_cloud_frame_empty_data_round_trips() {
    let entry = PointCloudFrame {
        point_count: 0,
        data: vec![],
    };
    let bytes = entry.encode_to_vec();
    assert_eq!(bytes.len(), 0);
    let decoded = PointCloudFrame::decode(&*bytes).expect("decode");
    assert_eq!(decoded, entry);
}

#[test]
fn point_cloud_frame_segment_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = serde_json::json!({
        "segment_duration_ns": 1_000_000_000i64,
        "retention_ns": 60_000_000_000i64,
        "kind": "test"
    });
    {
        let mut log: auki_logs::Log<PointCloudFrame> =
            auki_logs::Log::open(dir.path(), manifest).unwrap();
        log.append(100, &native_point_cloud_frame()).unwrap();
        log.append(
            200,
            &PointCloudFrame {
                point_count: 0,
                data: vec![],
            },
        )
        .unwrap();
    }
    let reader: auki_logs::LogReader<PointCloudFrame> =
        auki_logs::Log::<PointCloudFrame>::read(dir.path()).unwrap();
    let entries = reader.entries().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].timestamp_ns, 100);
    assert_eq!(entries[0].payload, native_point_cloud_frame());
    assert_eq!(entries[1].timestamp_ns, 200);
    assert_eq!(entries[1].payload.point_count, 0);
    assert_eq!(entries[1].payload.data, Vec::<u8>::new());
}
```

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
cargo test -p auki-datatypes point_cloud -- --nocapture
```

Expected: compile failure because `auki_datatypes::point_cloud::PointCloudFrame` is not generated yet.

- [ ] **Step 3: Write the protobuf implementation**

Replace `crates/auki-datatypes/proto/point_cloud.proto` with:

```proto
syntax = "proto3";

package auki.point_cloud;

// Native Auki pointcloud sample used by both Sensor Logs and
// `/auki/stream/0.1.0` pointcloud substreams.
//
// `data` is `point_count` packed point records. The fixed per-stream
// layout lives in the pinned `SensorBody::PointCloud` registry entry:
// `fields`, `point_step`, `frame_id`, and `frame_hash`. Numeric fields
// in Auki-native pointclouds are little-endian by contract. Every valid
// pointcloud layout starts with `x`, `y`, `z` as float32 at offsets
// 0, 4, and 8.
//
// Field number ledger:
//   PointCloudFrame.point_count = 1
//   PointCloudFrame.data        = 2

message PointCloudFrame {
  uint32 point_count = 1;
  bytes data = 2;
}
```

Delete `crates/auki-datatypes/proto/point_cloud_stream.proto`.

In `crates/auki-datatypes/build.rs`, remove:

```rust
"proto/point_cloud_stream.proto",
```

In `crates/auki-datatypes/src/lib.rs`, replace the pointcloud module with:

```rust
/// `auki.point_cloud` — native pointcloud sample used by both Sensor
/// Logs and libp2p `/auki/stream/0.1.0` pointcloud substreams.
pub mod point_cloud {
    include!(concat!(env!("OUT_DIR"), "/auki.point_cloud.rs"));
}

impl_log_payload!(point_cloud::PointCloudFrame);
```

Remove the old `pub mod point_cloud_stream` block.

- [ ] **Step 4: Run tests to verify pass**

Run:

```bash
cargo test -p auki-datatypes point_cloud -- --nocapture
```

Expected: all pointcloud datatypes tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-datatypes/proto/point_cloud.proto \
        crates/auki-datatypes/proto/point_cloud_stream.proto \
        crates/auki-datatypes/build.rs \
        crates/auki-datatypes/src/lib.rs
git commit -m "feat: make pointcloud frames native"
```

---

### Task 2: Rust Registry Pointcloud Layout Validation

**Files:**
- Modify: `crates/auki-registry/src/lib.rs`
- Test: `crates/auki-registry/src/lib.rs`

- [ ] **Step 1: Write failing registry validation tests**

Add these tests to the existing `crates/auki-registry/src/lib.rs` test module:

```rust
fn canonical_xyz_fields() -> Vec<PointField> {
    vec![
        PointField {
            name: "x".into(),
            offset: 0,
            datatype: PointFieldDataType::Float32,
            count: 1,
        },
        PointField {
            name: "y".into(),
            offset: 4,
            datatype: PointFieldDataType::Float32,
            count: 1,
        },
        PointField {
            name: "z".into(),
            offset: 8,
            datatype: PointFieldDataType::Float32,
            count: 1,
        },
    ]
}

#[test]
fn pointcloud_layout_accepts_canonical_xyz_prefix() {
    let pc = PointCloud {
        fields: canonical_xyz_fields(),
        point_step: 12,
        frame_rate_hz: 30,
        frame_id: "frame/points".into(),
        frame_hash: "hash".into(),
    };
    pc.validate_layout().unwrap();
}

#[test]
fn pointcloud_layout_rejects_missing_canonical_xyz_prefix() {
    let pc = PointCloud {
        fields: vec![PointField {
            name: "x".into(),
            offset: 0,
            datatype: PointFieldDataType::Float32,
            count: 1,
        }],
        point_step: 12,
        frame_rate_hz: 30,
        frame_id: "frame/points".into(),
        frame_hash: "hash".into(),
    };
    match pc.validate_layout() {
        Err(Error::InvalidPointCloudLayout(msg)) => {
            assert!(msg.contains("y"), "message should name missing y: {msg}");
        }
        other => panic!("expected InvalidPointCloudLayout; got {other:?}"),
    }
}

#[test]
fn pointcloud_layout_rejects_overlapping_fields() {
    let mut fields = canonical_xyz_fields();
    fields.push(PointField {
        name: "confidence".into(),
        offset: 8,
        datatype: PointFieldDataType::Float32,
        count: 1,
    });
    let pc = PointCloud {
        fields,
        point_step: 16,
        frame_rate_hz: 30,
        frame_id: "frame/points".into(),
        frame_hash: "hash".into(),
    };
    match pc.validate_layout() {
        Err(Error::InvalidPointCloudLayout(msg)) => {
            assert!(msg.contains("overlap"), "message should mention overlap: {msg}");
        }
        other => panic!("expected InvalidPointCloudLayout; got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p auki-registry pointcloud_layout -- --nocapture
```

Expected: compile failure because `PointCloud::validate_layout` and `Error::InvalidPointCloudLayout` do not exist.

- [ ] **Step 3: Write minimal registry implementation**

Remove `pub is_bigendian: bool` from `PointCloud`.

Add this error variant to `Error`:

```rust
InvalidPointCloudLayout(String),
```

Add this display arm:

```rust
Error::InvalidPointCloudLayout(msg) => write!(f, "invalid pointcloud layout: {msg}"),
```

Add this implementation near the pointcloud types:

```rust
impl PointCloud {
    pub fn validate_layout(&self) -> Result<()> {
        if self.point_step < 12 {
            return Err(Error::InvalidPointCloudLayout(format!(
                "point_step must be at least 12 bytes for canonical XYZ; got {}",
                self.point_step
            )));
        }
        require_xyz_field(&self.fields, "x", 0)?;
        require_xyz_field(&self.fields, "y", 4)?;
        require_xyz_field(&self.fields, "z", 8)?;
        validate_field_bounds_and_overlaps(&self.fields, self.point_step)?;
        Ok(())
    }
}

fn require_xyz_field(fields: &[PointField], name: &str, offset: u32) -> Result<()> {
    let Some(field) = fields.iter().find(|f| f.name == name) else {
        return Err(Error::InvalidPointCloudLayout(format!(
            "missing required {name}:float32 field at offset {offset}"
        )));
    };
    if field.offset != offset || field.datatype != PointFieldDataType::Float32 || field.count != 1 {
        return Err(Error::InvalidPointCloudLayout(format!(
            "field {name:?} must be float32 count=1 at offset {offset}; got offset={} datatype={:?} count={}",
            field.offset, field.datatype, field.count
        )));
    }
    Ok(())
}

fn validate_field_bounds_and_overlaps(fields: &[PointField], point_step: u32) -> Result<()> {
    let mut spans: Vec<(&str, u32, u32)> = Vec::new();
    for field in fields {
        let width = field.datatype.byte_width().checked_mul(field.count).ok_or_else(|| {
            Error::InvalidPointCloudLayout(format!(
                "field {:?} byte width overflows u32",
                field.name
            ))
        })?;
        if width == 0 {
            return Err(Error::InvalidPointCloudLayout(format!(
                "field {:?} count must be greater than zero",
                field.name
            )));
        }
        let end = field.offset.checked_add(width).ok_or_else(|| {
            Error::InvalidPointCloudLayout(format!("field {:?} offset overflows u32", field.name))
        })?;
        if end > point_step {
            return Err(Error::InvalidPointCloudLayout(format!(
                "field {:?} ends at byte {}, beyond point_step {}",
                field.name, end, point_step
            )));
        }
        spans.push((&field.name, field.offset, end));
    }
    spans.sort_by_key(|(_, start, _)| *start);
    for pair in spans.windows(2) {
        let (a_name, _a_start, a_end) = pair[0];
        let (b_name, b_start, _b_end) = pair[1];
        if a_end > b_start {
            return Err(Error::InvalidPointCloudLayout(format!(
                "field {a_name:?} overlaps field {b_name:?}"
            )));
        }
    }
    Ok(())
}
```

In `validate_sensor_frame_reference`, validate pointcloud layouts before the frame lookup:

```rust
SensorBody::PointCloud(point_cloud) => {
    point_cloud.validate_layout()?;
    Some((&point_cloud.frame_id, &point_cloud.frame_hash))
}
```

Keep the existing RGB camera and non-spatial sensor behavior unchanged.

- [ ] **Step 4: Run tests to verify pass**

```bash
cargo test -p auki-registry pointcloud_layout -- --nocapture
cargo test -p auki-registry
```

Expected: all registry tests pass after existing pointcloud fixtures remove `is_bigendian`.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-registry/src/lib.rs
git commit -m "feat: validate native pointcloud layouts"
```

---

### Task 3: Python Datatypes And Registry Bindings

**Files:**
- Modify: `crates/auki-datatypes-py/auki_datatypes/__init__.py`
- Modify: `crates/auki-datatypes-py/auki_datatypes/auki/point_cloud.py`
- Delete: `crates/auki-datatypes-py/auki_datatypes/auki/point_cloud_stream.py`
- Modify: `crates/auki-datatypes-py/tests/test_locked_vectors.py`
- Modify: `crates/auki-registry-py/src/lib.rs`
- Modify: `crates/auki-registry-py/python_tests/test_registry.py`

- [ ] **Step 1: Regenerate betterproto files**

```bash
cd crates/auki-datatypes-py
./regen.sh
```

Expected: `auki_datatypes/auki/point_cloud.py` defines `PointCloudFrame`, and `auki_datatypes/auki/point_cloud_stream.py` is gone.

- [ ] **Step 2: Write Python locked-vector test**

In `crates/auki-datatypes-py/tests/test_locked_vectors.py`, replace the pointcloud test with:

```python
def test_point_cloud_frame_locked_wire_bytes():
    entry = adt.point_cloud.PointCloudFrame(
        point_count=2,
        data=bytes(
            [
                0x00, 0x00, 0x80, 0x3F,
                0x00, 0x00, 0x00, 0x40,
                0x00, 0x00, 0x40, 0x40,
                0x00, 0x00, 0x80, 0x40,
                0x00, 0x00, 0xA0, 0x40,
                0x00, 0x00, 0xC0, 0x40,
            ]
        ),
    )
    expected = "080212180000803f0000004000004040000080400000a0400000c040"
    assert bytes(entry).hex() == expected
```

Update module-shape assertions so `"point_cloud_stream"` is absent.

- [ ] **Step 3: Run Python datatypes tests**

```bash
cd crates/auki-datatypes-py
pytest tests/
```

Expected: all Python datatypes locked-vector tests pass.

- [ ] **Step 4: Update registry Python tests first**

In `crates/auki-registry-py/python_tests/test_registry.py`, remove `is_bigendian=False` from every `point_cloud_sensor_entry(...)` call. Add:

```python
def test_point_cloud_sensor_rejects_non_xyz_layout(tmp_path: pathlib.Path) -> None:
    import auki_registry

    frame = auki_registry.frame_ros_optical(FRAME_ID)
    frame_hash = auki_registry.write_frame(tmp_path, frame)
    sensor = auki_registry.point_cloud_sensor_entry(
        sensor_id="K1-AABBCCDDEEFF/head_depth_points",
        fields=[auki_registry.point_field("x", 0, "float32")],
        point_step=4,
        frame_rate_hz=10,
        frame_id=FRAME_ID,
        frame_hash=frame_hash,
    )

    with pytest.raises(ValueError, match="invalid pointcloud layout"):
        auki_registry.write_sensor(tmp_path, sensor)
```

- [ ] **Step 5: Update PyO3 registry constructor**

In `crates/auki-registry-py/src/lib.rs`, replace `point_cloud_sensor_entry` with:

```rust
#[pyfunction]
#[pyo3(signature = (*, sensor_id, fields, point_step, frame_rate_hz, frame_id, frame_hash))]
fn point_cloud_sensor_entry(
    py: Python<'_>,
    sensor_id: &str,
    fields: &Bound<'_, PyAny>,
    point_step: u32,
    frame_rate_hz: u32,
    frame_id: &str,
    frame_hash: &str,
) -> PyResult<PyObject> {
    let fields: Vec<registry::PointField> = parse_py(py, fields, "fields")?;
    let entry = registry::SensorRegistryEntry {
        sensor_id: sensor_id.to_string(),
        body: registry::SensorBody::PointCloud(registry::PointCloud {
            fields,
            point_step,
            frame_rate_hz,
            frame_id: frame_id.to_string(),
            frame_hash: frame_hash.to_string(),
        }),
    };
    struct_to_pyobject(py, &entry)
}
```

Update Rust tests in the same file to remove the `is_bigendian` argument and kwargs.

- [ ] **Step 6: Run tests to verify pass**

```bash
cargo test -p auki-registry-py point_cloud -- --nocapture
cd crates/auki-registry-py
pytest python_tests/
```

Expected: Rust PyO3 tests and Python registry tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-datatypes-py \
        crates/auki-registry-py/src/lib.rs \
        crates/auki-registry-py/python_tests/test_registry.py
git commit -m "feat: expose native pointcloud metadata to Python"
```

---

### Task 4: Network, Domain, And Python Stream Surfaces

**Files:**
- Modify: `crates/auki-network/src/stream_protocol.rs`
- Modify: `crates/auki-network/src/stream_runtime.rs`
- Modify: `crates/auki-domain/src/cluster_manager.rs`
- Modify: `crates/auki-network-py/src/stream_types.rs`
- Modify: `crates/auki-network-py/python_tests/test_streams.py`
- Modify: `crates/auki-domain-py/src/lib.rs`
- Modify: `crates/auki-domain-py/python_tests/test_surface.py`

- [ ] **Step 1: Write failing Rust stream tests**

In `crates/auki-network/src/stream_protocol.rs` and `crates/auki-network/src/stream_runtime.rs`, update pointcloud fixtures to construct:

```rust
PointCloudFrame {
    point_count: 1,
    data: vec![
        0x00, 0x00, 0x80, 0x3f,
        0x00, 0x00, 0x00, 0x40,
        0x00, 0x00, 0x40, 0x40,
    ],
}
```

Change assertions from `.bytes` to `.data` and add `assert_eq!(payload.point_count, 1)`.

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p auki-network stream_protocol::tests::point_cloud --features swarm -- --nocapture
```

Expected: compile failure until `PointCloudFrame` is re-exported from `auki_datatypes::point_cloud`.

- [ ] **Step 3: Update Rust stream implementation**

In `crates/auki-network/src/stream_protocol.rs`, replace:

```rust
pub use auki_datatypes::point_cloud_stream::PointCloudFrame;
```

with:

```rust
pub use auki_datatypes::point_cloud::PointCloudFrame;
```

Update comments in `stream_protocol.rs` and `stream_runtime.rs` so pointclouds are described as native packed Auki point records, not CDR `PointCloud2`.

- [ ] **Step 4: Update resource payload hint**

In `crates/auki-domain/src/cluster_manager.rs`, update:

```rust
"point_cloud" => "auki.point_cloud.PointCloudFrame",
```

inside `stream_payload_for_sensor_kind`.

- [ ] **Step 5: Write failing PyO3 pointcloud stream tests**

In `crates/auki-network-py/src/stream_types.rs`, update the pointcloud PyClass test to:

```rust
let payload = PyBytes::new_bound(py, &[
    0x00, 0x00, 0x80, 0x3f,
    0x00, 0x00, 0x00, 0x40,
    0x00, 0x00, 0x40, 0x40,
]);
let f = PyPointCloudFrame::new(1, payload);
assert_eq!(f.point_count(), 1);
assert_eq!(f.data(py).as_bytes().len(), 12);
assert_eq!(f.__len__(), 1);
assert_eq!(f.__repr__(), "PointCloudFrame(point_count=1, data=<12 bytes>)");
```

In `crates/auki-network-py/python_tests/test_streams.py`, replace the surface test with:

```python
def test_pointcloud_frame_carries_point_count_and_data() -> None:
    data = b"\x00\x00\x80?\x00\x00\x00@\x00\x00@@"
    f = cluster.PointCloudFrame(1, data)
    assert f.point_count == 1
    assert f.data == data
    assert len(f) == 1
    assert "PointCloudFrame" in repr(f)
```

- [ ] **Step 6: Update PyO3 pointcloud frame implementation**

Replace `PyPointCloudFrame` with:

```rust
#[pyclass(name = "PointCloudFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PyPointCloudFrame {
    pub(crate) inner: RustPointCloudFrame,
}

#[pymethods]
impl PyPointCloudFrame {
    #[new]
    #[pyo3(signature = (point_count, data, /))]
    fn new(point_count: u32, data: Bound<'_, PyBytes>) -> Self {
        Self {
            inner: RustPointCloudFrame {
                point_count,
                data: data.as_bytes().to_vec(),
            },
        }
    }

    #[getter]
    fn point_count(&self) -> u32 {
        self.inner.point_count
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.data)
    }

    fn __len__(&self) -> usize {
        self.inner.point_count as usize
    }

    fn __repr__(&self) -> String {
        format!(
            "PointCloudFrame(point_count={}, data=<{} bytes>)",
            self.inner.point_count,
            self.inner.data.len()
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
```

Update `to_rust_pointcloud`, `from_rust_pointcloud`, direct tests, and Python stream tests to use `.data`.

- [ ] **Step 7: Update domain-py docs/tests**

In `crates/auki-domain-py/src/lib.rs`, update comments for `open_pointcloud_stream` so they say `PointCloudFrame(point_count, data)`.

In `crates/auki-domain-py/python_tests/test_surface.py`, update pointcloud resource payload expectations to `"auki.point_cloud.PointCloudFrame"`.

- [ ] **Step 8: Run tests to verify pass**

```bash
cargo test -p auki-network stream_protocol::tests::point_cloud --features swarm -- --nocapture
cargo test -p auki-network stream_runtime::tests::producer_accepts_and_streams_pointcloud_frames --features swarm -- --nocapture
cargo test -p auki-network stream_runtime::tests::one_producer_serves_camera_and_pointcloud_via_sensor_id_dispatch --features swarm -- --nocapture
cargo test -p auki-network-py point_cloud -- --nocapture
cd crates/auki-network-py
pytest python_tests/test_streams.py
```

Expected: all listed tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/auki-network/src/stream_protocol.rs \
        crates/auki-network/src/stream_runtime.rs \
        crates/auki-domain/src/cluster_manager.rs \
        crates/auki-network-py/src/stream_types.rs \
        crates/auki-network-py/python_tests/test_streams.py \
        crates/auki-domain-py/src/lib.rs \
        crates/auki-domain-py/python_tests/test_surface.py
git commit -m "feat: stream native pointcloud frames"
```

---

### Task 5: ROS Adapter Native Pointcloud Conversion

**Files:**
- Modify: `crates/auki-ros-adapter/src/lib.rs`
- Modify: `crates/auki-ros-adapter/README.md`
- Modify: `crates/auki-ros-adapter/src/readme.md`

- [ ] **Step 1: Write failing ROS adapter tests**

Rename pointcloud tests from `build_point_cloud_log_entry_*` to `build_point_cloud_frame_*` and update the core timestamp/data test:

```rust
#[test]
fn build_point_cloud_frame_extracts_timestamp_count_and_data() {
    let msg = xyz_pc2(2);
    let (ts, frame) = build_point_cloud_frame(&msg);
    assert_eq!(ts, stamp_to_ns(msg.stamp));
    assert_eq!(frame.point_count, 2);
    assert_eq!(frame.data.len(), 24);
}
```

For RGB normalization tests, assert `frame.data` instead of `log.data`.

Add a big-endian source conversion test:

```rust
#[test]
fn build_point_cloud_frame_converts_big_endian_xyz_to_little_endian() {
    let msg = PointCloud2Msg {
        stamp: StampMsg { sec: 1, nanosec: 0 },
        height: 1,
        width: 1,
        fields: vec![
            PointFieldMsg { name: "x".into(), offset: 0, datatype: 7, count: 1 },
            PointFieldMsg { name: "y".into(), offset: 4, datatype: 7, count: 1 },
            PointFieldMsg { name: "z".into(), offset: 8, datatype: 7, count: 1 },
        ],
        is_bigendian: true,
        point_step: 12,
        row_step: 12,
        data: vec![
            0x3f, 0x80, 0x00, 0x00,
            0x40, 0x00, 0x00, 0x00,
            0x40, 0x40, 0x00, 0x00,
        ],
        is_dense: true,
    };
    let (_ts, frame) = build_point_cloud_frame(&msg);
    assert_eq!(frame.point_count, 1);
    assert_eq!(
        frame.data,
        vec![
            0x00, 0x00, 0x80, 0x3f,
            0x00, 0x00, 0x00, 0x40,
            0x00, 0x00, 0x40, 0x40,
        ]
    );
}
```

- [ ] **Step 2: Run test to verify failure**

```bash
cargo test -p auki-ros-adapter point_cloud -- --nocapture
```

Expected: compile failure until `build_point_cloud_frame` exists and registry structs remove `is_bigendian`.

- [ ] **Step 3: Write implementation**

Replace:

```rust
pub use auki_datatypes::point_cloud::PointCloudLogEntry;
```

with:

```rust
pub use auki_datatypes::point_cloud::PointCloudFrame;
```

Remove `is_bigendian: msg.is_bigendian` from `build_point_cloud_registry_entry`.

Change `NormalizationPlan::PassThrough` so it knows element width and count:

```rust
enum NormalizationPlan {
    PassThrough {
        src_offset: u32,
        dst_offset: u32,
        element_width: u32,
        count: u32,
    },
    ExpandRgb {
        src_offset: u32,
        dst_offset: u32,
    },
    ExpandRgba {
        src_offset: u32,
        dst_offset: u32,
    },
}
```

In `normalize_layout`, build the pass-through plan with:

```rust
let datatype = ros_datatype_to_sdk(f.datatype);
let element_width = datatype.byte_width();
let size = element_width * f.count;
fields.push(PointField {
    name: f.name.clone(),
    offset: dst_offset,
    datatype,
    count: f.count,
});
plans.push(NormalizationPlan::PassThrough {
    src_offset: f.offset,
    dst_offset,
    element_width,
    count: f.count,
});
dst_offset += size;
```

Change `apply_normalization` to accept source endianness:

```rust
fn apply_normalization(
    plans: &[NormalizationPlan],
    src_data: &[u8],
    src_point_step: u32,
    num_points: usize,
    dst_point_step: u32,
    src_is_bigendian: bool,
) -> Vec<u8> {
    let dst_step = dst_point_step as usize;
    let src_step = src_point_step as usize;
    let mut out = vec![0u8; num_points * dst_step];

    for p in 0..num_points {
        let src_base = p * src_step;
        let dst_base = p * dst_step;
        for plan in plans {
            match *plan {
                NormalizationPlan::PassThrough {
                    src_offset,
                    dst_offset,
                    element_width,
                    count,
                } => {
                    let width = element_width as usize;
                    for i in 0..count as usize {
                        let so = src_base + src_offset as usize + i * width;
                        let d = dst_base + dst_offset as usize + i * width;
                        out[d..d + width].copy_from_slice(&src_data[so..so + width]);
                        if src_is_bigendian && width > 1 {
                            out[d..d + width].reverse();
                        }
                    }
                }
                NormalizationPlan::ExpandRgb { src_offset, dst_offset } => {
                    let so = src_base + src_offset as usize;
                    let d = dst_base + dst_offset as usize;
                    if src_is_bigendian {
                        out[d] = src_data[so + 1];
                        out[d + 1] = src_data[so + 2];
                        out[d + 2] = src_data[so + 3];
                    } else {
                        out[d] = src_data[so + 2];
                        out[d + 1] = src_data[so + 1];
                        out[d + 2] = src_data[so];
                    }
                }
                NormalizationPlan::ExpandRgba { src_offset, dst_offset } => {
                    let so = src_base + src_offset as usize;
                    let d = dst_base + dst_offset as usize;
                    if src_is_bigendian {
                        out[d] = src_data[so + 1];
                        out[d + 1] = src_data[so + 2];
                        out[d + 2] = src_data[so + 3];
                        out[d + 3] = src_data[so];
                    } else {
                        out[d] = src_data[so + 2];
                        out[d + 1] = src_data[so + 1];
                        out[d + 2] = src_data[so];
                        out[d + 3] = src_data[so + 3];
                    }
                }
            }
        }
    }
    out
}
```

Replace `build_point_cloud_log_entry` with:

```rust
pub fn build_point_cloud_frame(msg: &PointCloud2Msg) -> (i64, PointCloudFrame) {
    let timestamp_ns = stamp_to_ns(msg.stamp);
    let normalized = normalize_layout(&msg.fields);
    let num_points = (msg.width as usize).saturating_mul(msg.height as usize);
    let data = apply_normalization(
        &normalized.plans,
        &msg.data,
        msg.point_step,
        num_points,
        normalized.point_step,
        msg.is_bigendian,
    );
    let frame = PointCloudFrame {
        point_count: num_points as u32,
        data,
    };
    (timestamp_ns, frame)
}
```

Keep `PointCloud2Msg.is_bigendian` on the ROS mirror struct because it describes source data. The native output must be little-endian.

- [ ] **Step 4: Run tests to verify pass**

```bash
cargo test -p auki-ros-adapter point_cloud -- --nocapture
cargo test -p auki-ros-adapter
```

Expected: all ROS adapter tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-ros-adapter/src/lib.rs \
        crates/auki-ros-adapter/README.md \
        crates/auki-ros-adapter/src/readme.md
git commit -m "feat: convert ROS pointclouds to native frames"
```

---

### Task 6: Documentation And Changelog Propagation

**Files:**
- Modify: `README.md`
- Modify: `crates/README.md`
- Modify: affected crate README/src-readme files
- Modify: affected `changelog.md` files from each touched crate up to root

- [ ] **Step 1: Update active docs**

Replace active documentation of the retired CDR contract with:

```markdown
Pointcloud logs and live streams carry `auki.point_cloud.PointCloudFrame { point_count, data }`.
`data` is `point_count` packed records using the fixed layout declared by
`SensorBody::PointCloud`. A valid native Auki pointcloud layout starts with
little-endian `x/y/z` float32 fields at offsets `0/4/8`; producers convert
external formats such as ROS `PointCloud2` at the adapter boundary.
```

Update `SensorBody::PointCloud` examples to remove `is_bigendian`.

- [ ] **Step 2: Update leaf changelogs**

Add this entry to each touched crate changelog, with the crate-specific details in the first sentence:

```markdown
### Nils's codex · May 19, HKT, 2026

Native pointcloud frames replace the old ROS-CDR stream contract. `PointCloudFrame { point_count, data }` now lives in `auki.point_cloud` and is used for both logs and streams; `SensorBody::PointCloud` drops `is_bigendian` because Auki-native numeric fields are little-endian by contract.
```

Apply to the datatypes, datatypes-py, registry, registry-py, network, network-py, domain, domain-py, and ros-adapter changelogs.

- [ ] **Step 3: Propagate changelog one-liners upward**

Add this root-level one-liner and a matching crate-level one-liner to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Pointclouds are now native Auki records instead of ROS CDR streams.** Logs and live streams share `auki.point_cloud.PointCloudFrame { point_count, data }`; point layout is fixed by `SensorBody::PointCloud` with little-endian canonical XYZ. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 4: Run documentation checks**

```bash
rg -n "PointCloudLogEntry|point_cloud_stream|raw CDR|CDR-encoded|is_bigendian" README.md crates docs
git diff --check
```

Expected: remaining matches are historical changelog entries or explicitly marked as the retired contract.

- [ ] **Step 5: Commit**

```bash
git add README.md docs crates
git commit -m "docs: document native pointcloud wire break"
```

---

### Task 7: Full Verification Sweep

**Files:**
- No planned source edits.

- [ ] **Step 1: Run focused Rust tests**

```bash
cargo test -p auki-datatypes point_cloud -- --nocapture
cargo test -p auki-registry pointcloud_layout -- --nocapture
cargo test -p auki-network stream_protocol::tests::point_cloud --features swarm -- --nocapture
cargo test -p auki-network stream_runtime::tests::producer_accepts_and_streams_pointcloud_frames --features swarm -- --nocapture
cargo test -p auki-network stream_runtime::tests::one_producer_serves_camera_and_pointcloud_via_sensor_id_dispatch --features swarm -- --nocapture
cargo test -p auki-ros-adapter point_cloud -- --nocapture
```

Expected: all focused tests pass.

- [ ] **Step 2: Run focused Python/PyO3 tests**

```bash
cargo test -p auki-registry-py point_cloud -- --nocapture
cargo test -p auki-network-py point_cloud -- --nocapture
cd crates/auki-datatypes-py && pytest tests/
cd ../auki-registry-py && pytest python_tests/
cd ../auki-network-py && pytest python_tests/test_streams.py
```

Expected: all focused Python and PyO3 tests pass.

- [ ] **Step 3: Run broader workspace checks**

```bash
cargo test -p auki-datatypes
cargo test -p auki-registry
cargo test -p auki-network --features swarm
cargo test -p auki-domain
cargo test -p auki-ros-adapter
git diff --check
```

Expected: all commands pass.

- [ ] **Step 4: Record verification outcome**

If all verification commands pass without edits, leave the branch as-is. If verification forces additional edits, return to the task that owns the failing file and make a normal task-scoped fix commit there.
