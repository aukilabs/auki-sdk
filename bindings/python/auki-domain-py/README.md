# auki-domain-py

PyO3 bindings for [`auki-domain`](../../../crates/auki-domain) — the cluster
lifecycle crate. Exposes the cluster membership document, the full
`ClusterManager` lifecycle (create / join / bootstrap), the post-#216
`ResourceEntry` catalog shape, and the `ReadFrom` / `StreamRequest` stream
subscription types to Python daemons.

**Status:** Active — post-#216 schema (#230).

## Python module: `auki_domain`

### Value types

| Python class | Role |
|---|---|
| `ClusterMember` | One peer row in `ClusterMembership` |
| `ClusterMembership` | Cluster membership document (JSON round-trip) |
| `DaemonInfo` | Daemon-supplied identity fields for `participant_info` |
| `ParticipantInfo` | SDK-produced `/api/info` wire shape (`.to_json()`) |
| `SensorEntry` | One row in a peer's sensor catalog |
| `ResourceEntry` | Post-#216 resource catalog row. `variant` discriminates `sensor_log` \| `pose_log` \| `time_transform_log` \| `detection_log`. Nested blocks (`head`, `extent`, `available`, `sensor`, `pose`, `manifest`) returned as Python dicts. Construct via `ResourceEntry.from_dict(d)` or `ResourceEntry.from_json(s)` (see below). |
| `ReadFrom` | Stream start position: `.latest()` / `.from_start()` / `.from_timestamp(ns)` |
| `StreamRequest` | Consumer → Producer handshake: `resource_id`, `source_peer_id`, `from_` |
| `ClusterTarget` | Policy enum for `ClusterManager.bootstrap` |
| `StreamManifestBuilder` | Producer-side helper for building stream manifests |

### `ClusterManager`

Daemon-side cluster handle. All constructors are synchronous (block on an
internal multi-thread tokio runtime).

**Constructors (static methods):**
- `bootstrap(target, wallet_seed, discovery_url, …)` — policy-driven; headless daemons use this
- `create_cluster(wallet_seed, cluster_name, discovery_url, …)`
- `create_cluster_with_relay_multiaddrs(…, relay_multiaddrs, …)`
- `create_cluster_with_relay_reservation(…, relay_dial_multiaddr, relay_advertise_multiaddr, …)`
- `join_cluster(wallet_seed, cluster_name, discovery_url, …)`
- `list_clusters(discovery_url)` → `list[ClusterEntry]`

**Stream methods:**
- `open_stream(peer_id, resource_id)` — auto-dispatches by catalog variant
- `open_stream_with_request(peer_id, request: StreamRequest)` — full post-#216 control; resolves the payload kind by the request's `source_peer_id` + `resource_id`
- `open_camera_stream / open_pointcloud_stream / open_joint_encoders_stream / open_audio_stream / open_pose_stream`

**Catalog / registry:**
- `fetch_resources_catalog(peer_id, variants=None)` → `list[ResourceEntry]`
- `fetch_sensor_entry / fetch_clock_entry / fetch_frame_entry` → canonical JSON

## Constructing `ResourceEntry` from Python

Use `ResourceEntry.from_dict(d)` or `ResourceEntry.from_json(s)` to mint
catalog rows from Python — e.g. to feed into
`ClusterManager.set_resource_catalog_provider`. The dict / JSON shape must
match the `/auki/resources/0.2.0` wire format: a flat object with a `variant`
discriminator field plus the common fields and variant-specific `manifest`
block.

```python
from auki_domain import ResourceEntry

entry = ResourceEntry.from_dict({
    "variant": "pose_log",
    "source_peer_id": "galbot",
    "writer_peer_id": "galbot",
    "resource_id": "base_link->head_left_camera_color_optical_frame",
    "state": "live",
    "head": {"kind": "rolling", "retention_ns": 5_000_000_000},
    "available": {"bytes": 0, "entries": 0, "duration_ns": 0},
    "pose": {"writer_mode": "movable"},
    "manifest": {
        "from_frame": {"peer_id": "galbot", "id": "base_link",   "hash": "..."},
        "to_frame":   {"peer_id": "galbot", "id": "head_left_camera_color_optical_frame", "hash": "..."},
        "clock":      {"peer_id": "galbot", "id": "sdk_clock",   "hash": "..."},
        "source":     {"kind": "manual"},
        "expected_rate_hz": 10,
    },
})

# or from a JSON string:
entry2 = ResourceEntry.from_json('{"variant": "pose_log", ...}')

# pass to ClusterManager:
manager.set_resource_catalog_provider(lambda: [entry])
```

`set_resource_catalog_provider` stores the callable, not the callable's return
value. The callable is invoked for each inbound `/auki/resources/0.2.0` fetch,
so Python producers can return a live catalog that changes as backing streams
become ready or unavailable. Return only resources that can currently accept
stream opens; omit unavailable resources and re-add them later with the same
stable `resource_id`.

All four variants (`sensor_log`, `pose_log`, `time_transform_log`,
`detection_log`) are accepted. Serde discriminates by the `variant` field.
Invalid input raises `ValueError`.

## Post-#216 schema changes

The old `SensorStreamResource` / `TransformEdgeResource` / `PoseStreamResource`
pyclasses from v0.0.52 are deleted. They are replaced by the single flat
`ResourceEntry` type, which mirrors the `/auki/resources/0.2.0` wire shape.

Cross-reference:
- `ResourceEntry` shape: `docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md` §1
- `StreamRequest` / `ReadFrom` shape: spec §5
- Companion binding: [`auki-session-py`](../auki-session-py) owns the declarative
  Session API (sensor/clock/frame registration, domain join/leave). These two
  bindings are complementary, not exclusive.

## Depends on

- [`auki-domain`](../../../crates/auki-domain) — Rust crate it wraps
- [`auki-network`](../../../crates/auki-network) — `ResourceEntry`, `StreamRequest`, `ReadFrom`
- [`auki-network-py`](../auki-network-py) — shared stream types, `PyClusterEntry`, capsule bridge
- [`auki-registry`](../../../crates/auki-registry) — `RegistryRef` used in manifest dict rendering
