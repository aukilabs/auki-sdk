# Live Pose Stream Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add first-class live `pose_stream` discovery and `auki.pose.SpatialTransform` streaming so RoboStreamer can publish Galbot G1 `base_link -> head_left_rgb_optical` motion for Park.

**Architecture:** Keep `/auki/stream/0.1.0` as the transport and add additive request/manifest fields for resource-addressed pose streams. Add `PoseStreamResource` to `/auki/resources/0.0.1`, then thread `pose::SpatialTransform` through Rust stream dispatch, `auki-domain` resource enrichment, and Python producer/consumer bindings.

**Tech Stack:** Rust 2024 workspace, prost-generated protobuf via `auki-datatypes`, libp2p stream runtime in `auki-network`, `auki-domain` resource catalogs, PyO3 bindings in `auki-network-py` and `auki-domain-py`, Cargo tests, Python binding tests where available.

---

## File Structure

- `crates/auki-datatypes/proto/stream.proto`: add `StreamRequest.resource_id` and pose identity fields on `StreamManifest`; update field-number ledger comments.
- `crates/auki-network/src/stream_protocol.rs`: adjust docs/tests for additive stream fields and re-export `auki_datatypes::pose`.
- `crates/auki-network/src/stream_runtime.rs`: add `StreamDispatch::AcceptPose`, producer pump arm, and an end-to-end `SpatialTransform` stream test.
- `crates/auki-network/src/resources_protocol.rs`: add `ResourceKind::PoseStream`, `ResourceEntry::PoseStream`, `PoseStreamResource`, `ResourcesRequest.include_clock_entries`, helpers, JSON round-trip tests.
- `crates/auki-domain/src/lib.rs`: re-export `PoseStreamResource`.
- `crates/auki-domain/src/cluster_manager.rs`: enrich `PoseStreamResource` from frame and clock registries; include clock-entry request plumbing; add tests.
- `bindings/python/auki-network-py/src/stream_types.rs`: add `SpatialTransformFrame`, payload union case, `accept_pose`, retained source exclusion, entry stream handling, tests.
- `bindings/python/auki-network-py/src/lib.rs`: ensure module registration exposes the new Python stream type through `cluster`.
- `bindings/python/auki-domain-py/src/lib.rs`: add `PoseStreamResource`, extraction/conversion, generic stream resolution, `open_pose_stream`, and tests.
- `bindings/python/auki-domain-py/python_tests/test_surface.py` and `bindings/python/auki-network-py/python_tests/test_streams.py`: assert public Python names exist.
- Active docs: `README.md`, `Vision.md`, `crates/auki-network/README.md`, `crates/auki-domain/README.md`, `bindings/python/auki-network-py/README.md`, `bindings/python/auki-domain-py/README.md` if implementation changes their public surface claims.

## Task 1: Protobuf And Resource Catalog Foundation

**Files:**
- Modify: `crates/auki-datatypes/proto/stream.proto`
- Modify: `crates/auki-network/src/resources_protocol.rs`
- Modify: `crates/auki-network/src/stream_protocol.rs`

- [ ] **Step 1: Write failing resource catalog tests**

In `crates/auki-network/src/resources_protocol.rs`, extend `response_round_trips_with_sensor_and_transform` so the response also contains a pose stream row:

```rust
ResourceEntry::PoseStream(PoseStreamResource {
    id: "K1-LIVE01/base_link->K1-LIVE01/head_left_rgb_optical".into(),
    from_frame_id: "K1-LIVE01/base_link".into(),
    from_frame_hash: "basehash".into(),
    to_frame_id: "K1-LIVE01/head_left_rgb_optical".into(),
    to_frame_hash: "headhash".into(),
    clock_id: "K1-LIVE01/monotonic".into(),
    clock_hash: "clockhash".into(),
    stream_protocol: "/auki/stream/0.1.0".into(),
    payload: "spatial_transform".into(),
    writer_mode: "movable".into(),
    expected_rate_hz: 30,
    source: Some(serde_json::json!({
        "kind": "ros2_tf",
        "publishers": ["robot_state_publisher"]
    })),
    from_frame_entry_json: None,
    to_frame_entry_json: None,
    clock_entry_json: None,
}),
```

Also extend `resource_kinds_are_stable`:

```rust
assert_eq!(ResourceKind::PoseStream.as_str(), "pose_stream");
```

Add a request helper assertion to `request_round_trips`:

```rust
let req = ResourcesRequest::pose_streams().with_registry_entries();
let mut buf = Vec::new();
write_resources_request(&mut buf, &req).await.unwrap();
let mut cursor = futures::io::Cursor::new(buf);
let back = read_resources_request(&mut cursor).await.unwrap();
assert_eq!(back.kinds, vec!["pose_stream"]);
assert!(back.include_frame_entries);
assert!(back.include_clock_entries);
```

- [ ] **Step 2: Run focused resource tests and verify they fail**

Run:

```bash
cargo test -p auki-network resources_protocol::tests --features swarm -- --nocapture
```

Expected: compilation fails because `PoseStreamResource`, `ResourceEntry::PoseStream`, `ResourceKind::PoseStream`, `ResourcesRequest::pose_streams`, and `include_clock_entries` do not exist.

- [ ] **Step 3: Implement `PoseStreamResource` and request filtering**

In `ResourcesRequest`, add:

```rust
#[serde(default, skip_serializing_if = "is_false")]
pub include_clock_entries: bool,
```

Update docs on `kinds` to list `"pose_stream"`. Add:

```rust
pub fn pose_streams() -> Self {
    Self {
        kinds: vec![ResourceKind::PoseStream.as_str().into()],
        ..Self::default()
    }
}
```

Update `with_registry_entries`:

```rust
pub fn with_registry_entries(mut self) -> Self {
    self.include_sensor_entries = true;
    self.include_frame_entries = true;
    self.include_clock_entries = true;
    self
}
```

