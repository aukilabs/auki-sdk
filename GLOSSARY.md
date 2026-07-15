# Glossary

Definitions of key terms in the Auki SDK and the surrounding real-world-web protocol. This is a seed list — entries accrete as the SDK grows.

---

## Real World Web

The protocol surface this SDK targets — peer-to-peer networks where devices in the same physical space share a coherent view of that space (transforms, sensor frames, detections, anchors) without a central authority. Each peer is a [Daemon](#daemon); peers organize into [clusters](#cluster) around shared [Domain IDs](#domain-id); identity, signing, and content-addressing are wallet-rooted. The SDK ships the primitives the protocol composes from — registries, logs, transforms — not the protocol itself.

## Daemon

A long-running process that reads and writes the SDK's on-disk format and (optionally) participates in clusters as a libp2p peer. Concrete instances: BoosterApp (K1 sensor capture), Sentinel (extractor pipeline), Park (visualizer). A daemon owns a [Wallet](#wallet), reports an [App ID](#app-id) + [App Instance](#app-instance), runs zero or more [recording sessions](#session-id), and exposes a [Control API](docs/control-api.md). The SDK is the substrate; daemons are the consumers.

## Domain

A unique identifier applied as a tag to data, asserting that the data describes a specific physical space. The tag is what lets disparate data types — RGB video, point clouds, poses, detections — be grouped as describing the same place; without intent, an RGB clip is just video.

A Domain is *not* a scenegraph and *not* a coordinate system. A Domain has zero or more **scenegraphs** tagged with it; the **Domain Owner** designates one as the canonical **Map**.

When devices network on the real world web, they discover each other and form **clusters** around shared Domain IDs (a *domain-as-topic*). On disk, Domain membership rides on a data product as a `domain_membership` [TagClaim](#tagclaim), not as a path or filename — Domain is one kind of tag among many.

## Domain Owner

The entity that controls a Domain — concretely, the holder of the keypair whose pubkey hashes to the Domain ID. Has authority to designate a scenegraph as the Map and to issue `domain_membership` [TagClaim](#tagclaim)s under this Domain.

## Domain ID

The identifier for a Domain. Derived as `hash(domain_owner_pubkey)`. Used as the `tag_id` in `domain_membership` [TagClaim](#tagclaim)s.

## Domain Identity

The canonical string a Domain is indexed by on the network — what peers register with on Discovery and cluster around. `{Domain ID}/{name}` for user-named Domains (e.g. `d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a/demo-2026-05`), letting one wallet own multiple distinguishable Domains. One exception: the reserved `"Vinland"` singleton (the default Domain headless daemons fall back to when no Domain exists yet) has the canonical identity `Vinland` with no wallet prefix; Discovery serializes its creation so the singleton property holds. See [`crates/auki-domain`](crates/auki-domain) for the construction primitive ([`DomainIdentity`](crates/auki-domain/src/lib.rs)).

**Domain ID, Scenegraph ID, and Session ID are three distinct identifiers** — they answer different questions, and none is derivable from another:

| Identifier      | Question                              | Derivation                       |
|-----------------|---------------------------------------|----------------------------------|
| Domain ID       | which place?                          | `hash(domain_owner_pubkey)`      |
| Scenegraph ID   | which structured map of that place?   | many per Domain; Owner picks one as the Map |
| Session ID      | which recording run?                  | per-daemon UUIDv4 minted at session start |

## Cluster

The runtime group of devices networking around a shared Domain ID — a *domain-as-topic*. When devices come online and want to participate in a Domain, they discover each other (via DHT, mDNS, or a circuit relay) and form a cluster. The transport is libp2p (see [`auki-network`](crates/auki-network)); the Domain ID is what gives the cluster a reason to exist.

Cluster formation lives in higher layers; the SDK provides primitives, not the network protocol itself.

## ClusterDoc

The cluster's discovery doc — a JSON file (typically `cluster.json`) enumerating the peers participating in the cluster. Each entry pairs a [Peer ID](#peer-id) with its dialable multiaddrs and named capabilities. Distributed either statically (every peer reads `cluster.json` from disk at boot) or dynamically via [Discovery](#discovery). The ClusterDoc is the trust boundary — peers not in the doc cannot dial; libp2p's Noise layer refuses connections from unknown Peer IDs. See [`auki-network`](crates/auki-network).

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

The producer/math side ships in [`auki-time`](crates/auki-time) (pure transforms, NTP-style samples, and the sampler that writes the log); the consumer-side composition is pending.

## convert_pose

One of the SDK's two core operations. Translates a pose (translation + rotation) from one [Frame](#frame) into the equivalent in another, by composing transform edges along a path through the [Pose Log](#pose-log). Each edge is a [SpatialTransform](#spatialtransform) sample in a Pose Log keyed per `(from_frame_id, to_frame_id)`; the path traverses the frame pairs pinned in each log's manifest. Like `convert_time`, `convert_pose` lets consumers translate across coordinate systems without a canonical frame — every position ships with a named frame identity.

Pose Log capture is in place; the consumer-side composition / path-finding is pending.

## Wallet

The identity primitive — an ed25519 keypair plus deterministic child derivation (label-based, like BIP32 but simpler). One wallet seed regenerates every derived key on a fresh machine: the [Peer ID](#peer-id) (`derive_child("peer/v1")`), per-Domain owner keys, signing keys for [TagClaims](#tagclaim), and so on. Foundational for content-addressing and signing across the SDK; ships in [`auki-identity`](crates/auki-identity), WASM-friendly.

## Peer ID

The libp2p identifier used in [`cluster.json`](#clusterdoc), `/p2p/<peer-id>` multiaddrs, and the trust check at connection time. Derived from a [Wallet](#wallet) via `Wallet::derive_child("peer/v1")` then libp2p's standard `PublicKey → multihash → multibase-base58` chain. Same wallet seed → same Peer ID, deterministic across machines and reboots.

## Session ID

The identifier for a recording session — a single span of capture activity by one daemon (BoosterApp, Sentinel, etc.). Minted as a fresh UUIDv4 at daemon startup, used both as the on-disk session directory name and as the `session_id` value carried in every manifest written during the run (see [`auki-layout`](crates/auki-layout)).

Orthogonal to Domain and Scenegraph: a Session ID identifies *when and by whom* data was captured, not *what it's about*. Tying a session's data products to a Domain happens after the fact via [TagClaim](#tagclaim).

## App ID

The identifier for the application that produced data — same string a daemon's `ParticipantInfo` carries under `app` (e.g. `"boosterapp"`, `"sentinel"`, `"park"`). Carried in every Sensor Log / Pose Log / TimeTransform Log manifest under `app_id`. Lets a reader of a session directory know which application produced the recording without inspecting the bytes.

Orthogonal to Domain (which place?), Scenegraph (which structured map?), and Session ID (which run?). App ID answers *which application*.

## App Instance

An identifier for the specific machine an application is running on — derived by the SDK from a stable platform-level value. The current implementation (`auki-network::app_instance::derive`) reads the first non-loopback IEEE-administered MAC, sorts lexicographically for determinism, and renders as 12 lowercase hex chars without separators (e.g. `"00163eabcdef"`). Carried in `ParticipantInfo` so peers in a cluster can tell two `boosterapp` daemons apart when both register against Discovery.

Caveats: fragile in containers, VMs, and multi-NIC environments. A wallet-derived persistent stable-id alternative is parked.

## Sensor / Clock / Frame ID

The id format used for sensors, clocks, and frames: `<platform-tag>-<machine-id>/<name>`. Examples: `K1-AABBCCDDEEFF/head_left_cam` (RGB camera on a K1 robot), `K1-AABBCCDDEEFF/utc` (clock), `K1-AABBCCDDEEFF/head_left_cam_optical` (frame). The platform-tag and machine-id together make the prefix locally unique to the producing daemon; the trailing name is producer-scoped. The full id is what consumers pass to discovery and stream protocols; the corresponding registry entry pins the configuration via its content-addressed hash.

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

The runtime registry [`aukilabs/discovery`](https://github.com/aukilabs/discovery) that lets daemons find each other on a LAN without hardcoding `cluster.json` on every device. A daemon using Discovery registers its `peer_id` + addresses via signed `POST /clusters/<name>/peers`, then fetches the full [ClusterDoc](#clusterdoc) to bootstrap its libp2p mesh. The SDK ships [`auki-network::discovery_client`](crates/auki-network/src/discovery_client.rs) (Rust) and [`auki_network.discovery.DiscoveryClient`](bindings/python/auki-network-py) (Python) for daemons; the registry server itself lives in a separate repo. Daemons either use Discovery or a static `cluster.json`, picked at startup — no fallback.
