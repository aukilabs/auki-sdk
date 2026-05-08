# Glossary

Definitions of key terms in the Auki SDK and the surrounding real-world-web protocol. This is a seed list — entries accrete as the SDK grows.

---

## Domain

A unique identifier applied as a tag to data, asserting that the data describes a specific physical space. The tag is what lets disparate data types — RGB video, point clouds, poses, detections — be grouped as describing the same place; without intent, an RGB clip is just video.

A Domain is *not* a scenegraph and *not* a coordinate system. A Domain has zero or more **scenegraphs** tagged with it; the **Domain Owner** designates one as the canonical **Map**.

When devices network on the real world web, they discover each other and form **clusters** around shared Domain IDs (a *domain-as-topic*). On disk, Domain membership rides on a data product as a `domain_membership` [TagClaim](tags.md), not as a path or filename — Domain is one kind of tag among many.

## Domain Owner

The entity that controls a Domain — concretely, the holder of the keypair whose pubkey hashes to the Domain ID (see [`tags.md`](tags.md)). Has authority to designate a scenegraph as the Map and to issue `domain_membership` TagClaims under this Domain.

## Domain ID

The identifier for a Domain. Derived as `hash(domain_owner_pubkey)` (see [`tags.md`](tags.md)). Used as the `tag_id` in `domain_membership` TagClaims and as the topic peers cluster around on the network.

**Domain ID, Scenegraph ID, and Session ID are three distinct identifiers** — they answer different questions, and none is derivable from another:

| Identifier      | Question                              | Derivation                       |
|-----------------|---------------------------------------|----------------------------------|
| Domain ID       | which place?                          | `hash(domain_owner_pubkey)`      |
| Scenegraph ID   | which structured map of that place?   | many per Domain; Owner picks one as the Map |
| Session ID      | which recording run?                  | per-daemon UUIDv4 minted at session start |

## Cluster

The runtime group of devices networking around a shared Domain ID — a *domain-as-topic*. When devices come online and want to participate in a Domain, they discover each other (via DHT, mDNS, or a circuit relay) and form a cluster. The transport is libp2p (see [`auki-network`](crates/auki-network)); the Domain ID is what gives the cluster a reason to exist.

Cluster formation lives in higher layers; the SDK provides primitives, not the network protocol itself.

## Scenegraph

A structured representation of the spatial data for a Domain — typed nodes (frames, sensors, clocks) connected by transform edges. Evaluable at a timestamp by composition along a transform path:

```
T_X_session(t) = T_body_session(t) ∘ T_X_body(t)
```

Many scenegraphs may be tagged with the same Domain ID; they may differ in coverage, resolution, contributing data sources, or age.

## Scenegraph ID

The identifier for a specific scenegraph. Distinct from Domain ID — multiple scenegraphs can share a Domain ID; the Domain Owner picks one as the canonical Map.

## Map

The canonical scenegraph designated by a Domain Owner. The default served when a peer asks for "the map" of a Domain without specifying a Scenegraph ID. One Map at a time per Domain, but many candidate scenegraphs.

## Session ID

The identifier for a recording session — a single span of capture activity by one daemon (BoosterApp, Sentinel, etc.). Minted as a fresh UUIDv4 at daemon startup, used both as the on-disk session directory name and as the `session_id` value carried in every manifest written during the run (see [`auki-layout`](crates/auki-layout)).

Orthogonal to Domain and Scenegraph: a Session ID identifies *when and by whom* data was captured, not *what it's about*. Tying a session's data products to a Domain happens after the fact via [TagClaim](tags.md).

## App ID

The identifier for the application that produced data — same string a daemon's `/api/info` endpoint returns under `app` (e.g. `"boosterapp"`, `"sentinel"`, `"park"`). Carried in every Sensor Log / Pose Log / TimeTransform Log manifest under `app_id`. Lets a reader of a session directory know which application produced the recording without inspecting the bytes.

Orthogonal to Domain (which place?), Scenegraph (which structured map?), and Session ID (which run?). App ID answers *which application*.

## App Instance

An identifier for the specific machine an application is running on — derived by the SDK from a stable platform-level value. The current implementation (`auki-network::app_instance::derive`) reads the first non-loopback IEEE-administered MAC, sorts lexicographically for determinism, and renders as 12 lowercase hex chars without separators (e.g. `"00163eabcdef"`). Carried in `ParticipantInfo` so peers in a cluster can tell two `boosterapp` daemons apart when both register against Discovery.

Caveats: fragile in containers, VMs, and multi-NIC environments. A wallet-derived persistent stable-id alternative is parked.

## Frame

A coordinate system. In robotics and Auki's pose model, frames are typed nodes in a tree — each frame is a child of another (the body's frame, a sensor's frame, the world frame), and edges between frames are transforms. A frame's *convention* — handedness, what each axis points toward, length unit — is declared explicitly in the Frame Registry; the SDK never assumes a canonical frame.

Frame IDs follow the same `<platform-tag>-<machine-id>/<name>` convention as sensor and clock IDs (e.g. `"K1-AABBCCDDEEFF/head_left_cam_optical"`).

