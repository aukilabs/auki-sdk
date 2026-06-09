# For SDK Consumers

This section is for engineers building products on top of the Auki SDK. If you're integrating the SDK into:

- **Booster** — the robot operator-facing visualizer / control surface
- **Park** — the desktop / browser visualizer + scenegraph composition
- **Galbot ROS adapters** — publishing pose, camera, point-cloud streams from a ROS environment
- **Your own custom robot data plane consumer**

…you're in the right place.

## Where to start

- [Quickstart](Quickstart) — boot a peer, register a sensor and log, inspect the catalog
- [Concept: peer-owned logs](Concept-Peer-Owned-Logs) — the SDK's core data abstraction (source/writer split, materialization)

## Pose log discovery and stream open

Consumers such as Park should treat `/auki/resources/0.2.0` as the live
catalog of requestable resources. Poll it, reconcile additions/removals, and
open streams by the row's stable `resource_id`.

For a live movable pose stream:

1. Fetch a peer's resources with a `pose_log` variant filter.
2. Select rows where `state == "live"` and `pose.writer_mode == "movable"`.
3. Dial the row's `writer_peer_id` peer and open `/auki/stream/0.2.0` with
   `StreamRequest { source_peer_id: row.source_peer_id, resource_id:
   row.resource_id, from: ReadFrom::Latest }`.

Rust shape:

```rust
let response = cluster
    .fetch_resources_catalog_with(
        writer_peer_id,
        ResourcesRequest {
            variants: vec![Variant::PoseLog],
        },
    )
    .await?;

for row in response.resources {
    let is_live_pose = row.state == "live"
        && row
            .pose
            .as_ref()
            .is_some_and(|pose| pose.writer_mode == PoseWriterMode::Movable);
    if !is_live_pose {
        continue;
    }

    let request = StreamRequest {
        source_peer_id: row.source_peer_id.clone(),
        resource_id: row.resource_id.clone(),
        from: ReadFrom::Latest,
    };
    let subscription = cluster
        .open_stream::<auki_datatypes::pose::SpatialTransform>(writer_peer_id, request)
        .await?;
}
```

Python shape:

```python
from auki_domain import ReadFrom, StreamRequest

for row in manager.fetch_resources_catalog(peer_id, variants=["pose_log"]):
    pose = row.pose
    if row.state != "live":
        continue
    if pose is None or pose.get("writer_mode") != "movable":
        continue

    request = StreamRequest(
        resource_id=row.resource_id,
        source_peer_id=row.source_peer_id,
        from_=ReadFrom.latest(),
    )
    sub = manager.open_stream_with_request(row.writer_peer_id, request)
```

`writer_peer_id` chooses which peer to dial. `StreamRequest` identifies the
data by `source_peer_id` and `resource_id`; it does not carry `writer_peer_id`.

More concept primers and recipes coming as follow-up cards land:

- The three IDs (peer / app / session)
- Materialization (how Park copies Galbot's stream)
- Register a sensor and publish frames (recipe)
- Consume another peer's stream (recipe)
- Materialize a remote log with custom retention (recipe)
- Migrate from pre-#216 SDK to v0.0.53+ (recipe)
- Troubleshooting common errors

## API surface

Apps use `auki-session` as the entry point. See:

- [auki-session crate README](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-session/README.md) — Rust API
- [auki-session-py README](https://github.com/aukilabs/auki-sdk/blob/develop/bindings/python/auki-session-py/README.md) — Python API

## Release history

- [Release history](Release-History) — what changed between SDK versions, what to expect when bumping pins *(stub)*

For the immediate "what does this version break?" answer, the annotated git tag is authoritative: `git show v0.0.53` etc.
