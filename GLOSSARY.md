# Glossary

Definitions of key terms in the Auki SDK and the surrounding real-world-web protocol. This is a seed list — entries accrete as the SDK grows.

---

## Real World Web

The protocol surface this SDK targets — peer-to-peer networks where devices in
the same physical space share a coherent view of that space (transforms,
sensor frames, detections, anchors) without a topology leader. Each peer is a
[Daemon](#daemon); peers authenticate into an exact [Domain ID](#domain-id),
and identity, signing, and content-addressing are cryptographically rooted.

## Daemon

A long-running process that reads and writes the SDK's on-disk format and
(optionally) participates in an authenticated Domain as a libp2p peer.
Concrete instances include BoosterApp, Sentinel, and Park. A daemon owns a
[Wallet](#wallet), reports an [App ID](#app-id) + [App Instance](#app-instance),
runs zero or more [recording sessions](#session-id), and may expose a
[Control API](docs/control-api.md).

## Domain

A unique identifier applied as a tag to data, asserting that the data describes a specific physical space. The tag is what lets disparate data types — RGB video, point clouds, poses, detections — be grouped as describing the same place; without intent, an RGB clip is just video.

A Domain is *not* a scenegraph and *not* a coordinate system. A Domain has zero or more **scenegraphs** tagged with it; the **Domain Owner** designates one as the canonical **Map**.

When devices network on the real world web, each `Domain` runtime is configured
with one exact DDS Domain UUID. A DDS-signed P2P access token is the baseline
admission rule; explicit routes describe reachability but grant no authority.

## Domain Owner

The entity that controls Domain policy in DDS and can designate a scenegraph as
the Map. Runtime access is represented by DDS-signed Domain credentials rather
than an SDK-local Manager or membership roster.

## Domain ID

The canonical DDS Domain UUID. The same UUID is carried in local configuration,
signed P2P credentials, mutual-authentication requirements, and peer
observations.

## Domain Identity

The network identity of a Domain is its canonical DDS UUID. Human-readable
names and application indexing may exist outside the P2P runtime, but they do
not replace or derive the authority identifier.

**Domain ID, Scenegraph ID, and Session ID are three distinct identifiers** — they answer different questions, and none is derivable from another:

| Identifier      | Question                              | Derivation                       |
|-----------------|---------------------------------------|----------------------------------|
| Domain ID       | which place?                          | DDS UUID                         |
| Scenegraph ID   | which structured map of that place?   | many per Domain; Owner picks one as the Map |
| Session ID      | which recording run?                  | per-daemon UUIDv4 minted at session start |

## Cluster (retired SDK model)

The removed Manager-era runtime grouped peers into a mutable cluster roster.
The authenticated SDK does not expose that product model: peers independently
hold signed access to a DDS Domain and become known only after successful
mutual authentication on a live connection.

## ClusterDoc (retired SDK model)

The removed runtime used a peer/address roster as both topology and authority.
The replacement separates those concerns: `DomainRoutes` stores explicit dial
hints, while signed Domain credentials plus the Noise Peer ID decide access.

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

## convert_time

One of the SDK's two core operations. Translates a timestamp on one [clock](#clock-registry) into the equivalent timestamp on another, by interpolating the offset samples in a [TimeTransform Log](#timetransform-log) at the source timestamp. Lets a downstream consumer correlate data captured under different clocks (e.g. ROS 2 wall-clock, robot session-monotonic, peer-supplied UTC) without picking a "canonical" clock — every timestamp ships with a named clock identity, and `convert_time` is what bridges them.

The producer/math side ships in [`auki-time`](crates/auki-time) (fixed affine
transforms and the local sampler that writes explicit clock relations); the
consumer-side composition is pending.

## convert_pose

One of the SDK's two core operations. Translates a pose (translation + rotation) from one [Frame](#frame) into the equivalent in another, by composing transform edges along a path through the [Pose Log](#pose-log). Each edge is a [SpatialTransform](#spatialtransform) sample in a Pose Log keyed per `(from_frame_id, to_frame_id)`; the path traverses the frame pairs pinned in each log's manifest. Like `convert_time`, `convert_pose` lets consumers translate across coordinate systems without a canonical frame — every position ships with a named frame identity.

Pose Log capture is in place; the consumer-side composition / path-finding is pending.

## Wallet

The identity primitive — an ed25519 keypair plus deterministic child derivation (label-based, like BIP32 but simpler). One wallet seed regenerates every derived key on a fresh machine: the [Peer ID](#peer-id) (`derive_child("peer/v1")`), per-Domain owner keys, signing keys for [TagClaims](#tagclaim), and so on. Foundational for content-addressing and signing across the SDK; ships in [`auki-identity`](crates/auki-identity), WASM-friendly.

## Peer ID

The libp2p identifier used in `/p2p/<peer-id>` multiaddrs and bound by Noise
during mutual authentication. Derived from a [Wallet](#wallet) via
`Wallet::derive_child("peer/v1")` and the standard libp2p public-key chain.
Same wallet seed means the same Peer ID across machines and reboots.

## Session ID

The identifier for a recording session — a single span of capture activity by one daemon (BoosterApp, Sentinel, etc.). Minted as a fresh UUIDv4 at daemon startup, used both as the on-disk session directory name and as the `session_id` value carried in every manifest written during the run (see [`auki-layout`](crates/auki-layout)).

Orthogonal to Domain and Scenegraph: a Session ID identifies *when and by whom* data was captured, not *what it's about*. Tying a session's data products to a Domain happens after the fact via [TagClaim](#tagclaim).

## App ID

The identifier for the application that produced data — same string a daemon's `ParticipantInfo` carries under `app` (e.g. `"boosterapp"`, `"sentinel"`, `"park"`). Carried in every Sensor Log / Pose Log / TimeTransform Log manifest under `app_id`. Lets a reader of a session directory know which application produced the recording without inspecting the bytes.

Orthogonal to Domain (which place?), Scenegraph (which structured map?), and Session ID (which run?). App ID answers *which application*.

## App Instance

An identifier for the specific machine an application is running on — derived
by the SDK from a stable platform-level value. The current implementation
(`auki-network::app_instance::derive`) reads the first non-loopback
IEEE-administered MAC, sorts lexicographically for determinism, and renders 12
lowercase hex characters. It is optional diagnostic metadata, never authority.

Caveats: fragile in containers, VMs, and multi-NIC environments. A wallet-derived persistent stable-id alternative is parked.

## Sensor / Clock / Frame ID

The id format used for sensors, clocks, and frames:
`<platform-tag>-<machine-id>/<name>`. The platform tag and machine id make the
prefix locally unique; the trailing name is producer-scoped. Resource,
registry, and stream requests combine this id with a content-addressed hash.

## Frame

A coordinate system. In robotics and Auki's pose model, frames are typed nodes in a tree — each frame is a child of another (the body's frame, a sensor's frame, the world frame), and edges between frames are transforms. A frame's *convention* — handedness, what each axis points toward, length unit — is declared explicitly in the Frame Registry; the SDK never assumes a canonical frame.

Frame IDs follow the same `<platform-tag>-<machine-id>/<name>` convention as sensor and clock IDs (e.g. `"K1-AABBCCDDEEFF/head_left_cam_optical"`).

## Sensor Registry

The [registry](README.md) of named sensor configurations. `SensorRegistryEntry` records describe what a sensor *is* — camera intrinsics, point-cloud field layout, audio sample format — so that a [Sensor Log](#sensor-log) reader can interpret the per-frame byte payload. Lives at `<app_root>/registries/sensors/<sensor_id>/<hash>.json`, content-addressed by JCS hash; the active configuration is whichever hash sits in the live log's manifest. Implementation in [`auki-registry`](crates/auki-registry).

## Clock Registry

The [registry](README.md) of named clocks. `ClockRegistryEntry` records describe a clock's epoch, monotonicity guarantees, and provenance (e.g. system `CLOCK_REALTIME`, ROS 2 sim time, session-monotonic). Lives at `<app_root>/registries/clocks/<clock_id>/<hash>.json`, content-addressed by JCS hash. Every timestamp the SDK writes references a `(clock_id, clock_hash)` pair — this registry is what makes that reference resolvable, and what [convert_time](#convert_time) crosses between. Implementation in [`auki-registry`](crates/auki-registry).

## Frame Registry

The third [registry](README.md) alongside Sensor + Clock. Holds [`FrameRegistryEntry`](crates/auki-registry/src/lib.rs) records — `{frame_id, handedness, axes, units}` — that declare the coordinate convention of a named frame. Lives at `<app_root>/registries/frames/<frame_id>/<hash>.json`, content-addressed by the entry's JCS hash like the other registries.

Tree structure (parent-child relations between frames) lives in the Pose Log: each Pose Log is keyed per `(from_frame_id, to_frame_id)` pair pinned in its manifest. The registry declares what each frame *is in isolation*; the Pose Log manifests declare the edges between them. Rotation representation is fixed at the [SpatialTransform](#spatialtransform) layer (Hamilton quaternion `(x, y, z, w)`); not per-frame.

Four preset constructors cover the conventions for almost every real-world frame: `ros_body` (REP-103: right, x=forward y=left z=up, meters), `ros_optical` (REP-103 optical: right, x=right y=down z=forward, meters), `opengl` (right, x=right y=up z=backward, meters), `unity` (left, x=right y=up z=forward, meters). The on-disk JSON is fully spelled-out either way — presets are pure ergonomics.

## Coordinate convention

The four declarations that make a [Frame](#frame) interpretable: **handedness** (right or left), **axis directions** (what `+x`, `+y`, `+z` point toward semantically), **length unit** (meters / millimeters / centimeters), and **rotation representation** (fixed in this SDK at quaternion-xyzw / Hamilton convention; not per-frame). The SDK never assumes a default — every frame ships with the four declarations explicit, and the `Wallet → libp2p PeerId` model has a parallel pattern: every timestamp ships with a named clock identity, every position ships with a named frame identity.

## Manifest

The per-recording metadata sidecar — a JCS-canonical UTF-8 JSON file at the root of every [log](README.md)'s directory. Carries the identity references the segment payloads need (`sensor_id` + `sensor_hash`, `clock_id` + `clock_hash`, `session_id`, `app_id`, etc.) plus the rollover/retention parameters (`segment_duration_ns`, `retention_ns`, `duration_ns`). Schemas and builders live in [`auki-manifests`](crates/auki-manifests); sibling crate to [`auki-datatypes`](crates/auki-datatypes), which owns the segment payload schemas.

## SpatialTransform

The data type at the core of [convert_pose](#convert_pose) — a translation `Vec3 { x, y, z }` plus a rotation quaternion `Quat { x, y, z, w }` (Hamilton convention). Stored as a flat segment entry in the [Pose Log](#pose-log); the `(from_frame_id, to_frame_id)` identity lives in the manifest, not on each sample. Implementation is `auki_datatypes::pose::SpatialTransform` (prost-encoded); the rename from the pre-migration `TransformSample` shape (per-sample frame labels) landed at Step 5 of the auki-datatypes migration on 2026-05-08.

## Pose Log

One of the [four logs](README.md). Stores per-sample [SpatialTransform](#spatialtransform) entries for one `(from_frame_id, to_frame_id)` pair — same shape as TimeTransform Logs, which key per ordered clock pair. Lives at `<session>/poselogs/<from_id>__<to_id>/`. The auki-logs framing's `timestamp_ns` is the sample time on the manifest's clock; per-frame translation + Hamilton quaternion. The log's manifest pins both Frame Registry entries via `(from_frame_id, from_frame_hash) + (to_frame_id, to_frame_hash)`, the inline [Pose Source](#pose-source) provenance tag, plus `writer_mode` (`"rigid"` vs `"movable"`) and `expected_rate_hz` hints per the synthesis decided 2026-05-07. A producer that observes a multi-pair ROS `TFMessage` fans the message into N parallel pose logs.

## Sensor Log

One of the [four logs](README.md). Stores per-frame sensor payloads — JPEG / NV12 frames, point cloud bytes, audio chunks — keyed via the manifest to a `(sensor_id, sensor_hash)` pair in the Sensor Registry and a `(clock_id, clock_hash)` pair in the Clock Registry. The body variant of the referenced `SensorRegistryEntry` (`RgbCamera`, `PointCloud`, `Microphone`) tells a reader how to interpret the byte payload.

## Detection Log

One of the [four logs](README.md). Stores per-frame detection outputs from extractors (object boxes, segmentation masks, keypoints). **Not yet implemented** — schema and capture path are pending.

## TimeTransform Log

One of the [four logs](README.md). Stores periodic clock-offset samples between two clocks named in the manifest's `(from_clock_id, to_clock_id)` pair — flat `TimeTransformEntry { offset_ns, uncertainty_ns }` entries (`auki_datatypes::time_transform`, prost-encoded since Step 6, 2026-05-08). Lets `convert_time` (planned) translate a timestamp on clock A into the equivalent on clock B by interpolating the sampled offsets at the source timestamp. Lives at `<session>/timetransform_logs/<from_id>__<to_id>/`. The manifest carries the inline `TimeTransformSource` provenance tag (`LocalClockRead` ships today; mirrors `PoseSource`'s extension pattern). Discontinuity detection (was a per-entry bool pre-migration) is reader-side now — readers compute it against their own threshold. Producer side ships in [`auki-time`](crates/auki-time); the consumer-side `convert_time` operation is pending.

## Pose Source

The producer of a [Pose Log](#pose-log). A tagged-enum body — `Ros2Tf { publishers }` ships in v1, with extension points for SLAM / odometry / manual fixtures. Lives **inline** in the log's manifest under `"source"` because frame identity (`from_frame_id`, `to_frame_id`) is also in the manifest — source is just provenance, not a decoder. Cf. Sensor Log, which earns a registry because its byte payload is uninterpretable without one. Implementation in [`auki-manifests`](crates/auki-manifests).

## Anchor

A coordinate-frame fix — typically a fiducial marker (QR / ArUco) at a known location, or a SLAM-recognized scene feature. An anchor lets a peer compute its pose in a domain coordinate space by observing the anchor and looking up the published pose for that anchor's `anchor_id`. **Not directly modeled by an SDK primitive** — anchors surface today as `tag_id` values in `anchor_citation` [TagClaim](#tagclaim)s. The associated frame is declared explicitly in the Frame Registry; the citation just asserts that the data product saw the anchor.

## TagClaim

A signed assertion that some data product has a property — e.g. *"this Pose Log is part of `domain_X`"*, *"this RGB clip cited anchor `Y`"*, *"the prior claim referenced here is hereby revoked"*. Issued by the holder of an issuer wallet; attached to the data via an append-only `tags.jsonl` sidecar next to the log's manifest, separate from the manifest itself (which is treated as immutable). Append-only by design — revocation is a *new* claim of `claim_type: "revoke"` referencing the prior claim's hash. The full v0 schema and claim taxonomy previously lived in a dedicated `tags.md`, since removed (see `git show 56f8037` for the last version); it has not been re-documented elsewhere yet.

## Discovery

Any host/application mechanism that supplies explicit peer routes. Static
configuration, DDS/DMS adapters, mDNS, or another discovery service may all do
this without changing authentication. Core `auki-p2p`, `auki-network`, and
`auki-domain` perform no Discovery HTTP requests and never treat route
knowledge as authorization.