## Frame Registry

The third [registry](README.md) alongside Sensor + Clock. Holds [`FrameRegistryEntry`](crates/auki-registry/src/lib.rs) records — `{frame_id, handedness, axes, units}` — that declare the coordinate convention of a named frame. Lives at `<app_root>/registries/frames/<frame_id>/<hash>.json`, content-addressed by the entry's JCS hash like the other registries.

Tree structure (parent-child relations between frames) lives in the Pose Log via `TransformSample.parent_frame` / `child_frame`, not in the registry — the registry declares what each frame *is in isolation*; the Pose Log declares the edges between them. Rotation representation is fixed at the `TransformSample` layer (Hamilton convention `[x, y, z, w]`); not per-frame.

Four preset constructors cover the conventions for almost every real-world frame: `ros_body` (REP-103: right, x=forward y=left z=up, meters), `ros_optical` (REP-103 optical: right, x=right y=down z=forward, meters), `opengl` (right, x=right y=up z=backward, meters), `unity` (left, x=right y=up z=forward, meters). The on-disk JSON is fully spelled-out either way — presets are pure ergonomics.

## Coordinate convention

The four declarations that make a [Frame](#frame) interpretable: **handedness** (right or left), **axis directions** (what `+x`, `+y`, `+z` point toward semantically), **length unit** (meters / millimeters / centimeters), and **rotation representation** (fixed in this SDK at quaternion-xyzw / Hamilton convention; not per-frame). The SDK never assumes a default — every frame ships with the four declarations explicit, and the `Wallet → libp2p PeerId` model has a parallel pattern: every timestamp ships with a named clock identity, every position ships with a named frame identity.

## Pose Log

One of the [four logs](README.md). Stores per-batch transforms — `Vec<TransformSample>` per `auki-logs` entry, framing-timestamped on the daemon's clock. Each `TransformSample` carries `parent_frame` / `child_frame` strings (referencing entries in the Frame Registry), a translation `[x, y, z]` in the frame's units, and a rotation quaternion `[x, y, z, w]` (Hamilton). Multiple recordings per session distinguished by `retention_ns`. The log's manifest carries a [`PoseSource`](crates/auki-registry/src/lib.rs) inline (no Pose Source Registry — the payload is fully self-describing).

## Sensor Log

One of the [four logs](README.md). Stores per-frame sensor payloads — JPEG / NV12 frames, point cloud bytes, audio chunks — keyed via the manifest to a `(sensor_id, sensor_hash)` pair in the Sensor Registry and a `(clock_id, clock_hash)` pair in the Clock Registry. The body variant of the referenced `SensorRegistryEntry` (`RgbCamera`, `PointCloud`, `Microphone`) tells a reader how to interpret the byte payload.

## Detection Log

One of the [four logs](README.md). Stores per-frame detection outputs from extractors (object boxes, segmentation masks, keypoints). **Not yet implemented** — schema and capture path are pending.

## TimeTransform Log

One of the [four logs](README.md). Stores periodic clock-offset samples between two clocks named in the manifest's `from_id` / `to_id` pair. Lets `convert_time` (planned) translate a timestamp on clock A into the equivalent on clock B by interpolating the sampled offsets at the source timestamp. Lives at `<session>/timetransform_logs/<from_id>__<to_id>/`. Producer side ships in [`auki-time-transforms`](crates/auki-time-transforms); the consumer-side `convert_time` operation is pending.

## Pose Source

The producer of a [Pose Log](#pose-log). A tagged-enum body — `Ros2Tf { publishers }` ships in v1, with extension points for SLAM / odometry / manual fixtures. Lives **inline** in the log's manifest under `"source"` because the Pose Log payload is fully self-describing (frame names sit in each `TransformSample`); source identity is provenance, not a decoder. Cf. Sensor Log, which earns a registry because its byte payload is uninterpretable without one.

## Anchor

A coordinate-frame fix — typically a fiducial marker (QR / ArUco) at a known location, or a SLAM-recognized scene feature. An anchor lets a peer compute its pose in a domain coordinate space by observing the anchor and looking up the published pose for that anchor's `anchor_id`. **Not directly modeled by an SDK primitive** — anchors surface today as `tag_id` values in `anchor_citation` [TagClaim](tags.md)s. The associated frame is declared explicitly in the Frame Registry; the citation just asserts that the data product saw the anchor.

## Discovery

The runtime registry [`aukilabs/discovery`](https://github.com/aukilabs/discovery) that lets daemons find each other on a LAN without hardcoding `cluster.json` on every device. A Vinland-mode daemon registers its `peer_id` + addresses with Discovery via signed `POST /clusters/<name>/peers`, then fetches the full `ClusterDoc` to bootstrap its libp2p mesh. The SDK ships [`auki-network::discovery_client`](crates/auki-network/src/discovery_client.rs) (Rust) and [`auki_network.discovery.DiscoveryClient`](crates/auki-network-py/src/discovery.rs) (Python) for daemons; the registry server itself lives in a separate repo. Daemons either use Discovery or a static `cluster.json`, picked at startup — no fallback (D3).