Add the resource kind and row:

```rust
pub enum ResourceKind {
    SensorStream,
    TransformEdge,
    PoseStream,
}

impl ResourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceKind::SensorStream => "sensor_stream",
            ResourceKind::TransformEdge => "transform_edge",
            ResourceKind::PoseStream => "pose_stream",
        }
    }
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceEntry {
    SensorStream(SensorStreamResource),
    TransformEdge(TransformEdgeResource),
    PoseStream(PoseStreamResource),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PoseStreamResource {
    pub id: String,
    pub from_frame_id: String,
    pub from_frame_hash: String,
    pub to_frame_id: String,
    pub to_frame_hash: String,
    pub clock_id: String,
    pub clock_hash: String,
    pub stream_protocol: String,
    pub payload: String,
    pub writer_mode: String,
    pub expected_rate_hz: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_frame_entry_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_frame_entry_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_entry_json: Option<String>,
}
```

Update `ResourceEntry::kind()` and `ResourceEntry::id()` with `PoseStream` match arms.

- [ ] **Step 4: Add additive stream proto fields**

In `crates/auki-datatypes/proto/stream.proto`, update the field ledger:

```proto
//   StreamRequest.sensor_id      = 1
//   StreamRequest.resource_id    = 2
//   StreamManifest.sensor_id          = 1
//   StreamManifest.sensor_hash        = 2
//   StreamManifest.clock_id           = 3
//   StreamManifest.clock_hash         = 4
//   StreamManifest.frame_id           = 5
//   StreamManifest.frame_hash         = 6
//   StreamManifest.resource_id        = 7
//   StreamManifest.payload            = 8
//   StreamManifest.from_frame_id      = 9
//   StreamManifest.from_frame_hash    = 10
//   StreamManifest.to_frame_id        = 11
//   StreamManifest.to_frame_hash      = 12
//   StreamManifest.writer_mode        = 13
//   StreamManifest.expected_rate_hz   = 14
```

Then add fields:

```proto
message StreamRequest {
  string sensor_id = 1;
  string resource_id = 2;
}

message StreamManifest {
  string sensor_id = 1;
  string sensor_hash = 2;
  string clock_id = 3;
  string clock_hash = 4;
  string frame_id = 5;
  string frame_hash = 6;
  string resource_id = 7;
  string payload = 8;
  string from_frame_id = 9;
  string from_frame_hash = 10;
  string to_frame_id = 11;
  string to_frame_hash = 12;
  string writer_mode = 13;
  uint32 expected_rate_hz = 14;
}
```

- [ ] **Step 5: Update stream protocol tests for new fields**

In `crates/auki-network/src/stream_protocol.rs`, update helper constructors to initialize new fields with `..Default::default()` if generated types implement it, or explicit empty/zero fields if not. Add:

```rust
#[test]
fn pose_stream_request_and_manifest_round_trip() {
    let request = StreamRequest {
        sensor_id: String::new(),
        resource_id: "K1/base_link->K1/head_left_rgb_optical".into(),
    };
    let msg = StreamMessage::request(request.clone());
    let mut bytes = Vec::new();
    msg.encode(&mut bytes).unwrap();
    let back = StreamMessage::decode(&*bytes).unwrap();
    assert_eq!(back, msg);

    let manifest = StreamManifest {
        sensor_id: String::new(),
        sensor_hash: String::new(),
        clock_id: "K1/monotonic".into(),
        clock_hash: "clockhash".into(),
        frame_id: String::new(),
        frame_hash: String::new(),
        resource_id: "K1/base_link->K1/head_left_rgb_optical".into(),
        payload: "spatial_transform".into(),
        from_frame_id: "K1/base_link".into(),
        from_frame_hash: "basehash".into(),
        to_frame_id: "K1/head_left_rgb_optical".into(),
        to_frame_hash: "headhash".into(),
        writer_mode: "movable".into(),
        expected_rate_hz: 30,
    };
    let msg = StreamMessage::accept(manifest);
    let mut bytes = Vec::new();
    msg.encode(&mut bytes).unwrap();
    let back = StreamMessage::decode(&*bytes).unwrap();
    assert_eq!(back, msg);
}
```

- [ ] **Step 6: Run resource/proto verification**

Run:

```bash
cargo test -p auki-network resources_protocol::tests stream_protocol::tests --features swarm -- --nocapture
```

Expected: all selected tests pass.

- [ ] **Step 7: Commit foundation**

Run:

```bash
git add crates/auki-datatypes/proto/stream.proto crates/auki-network/src/resources_protocol.rs crates/auki-network/src/stream_protocol.rs
git commit -m "feat: add pose stream resource metadata"
```

## Task 2: Rust Stream Runtime Pose Dispatch

**Files:**
- Modify: `crates/auki-network/src/stream_protocol.rs`
- Modify: `crates/auki-network/src/stream_runtime.rs`

- [ ] **Step 1: Write failing end-to-end pose stream runtime test**

In `crates/auki-network/src/stream_runtime.rs` tests, import pose types:

```rust
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
```

Add helpers:

```rust
fn spatial_transform(tx: f64, ty: f64, tz: f64, qx: f64, qy: f64, qz: f64, qw: f64) -> SpatialTransform {
    SpatialTransform {
        translation: Some(Vec3 { x: tx, y: ty, z: tz }),
        orientation: Some(Quat { x: qx, y: qy, z: qz, w: qw }),
    }
}

fn pose_manifest(resource_id: &str) -> StreamManifest {
    StreamManifest {
        sensor_id: String::new(),
        sensor_hash: String::new(),
        clock_id: "K1/monotonic".into(),
        clock_hash: "clockhash".into(),
        frame_id: String::new(),
        frame_hash: String::new(),
        resource_id: resource_id.into(),
        payload: "spatial_transform".into(),
        from_frame_id: "K1/base_link".into(),
        from_frame_hash: "basehash".into(),
        to_frame_id: "K1/head_left_rgb_optical".into(),
        to_frame_hash: "headhash".into(),
        writer_mode: "movable".into(),
        expected_rate_hz: 30,
    }
}

fn pose_provider_yielding_three_samples() -> StreamProvider {
    Arc::new(|_peer, req| {
        if req.resource_id != "K1/base_link->K1/head_left_rgb_optical" {
            return StreamDispatch::Decline {
                reason: DeclineReason::sensor_not_found(),
            };
        }
        let samples = futures::stream::iter(vec![
            Ok(StreamItem {
                timestamp_ns: 1_000,
                payload: spatial_transform(0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0),
            }),
            Ok(StreamItem {
                timestamp_ns: 2_000,
                payload: spatial_transform(0.1, 0.0, 1.0, 0.0, 0.0, 0.1, 0.995),
            }),
            Ok(StreamItem {
                timestamp_ns: 3_000,
                payload: spatial_transform(0.2, 0.0, 1.0, 0.0, 0.0, 0.2, 0.98),
            }),
        ]);
        StreamDispatch::AcceptPose {
            manifest: pose_manifest(&req.resource_id),
            source: Box::pin(samples),
        }
    })
}
```

Add test:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_accepts_and_streams_pose_samples() {
    let id_p = PeerIdentity::from_seed(&[151u8; 32]);
    let id_c = PeerIdentity::from_seed(&[152u8; 32]);

    let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-pose-producer/0").await;
    let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-pose-consumer/0").await;

    let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
        swarm_p,
        vec![AllowedPeer {
            peer_id: id_c.peer_id(),
            multiaddrs: vec![addr_c],
        }],
        pose_provider_yielding_three_samples(),
        crate::network_runtime::test_heartbeat_timestamps(),
    )
    .expect("producer spawn");
    let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
        swarm_c,
        vec![AllowedPeer {
            peer_id: id_p.peer_id(),
            multiaddrs: vec![addr_p],
        }],
        decline_all_streams(),
        crate::network_runtime::test_heartbeat_timestamps(),
    )
    .expect("consumer spawn");

    let connected = poll_until(
        || consumer.connected_peers().contains(&id_p.peer_id()),
        Duration::from_secs(15),
    )
    .await;
    assert!(connected);

    let sub: StreamSubscription<pose::SpatialTransform> = consumer
        .open_stream(
            id_p.peer_id(),
            StreamRequest {
                sensor_id: String::new(),
                resource_id: "K1/base_link->K1/head_left_rgb_optical".into(),
            },
        )
        .await
        .expect("open_stream<SpatialTransform>");

    assert_eq!(sub.manifest.resource_id, "K1/base_link->K1/head_left_rgb_optical");
    assert_eq!(sub.manifest.payload, "spatial_transform");
    assert_eq!(sub.manifest.from_frame_id, "K1/base_link");
    assert_eq!(sub.manifest.to_frame_id, "K1/head_left_rgb_optical");

    let mut entries = sub.entries;
    let first = entries.next().await.unwrap().expect("sample 0");
    assert_eq!(first.seq, 0);
    assert_eq!(first.timestamp_ns, 1_000);
    assert_eq!(first.payload.translation.unwrap().z, 1.0);
    let second = entries.next().await.unwrap().expect("sample 1");
    assert_eq!(second.seq, 1);
    let third = entries.next().await.unwrap().expect("sample 2");
    assert_eq!(third.seq, 2);
    assert_eq!(third.payload.translation.unwrap().x, 0.2);

    let end = entries.next().await.unwrap().expect_err("expected EndOfStream");
    match end {
        StreamError::EndOfStream { reason }
            if matches!(reason.kind, Some(end_reason::Kind::SourceEnded(_))) => {}
        other => panic!("expected SourceEnded, got {other:?}"),
    }

    producer.shutdown();
    consumer.shutdown();
}
```

- [ ] **Step 2: Run focused runtime test and verify it fails**

Run:

```bash
cargo test -p auki-network producer_accepts_and_streams_pose_samples --features swarm -- --nocapture
```

Expected: compilation fails because `pose` is not re-exported by `stream_protocol` and `StreamDispatch::AcceptPose` does not exist.

- [ ] **Step 3: Re-export pose and add dispatch variant**

In `crates/auki-network/src/stream_protocol.rs`, add:

```rust
pub use auki_datatypes::{audio, joint_encoders, point_cloud, pose};
```

In `crates/auki-network/src/stream_runtime.rs`, import `pose` and add:

```rust
AcceptPose {
    manifest: StreamManifest,
    source: SourceStream<pose::SpatialTransform>,
},
```

Update `handle_inbound_substream`:

```rust
StreamDispatch::AcceptPose { manifest, source } => {
    pump_typed::<pose::SpatialTransform>(substream, manifest, source, shutdown_rx).await;
}
```

Update docs around supported `T`s to include `pose::SpatialTransform`.

- [ ] **Step 4: Run runtime verification**

Run:

```bash
cargo test -p auki-network producer_accepts_and_streams_pose_samples --features swarm -- --nocapture
cargo test -p auki-network stream_runtime::tests --features swarm -- --nocapture
```

Expected: pose stream test passes and existing stream runtime tests still pass.

- [ ] **Step 5: Commit runtime dispatch**

Run:

```bash
git add crates/auki-network/src/stream_protocol.rs crates/auki-network/src/stream_runtime.rs
git commit -m "feat: stream SpatialTransform pose samples"
```

## Task 3: Domain Resource Enrichment And Rust Re-Exports

**Files:**
- Modify: `crates/auki-domain/src/lib.rs`
- Modify: `crates/auki-domain/src/cluster_manager.rs`
- Modify: `crates/auki-domain/README.md`

- [ ] **Step 1: Write failing resource enrichment test**

In `crates/auki-domain/src/cluster_manager.rs` tests, add a fixture helper that writes `base_link`, `head_left_rgb_optical`, and a clock registry entry. Use existing `write_frame` and `write_clock` patterns in the file. Add a test:

```rust
#[test]
fn resource_enrichment_embeds_pose_stream_frame_and_clock_entries() {
    use auki_network::resources_protocol::PoseStreamResource;
    use auki_registry::{ClockBody, ClockMeta, Scope};

    let dir = tempfile::tempdir().unwrap();
    let base = FrameRegistryEntry::ros_body("K1/base_link");
    let base_hash = write_frame(dir.path(), &base).unwrap().hash().to_string();
    let head = FrameRegistryEntry::ros_optical("K1/head_left_rgb_optical");
    let head_hash = write_frame(dir.path(), &head).unwrap().hash().to_string();
    let clock = ClockRegistryEntry {
        clock_id: "K1/monotonic".into(),
        body: ClockBody::MonotonicClock(ClockMeta {
            unit: "nanoseconds".into(),
            monotonic: true,
            epoch: None,
            scope: Scope::DeviceLocal,
        }),
    };
    let clock_hash = write_clock(dir.path(), &clock).unwrap().hash().to_string();

    let mut resources = vec![ResourceEntry::PoseStream(PoseStreamResource {
        id: "K1/base_link->K1/head_left_rgb_optical".into(),
        from_frame_id: base.frame_id.clone(),
        from_frame_hash: base_hash,
        to_frame_id: head.frame_id.clone(),
        to_frame_hash: head_hash,
        clock_id: clock.clock_id.clone(),
        clock_hash,
        stream_protocol: "/auki/stream/0.1.0".into(),
        payload: "spatial_transform".into(),
        writer_mode: "movable".into(),
        expected_rate_hz: 30,
        source: None,
        from_frame_entry_json: None,
        to_frame_entry_json: None,
        clock_entry_json: None,
    })];

    enrich_resource_entries(
        &mut resources,
        &ResourcesRequest {
            include_frame_entries: true,
            include_clock_entries: true,
            ..ResourcesRequest::pose_streams()
        },
        dir.path(),
    );

    let ResourceEntry::PoseStream(row) = &resources[0] else {
        panic!("expected pose stream");
    };
    assert!(row.from_frame_entry_json.as_ref().unwrap().contains("base_link"));
    assert!(row.to_frame_entry_json.as_ref().unwrap().contains("head_left_rgb_optical"));
    assert!(row.clock_entry_json.as_ref().unwrap().contains("monotonic"));
}
```

- [ ] **Step 2: Run focused domain test and verify it fails**

Run:

```bash
cargo test -p auki-domain resource_enrichment_embeds_pose_stream_frame_and_clock_entries --features swarm -- --nocapture
```

Expected: compilation fails because `PoseStreamResource` is not re-exported and `enrich_resource_entries` has no `PoseStream` match arm.

- [ ] **Step 3: Re-export `PoseStreamResource`**

In `crates/auki-domain/src/lib.rs`, update the resource re-export list:

```rust
pub use auki_network::resources_protocol::{
    PoseStreamResource, ResourceEntry, ResourceKind, ResourcePinholeIntrinsics, ResourceQuat,
    ResourceSpatialTransform, ResourceVec3, ResourcesRequest, ResourcesResponse,
    SensorStreamResource, TransformEdgeResource,
};
```

- [ ] **Step 4: Add pose stream enrichment**

In `enrich_resource_entries`, update the early return:

```rust
if !request.include_sensor_entries && !request.include_frame_entries && !request.include_clock_entries {
    return;
}
```

Add a `ResourceEntry::PoseStream(pose)` arm:

```rust
ResourceEntry::PoseStream(pose) => {
    if request.include_frame_entries {
        match auki_registry::read_frame(app_root, &pose.from_frame_id, &pose.from_frame_hash) {
            Ok(Some(frame)) if frame.hash() == pose.from_frame_hash => {
                pose.from_frame_entry_json = Some(canonical_json(frame.canonical_bytes()));
            }
            Ok(Some(_)) | Ok(None) | Err(_) => {}
        }
        match auki_registry::read_frame(app_root, &pose.to_frame_id, &pose.to_frame_hash) {
            Ok(Some(frame)) if frame.hash() == pose.to_frame_hash => {
                pose.to_frame_entry_json = Some(canonical_json(frame.canonical_bytes()));
            }
            Ok(Some(_)) | Ok(None) | Err(_) => {}
        }
    }
    if request.include_clock_entries {
        match auki_registry::read_clock(app_root, &pose.clock_id, &pose.clock_hash) {
            Ok(Some(clock)) if clock.hash() == pose.clock_hash => {
                pose.clock_entry_json = Some(canonical_json(clock.canonical_bytes()));
            }
            Ok(Some(_)) | Ok(None) | Err(_) => {}
        }
    }
}
```

Update `spawn_resources_handler` to pass `include_clock_entries` through provider snapshots by relying on the existing `ResourcesRequest` clone.

- [ ] **Step 5: Run domain verification**

Run:

```bash
cargo test -p auki-domain resource_enrichment_embeds_pose_stream_frame_and_clock_entries --features swarm -- --nocapture
cargo test -p auki-domain --features swarm
```

Expected: focused test and domain suite pass.

- [ ] **Step 6: Commit domain resource support**

Run:

```bash
git add crates/auki-domain/src/lib.rs crates/auki-domain/src/cluster_manager.rs crates/auki-domain/README.md
git commit -m "feat: expose pose stream resources in domain"
```

## Task 4: Python Network Binding Pose Payload

**Files:**
- Modify: `bindings/python/auki-network-py/src/stream_types.rs`
- Modify: `bindings/python/auki-network-py/src/lib.rs`
- Modify: `bindings/python/auki-network-py/README.md`
- Modify: `bindings/python/auki-network-py/python_tests/test_streams.py`

- [ ] **Step 1: Write failing Rust PyO3 unit tests**

In `bindings/python/auki-network-py/src/stream_types.rs` tests, import:

```rust
use auki_datatypes::pose::SpatialTransform as PoseSpatialTransform;
```

Add tests:

```rust
#[test]
fn spatial_transform_frame_round_trips_flat_values() {
    Python::with_gil(|_py| {
        let f = PySpatialTransformFrame::new(vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        assert_eq!(f.values(), vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(f.__len__(), 7);
        assert!(f.__repr__().starts_with("SpatialTransformFrame("));
        assert!(f.__repr__().contains("1.0"));
    });
}

#[test]
fn stream_item_extracts_to_rust_pose() {
    Python::with_gil(|py| {
        let frame = PySpatialTransformFrame::new(vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        let payload_any = Py::new(py, frame).unwrap().bind(py).clone().into_any();
        let item = PyStreamItem::new(123, payload_any).unwrap();
        let rust = item.to_rust_pose().expect("payload is pose");
        assert_eq!(rust.timestamp_ns, 123);
        assert_eq!(rust.payload.translation.unwrap().x, 1.0);
        assert_eq!(rust.payload.orientation.unwrap().w, 1.0);
    });
}
```

- [ ] **Step 2: Run focused binding test and verify it fails**

Run:

```bash
cargo test -p auki-network-py spatial_transform_frame stream_item_extracts_to_rust_pose -- --nocapture
```

Expected: compilation fails because `PySpatialTransformFrame`, `StreamPayload::Pose`, and `to_rust_pose` do not exist.

- [ ] **Step 3: Add `SpatialTransformFrame` pyclass**

Near the existing payload pyclasses in `stream_types.rs`, add:

```rust
use auki_datatypes::pose::{
    Quat as RustPoseQuat, SpatialTransform as RustPoseSpatialTransform, Vec3 as RustPoseVec3,
};

#[pyclass(name = "SpatialTransformFrame", frozen)]
#[derive(Clone, Debug)]
pub struct PySpatialTransformFrame {
    pub(crate) inner: RustPoseSpatialTransform,
}

#[pymethods]
impl PySpatialTransformFrame {
    #[new]
    fn new(values: Vec<f64>) -> PyResult<Self> {
        if values.len() != 7 {
            return Err(PyValueError::new_err(format!(
                "SpatialTransformFrame expects 7 values [tx, ty, tz, qx, qy, qz, qw]; got {}",
                values.len()
            )));
        }
        Ok(Self {
            inner: RustPoseSpatialTransform {
                translation: Some(RustPoseVec3 {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                }),
                orientation: Some(RustPoseQuat {
                    x: values[3],
                    y: values[4],
                    z: values[5],
                    w: values[6],
                }),
            },
        })
    }

    #[getter]
    fn values(&self) -> Vec<f64> {
        let t = self.inner.translation.as_ref();
        let q = self.inner.orientation.as_ref();
        vec![
            t.map(|v| v.x).unwrap_or(0.0),
            t.map(|v| v.y).unwrap_or(0.0),
            t.map(|v| v.z).unwrap_or(0.0),
            q.map(|v| v.x).unwrap_or(0.0),
            q.map(|v| v.y).unwrap_or(0.0),
            q.map(|v| v.z).unwrap_or(0.0),
            q.map(|v| v.w).unwrap_or(1.0),
        ]
    }

    fn __len__(&self) -> usize {
        7
    }

    fn __repr__(&self) -> String {
        format!("SpatialTransformFrame({:?})", self.values())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
```

- [ ] **Step 4: Thread pose through payload unions and stream decisions**

Update `StreamPayload`:

```rust
Pose(PySpatialTransformFrame),
```

Update `from_py`, `into_py`, `repr`, and `kind_name` to include `SpatialTransformFrame`.

Add `PyStreamItem::to_rust_pose`:

```rust
pub(crate) fn to_rust_pose(&self) -> Result<RustStreamItem<RustPoseSpatialTransform>, String> {
    match &self.payload {
        StreamPayload::Pose(f) => Ok(RustStreamItem {
            timestamp_ns: self.timestamp_ns,
            payload: f.inner.clone(),
        }),
        other => Err(format!(
            "AcceptPose source yielded a StreamItem with {} payload; the substream is mono-T — yield SpatialTransformFrame or use the matching factory",
            other.kind_name(),
        )),
    }
}
```

Update `DecisionInner`:

```rust
AcceptPose {
    manifest: PyStreamManifest,
    source: Py<PyAny>,
},
```

Add factory:

```rust
#[staticmethod]
#[pyo3(signature = (*, manifest, source))]
fn accept_pose(manifest: PyStreamManifest, source: Py<PyAny>) -> Self {
    Self {
        inner: Mutex::new(Some(DecisionInner::AcceptPose { manifest, source })),
    }
}
```

Update `kind()` and `build_stream_provider`:

```rust
Ok(DecisionInner::AcceptPose { manifest, source }) => {
    let source_stream = python_iter_into_source_stream::<RustPoseSpatialTransform>(source, |pf| {
        pf.to_rust_pose()
    });
    RustStreamDispatch::AcceptPose {
        manifest: manifest.inner,
        source: source_stream,
    }
}
```

- [ ] **Step 5: Thread pose through consumer subscriptions**

Add:

```rust
type RustPoseStream =
    Pin<Box<dyn Stream<Item = Result<RustStreamEntry<RustPoseSpatialTransform>, RustStreamError>> + Send>>;
```

Add `EntryStreamKind::Pose(RustPoseStream)`, `EntryNext::Pose(Result<RustStreamEntry<RustPoseSpatialTransform>, RustStreamError>)`, `PyStreamEntry::from_rust_pose`, `PyStreamSubscription::from_rust_pose`, and `__next__` match arms that call `stream.next().await`, restore the stream on `Some(_)`, exhaust it on `None`, and convert successful pose entries with `PyStreamEntry::from_rust_pose`.

- [ ] **Step 6: Register public Python class**

In `pub(crate) fn register(py: Python<'_>, cluster: &Bound<'_, PyModule>) -> PyResult<()>`, add:

```rust
cluster.add_class::<PySpatialTransformFrame>()?;
```

In `bindings/python/auki-network-py/python_tests/test_streams.py`, add:

```python
def test_pose_stream_surface_is_exposed():
    from auki_network import cluster

    frame = cluster.SpatialTransformFrame([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0])
    assert frame.values == [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0]
    assert hasattr(cluster.StreamDecision, "accept_pose")
```

- [ ] **Step 7: Run Python network binding verification**

Run:

```bash
cargo test -p auki-network-py spatial_transform_frame stream_item_extracts_to_rust_pose -- --nocapture
cargo test -p auki-network-py
```

Expected: all `auki-network-py` tests pass.

- [ ] **Step 8: Commit Python network support**

Run:

```bash
git add bindings/python/auki-network-py/src/stream_types.rs bindings/python/auki-network-py/src/lib.rs bindings/python/auki-network-py/README.md bindings/python/auki-network-py/python_tests/test_streams.py
git commit -m "feat: expose pose stream payloads in Python network bindings"
```

## Task 5: Python Domain Resource And Open Helpers

**Files:**
- Modify: `bindings/python/auki-domain-py/src/lib.rs`
- Modify: `bindings/python/auki-domain-py/README.md`
- Modify: `bindings/python/auki-domain-py/python_tests/test_surface.py`

- [ ] **Step 1: Write failing domain binding tests**

In `bindings/python/auki-domain-py/src/lib.rs` tests, add module exposure assertions:

```rust
assert!(module.getattr("PoseStreamResource").is_ok());
assert!(
    module
        .getattr("ClusterManager")
        .unwrap()
        .getattr("open_pose_stream")
        .is_ok()
);
```

Add a Rust unit test for generic resolver:

```rust
#[test]
fn generic_stream_resolver_uses_pose_stream_resource_metadata() {
    let resources = vec![RustResourceEntry::PoseStream(RustPoseStreamResource {
        id: "K1/base_link->K1/head_left_rgb_optical".into(),
        from_frame_id: "K1/base_link".into(),
        from_frame_hash: "basehash".into(),
        to_frame_id: "K1/head_left_rgb_optical".into(),
        to_frame_hash: "headhash".into(),
        clock_id: "K1/monotonic".into(),
        clock_hash: "clockhash".into(),
        stream_protocol: "/auki/stream/0.1.0".into(),
        payload: "spatial_transform".into(),
        writer_mode: "movable".into(),
        expected_rate_hz: 30,
        source: None,
        from_frame_entry_json: None,
        to_frame_entry_json: None,
        clock_entry_json: None,
    })];

    assert_eq!(
        resolve_generic_stream_payload_kind(&resources, "K1/base_link->K1/head_left_rgb_optical").unwrap(),
        GenericStreamPayloadKind::Pose
    );
}
```

In `bindings/python/auki-domain-py/python_tests/test_surface.py`, add:

```python
def test_pose_stream_resource_surface():
    import auki_domain

    row = auki_domain.PoseStreamResource(
        id="K1/base_link->K1/head_left_rgb_optical",
        from_frame_id="K1/base_link",
        from_frame_hash="basehash",
        to_frame_id="K1/head_left_rgb_optical",
        to_frame_hash="headhash",
        clock_id="K1/monotonic",
        clock_hash="clockhash",
        stream_protocol="/auki/stream/0.1.0",
        payload="spatial_transform",
        writer_mode="movable",
        expected_rate_hz=30,
    )
    assert row.kind == "pose_stream"
    assert row.payload == "spatial_transform"
    assert hasattr(auki_domain.ClusterManager, "open_pose_stream")
```

- [ ] **Step 2: Run focused domain binding tests and verify they fail**

Run:

```bash
cargo test -p auki-domain-py pose_stream generic_stream_resolver_uses_pose_stream_resource_metadata -- --nocapture
```

Expected: compilation fails because `RustPoseStreamResource`, `PyPoseStreamResource`, and `GenericStreamPayloadKind::Pose` do not exist.

- [ ] **Step 3: Import Rust pose types and extend generic resolver**

In imports:

```rust
PoseStreamResource as RustPoseStreamResource,
```

And:

```rust
use auki_network::stream_protocol::pose::SpatialTransform as RustPoseSpatialTransform;
```

Extend enum:

```rust
Pose,
```

Update resolver:

```rust
RustResourceEntry::PoseStream(stream) if stream.id == stream_id => {
    if stream.payload == "spatial_transform" {
        return Ok(GenericStreamPayloadKind::Pose);
    }
    return Err(PyValueError::new_err(format!(
        "pose stream {:?} advertises unsupported payload {:?}",
        stream.id, stream.payload
    )));
}
```

Rename the helper parameter `sensor_id: &str` to `stream_id: &str` in `resolve_generic_stream_payload_kind` and `resolve_stream_payload_kind`, update the sensor-stream match to `stream.sensor_id == stream_id`, and keep the public `open_stream(peer_id, sensor_id)` signature unchanged for backwards compatibility.

- [ ] **Step 4: Add `PyPoseStreamResource`**

Near other resource pyclasses, add:

```rust
#[pyclass(name = "PoseStreamResource")]
#[derive(Clone)]
pub struct PyPoseStreamResource {
    inner: RustPoseStreamResource,
}

#[pymethods]
impl PyPoseStreamResource {
    #[new]
    #[pyo3(signature = (
        id,
        from_frame_id,
        from_frame_hash,
        to_frame_id,
        to_frame_hash,
        clock_id,
        clock_hash,
        stream_protocol,
        payload,
        writer_mode,
        expected_rate_hz,
        source_json = None,
        from_frame_entry_json = None,
        to_frame_entry_json = None,
        clock_entry_json = None
    ))]
    fn new(
        id: String,
        from_frame_id: String,
        from_frame_hash: String,
        to_frame_id: String,
        to_frame_hash: String,
        clock_id: String,
        clock_hash: String,
        stream_protocol: String,
        payload: String,
        writer_mode: String,
        expected_rate_hz: u32,
        source_json: Option<String>,
        from_frame_entry_json: Option<String>,
        to_frame_entry_json: Option<String>,
        clock_entry_json: Option<String>,
    ) -> PyResult<Self> {
        let source = match source_json {
            Some(json) => Some(serde_json::from_str(&json).map_err(|e| {
                PyValueError::new_err(format!("source_json must be valid JSON: {e}"))
            })?),
            None => None,
        };
        Ok(Self {
            inner: RustPoseStreamResource {
                id,
                from_frame_id,
                from_frame_hash,
                to_frame_id,
                to_frame_hash,
                clock_id,
                clock_hash,
                stream_protocol,
                payload,
                writer_mode,
                expected_rate_hz,
                source,
                from_frame_entry_json,
                to_frame_entry_json,
                clock_entry_json,
            },
        })
    }

    #[getter]
    fn kind(&self) -> &'static str { "pose_stream" }
    #[getter]
    fn id(&self) -> String { self.inner.id.clone() }
    #[getter]
    fn from_frame_id(&self) -> String { self.inner.from_frame_id.clone() }
    #[getter]
    fn from_frame_hash(&self) -> String { self.inner.from_frame_hash.clone() }
    #[getter]
    fn to_frame_id(&self) -> String { self.inner.to_frame_id.clone() }
    #[getter]
    fn to_frame_hash(&self) -> String { self.inner.to_frame_hash.clone() }
    #[getter]
    fn clock_id(&self) -> String { self.inner.clock_id.clone() }
    #[getter]
    fn clock_hash(&self) -> String { self.inner.clock_hash.clone() }
    #[getter]
    fn stream_protocol(&self) -> String { self.inner.stream_protocol.clone() }
    #[getter]
    fn payload(&self) -> String { self.inner.payload.clone() }
    #[getter]
    fn writer_mode(&self) -> String { self.inner.writer_mode.clone() }
    #[getter]
    fn expected_rate_hz(&self) -> u32 { self.inner.expected_rate_hz }
    #[getter]
    fn source_json(&self) -> Option<String> {
        self.inner.source.as_ref().map(|value| serde_json::to_string(value).expect("serde_json::Value serializes"))
    }
    #[getter]
    fn from_frame_entry_json(&self) -> Option<String> { self.inner.from_frame_entry_json.clone() }
    #[getter]
    fn to_frame_entry_json(&self) -> Option<String> { self.inner.to_frame_entry_json.clone() }
    #[getter]
    fn clock_entry_json(&self) -> Option<String> { self.inner.clock_entry_json.clone() }
}
```

- [ ] **Step 5: Thread pose resource through provider extraction and conversion**

Update extraction error text to include `PoseStreamResource`. Add:

```rust
if let Ok(pose) = item.extract::<PyRef<'_, PyPoseStreamResource>>() {
    resources.push(RustResourceEntry::PoseStream(pose.inner.clone()));
    continue;
}
```

Update `resource_entry_to_py`:

```rust
RustResourceEntry::PoseStream(inner) => {
    Ok(Py::new(py, PyPoseStreamResource { inner })?.into_py(py))
}
```

Register:

```rust
m.add_class::<PyPoseStreamResource>()?;
```

- [ ] **Step 6: Add pose stream open helpers**

Add method:

```rust
fn open_pose_stream(
    &self,
    py: Python<'_>,
    peer_id: &str,
    resource_id: &str,
) -> PyResult<PyStreamSubscription> {
    self.open_typed_stream_with_request::<RustPoseSpatialTransform>(
        py,
        peer_id,
        RustStreamRequest {
            sensor_id: String::new(),
            resource_id: resource_id.to_string(),
        },
        |sub| PyStreamSubscription::from_rust_pose(sub),
    )
}
```

Refactor existing `open_typed_stream` to call a new helper:

```rust
fn open_typed_stream_with_request<T>(
    &self,
    py: Python<'_>,
    peer_id: &str,
    request: RustStreamRequest,
    to_py_sub: impl FnOnce(auki_network::stream_runtime::StreamSubscription<T>) -> PyStreamSubscription
        + Send
        + 'static,
) -> PyResult<PyStreamSubscription>
where
    T: prost::Message + Default + Send + 'static,
{
    let peer_id_parsed = parse_peer_id(peer_id)?;
    let inner = self.inner.clone();
    py.allow_threads(|| {
        shared_runtime().block_on(async move {
            let guard = inner.lock().expect("ClusterManager lock");
            let manager = guard
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("ClusterManager has been shut down"))?;
            let rust_sub = manager
                .open_stream::<T>(peer_id_parsed, request)
                .await
                .map_err(|e| Python::with_gil(|py| open_stream_error_to_pyerr(py, e)))?;
            Ok(to_py_sub(rust_sub))
        })
    })
}
```

Existing sensor openers pass `RustStreamRequest { sensor_id: sensor_id.to_string(), resource_id: String::new() }`.

Update generic `open_stream` to resolve `Pose` and call `open_pose_stream`.

- [ ] **Step 7: Run domain Python binding verification**

Run:

```bash
cargo test -p auki-domain-py pose_stream generic_stream_resolver_uses_pose_stream_resource_metadata -- --nocapture
cargo test -p auki-domain-py
```

Expected: all `auki-domain-py` tests pass.

- [ ] **Step 8: Commit Python domain support**

Run:

```bash
git add bindings/python/auki-domain-py/src/lib.rs bindings/python/auki-domain-py/README.md bindings/python/auki-domain-py/python_tests/test_surface.py
git commit -m "feat: expose pose stream resources in Python domain bindings"
```

## Task 6: Documentation And Workspace Verification

**Files:**
- Modify: `README.md`
- Modify: `Vision.md`
- Modify: `crates/auki-network/README.md`
- Modify: `crates/auki-domain/README.md`
- Modify: `bindings/python/auki-network-py/README.md`
- Modify: `bindings/python/auki-domain-py/README.md`

- [ ] **Step 1: Update active docs**

Update current-state docs to say:

- `/auki/resources/0.0.1` supports `sensor_stream`, rigid `transform_edge`, and live movable `pose_stream` rows.
- `/auki/stream/0.1.0` supports `SpatialTransform` pose payloads in addition to camera, point cloud, joint encoders, audio, and detection where already surfaced.
- `auki-domain-py` exposes `PoseStreamResource` and `ClusterManager.open_pose_stream`.
- First hardware target is RoboStreamer on Galbot G1 publishing `base_link -> head_left_rgb_optical` for Park.

- [ ] **Step 2: Run repository search for stale future-work claims**

Run:

```bash
rg -n "pose streams.*future|movable pose streams.*future|Full live pose-stream|rigid `transform_edge` rows so peers can discover stream sources and direct frame edges; movable pose streams" README.md Vision.md crates bindings docs
```

Expected: no active docs still claim live pose streams are wholly future work. Historical design/plan files may describe earlier state if they are clearly dated.

- [ ] **Step 3: Run final Rust verification**

Run:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test -p auki-network resources_protocol::tests stream_protocol::tests producer_accepts_and_streams_pose_samples --features swarm -- --nocapture
cargo test -p auki-domain resource_enrichment_embeds_pose_stream_frame_and_clock_entries --features swarm -- --nocapture
cargo test -p auki-network-py
cargo test -p auki-domain-py
```

Expected: all commands pass. Existing PyO3 Rust-2024 warnings may remain, but no new errors.

- [ ] **Step 4: Check git status**

Run:

```bash
git status --short
```

Expected: only intentional docs changes remain unstaged if previous tasks committed their implementation changes.

- [ ] **Step 5: Commit documentation**

Run:

```bash
git add README.md Vision.md crates/auki-network/README.md crates/auki-domain/README.md bindings/python/auki-network-py/README.md bindings/python/auki-domain-py/README.md
git commit -m "docs: document live pose stream support"
```

## Task 7: Hardware Handoff Notes

**Files:**
- Modify: `docs/superpowers/specs/2026-05-25-live-pose-stream-design.md`
- Create: `docs/superpowers/plans/2026-05-25-live-pose-stream-hardware-smoke.md`

- [ ] **Step 1: Record concrete Galbot smoke expectations**

Create `docs/superpowers/plans/2026-05-25-live-pose-stream-hardware-smoke.md`:

```markdown
# Galbot G1 Live Pose Stream Smoke

Goal: Validate RoboStreamer publishes a movable `base_link -> head_left_rgb_optical` pose stream and Park consumes it alongside `base_lidar_pointcloud` and `head_left_rgb`.

Producer expectations:
- RoboStreamer advertises a `PoseStreamResource` with `payload = "spatial_transform"`.
- `from_frame_id` is the Galbot `base_link` frame and `to_frame_id` is the Galbot `head_left_rgb_optical` frame.
- `writer_mode = "movable"`.
- `StreamEntry.timestamp_ns` uses the same clock identity as the Galbot sensor streams.

Consumer expectations:
- Park fetches `/auki/resources/0.0.1` with frame and clock embedding enabled.
- Park opens `ClusterManager.open_pose_stream(peer_id, resource.id)`.
- Park receives changing `SpatialTransformFrame.values` while the Galbot head moves.
- Park rejects the setup if the live manifest disagrees with the resource row.
```

- [ ] **Step 2: Commit handoff note**

Run:

```bash
git add docs/superpowers/plans/2026-05-25-live-pose-stream-hardware-smoke.md
git commit -m "docs: capture Galbot pose stream smoke expectations"
```

## Final Verification

- [ ] **Step 1: Run full branch checks**

Run:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test -p auki-network --features swarm
cargo test -p auki-domain --features swarm
cargo test -p auki-network-py
cargo test -p auki-domain-py
```

Expected: all commands pass. When a long-running network integration test flakes, rerun once and record both outputs in the PR notes.

- [ ] **Step 2: Confirm branch status**

Run:

```bash
git status --short --branch
```

Expected: clean worktree on `feat/issue-206-live-pose-stream`, ahead of `origin/develop`.

- [ ] **Step 3: Prepare PR summary**

Use this PR body skeleton:

```markdown
Closes #206

## Why

RoboStreamer and Park need a first-class live movable pose edge for Galbot G1 head motion. A rigid transform edge is incorrect because the Galbot head moves relative to `base_link`.

## What

- Added `pose_stream` resource catalog rows.
- Added resource-addressed stream request/manifest metadata.
- Added live `SpatialTransform` stream dispatch.
- Exposed pose stream producer and consumer surfaces in Python.
- Documented the Galbot G1 RoboStreamer to Park hardware smoke path.

## Verification

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test -p auki-network --features swarm`
- `cargo test -p auki-domain --features swarm`
- `cargo test -p auki-network-py`
- `cargo test -p auki-domain-py`
```
