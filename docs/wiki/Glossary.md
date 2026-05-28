# Glossary

Long-form companion to the repo's [`GLOSSARY.md`](https://github.com/aukilabs/auki-sdk/blob/develop/GLOSSARY.md). `GLOSSARY.md` is the authoritative one-line source; this page expands the SDK's most-encountered terms with where they appear in code, what gets confused with what, and where to read more.

If you're new, skim top-to-bottom — terms are ordered roughly from identity outward to network-level concepts. If you're looking one up, jump to it.

---

## Peer

A device participating in the Auki network — addressed by a libp2p `PeerId` derived from a [Wallet](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-identity) via `Wallet::derive_child("peer/v1")`. Same wallet seed produces the same `PeerId` across reboots and reinstalls; a peer's identity is durable per device, not per process.

**In code:** `auki_identity::Wallet`, `auki_network::PeerIdentity`, `peer_id: PeerId` fields on every registry entry, log manifest, and catalog row post-#216.

**Common confusions:**
- *Peer vs session.* `peer_id` is **durable**; a peer reboots and keeps its peer_id. The `session_id` changes every boot.
- *Peer vs app.* One device (peer) typically runs one or more apps; the same peer can serve `galbot-ctrl` and `galbot-teleop` data products simultaneously.

**See also:** [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs), [The Five Questions § Identity](The-Five-Questions#identity--who-am-i).

---

## Session

One process boot of an app on a peer. A `Session` is the SDK's app-facing entry point — it declares which peer / app it represents, holds the registry and log handles for the lifetime of the process, and (optionally) joins a cluster on the user's behalf.

**In code:** `auki_session::Session`. Constructed with `Session::new(peer_id, app_id)`; the SDK fills in a fresh ULID `session_id`. See `crates/auki-session/tests/end_to_end.rs` for a complete worked example.

**Common confusions:**
- *Session vs domain participation.* A session can register logs without ever joining a domain — `Session::join_domain` is opt-in. Domain membership is orthogonal to session lifetime.
- *Session vs HTTP session.* Unrelated to web "sessions" — this is a process-boot scope.

**See also:** [Quickstart](Quickstart), [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs).

---

## App

The application a session belongs to. The `app_id` is a caller-supplied string (`"boosterapp"`, `"galbot-ctrl"`, `"park-vis"`, …) carried in every log manifest written during the session. Lets a reader of a session directory know which application wrote the data without inspecting the bytes.

**In code:** `Session::app_id()`, the `app_id: String` field on every manifest (`SensorLogManifest`, `PoseLogManifest`, etc.) in `auki-manifests`.

**Common confusions:**
- *App vs peer.* The same peer can run multiple apps; the same app can run on multiple peers. The peer_id × app_id pair identifies a producer; either alone does not.
- *App vs session.* The app_id is stable across sessions of the same app; the session_id is fresh per boot.

**See also:** [The Five Questions § Identity](The-Five-Questions#identity--who-am-i).

---

## Domain

A unique identifier — derived as `hash(domain_owner_pubkey)` — applied as a tag to data, asserting the data describes a specific physical space. Domains are how the Auki network groups data across data types: an RGB clip, a point-cloud capture, and a robot pose log all tagged with the same Domain ID are all *about the same place*.

A domain is **not** a scenegraph (the structured map of a space; many possible per domain), and **not** a session (one process boot of a daemon). The three IDs are independent and not derivable from each other.

**In code:** [`auki-domain`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-domain) — `ClusterManager` is the runtime layer; `Session::join_domain` is the app-facing API. The Domain ID itself surfaces in `DomainIdentity` / `ClusterTarget`.

**Common confusions:**
- *Domain vs cluster.* The Domain is the *topic* (the conceptual identifier); the **cluster** is the runtime group of devices networking around that topic.
- *Domain vs scenegraph.* One domain can have many scenegraphs (different structured maps of the same place); the Domain Owner picks one as the canonical **Map**.

**See also:** [`crates/auki-domain/README.md`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-domain/README.md), [`GLOSSARY.md`](https://github.com/aukilabs/auki-sdk/blob/develop/GLOSSARY.md#domain-id) for the Domain ID / Scenegraph ID / Session ID table.

---

## Log

A peer-owned timestamped data product. The SDK has four log variants — each addresses a different question:

| Log | What it stores |
|-----|---------------|
| **Sensor Log** | Per-sample sensor payloads (camera frames, point clouds, audio samples, joint encoders) |
| **Pose Log** | `from → to` transforms over time |
| **Detection Log** | Per-frame detector outputs (boxes, masks, keypoints) |
| **TimeTransform Log** | Sampled offsets between two clocks |

Every log carries a `source_peer_id` (canonical owner) and a `writer_peer_id` (whoever wrote *this* manifest). For origin logs they're equal; for materialized copies they differ. See [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs).

**In code:** the lower-level primitive is `auki_logs::Log<T>` (encoder-agnostic, segmented ring-buffer). The app-facing surface is `auki_session::Session::register_*_log` for each of the four variants, returning a typed handle (`SensorLogHandle`, etc.).

**Common confusions:**
- *"A log" vs a log file.* A log is a *data product*: manifest + segments + identity. The underlying file storage is an implementation detail.
- *Log vs registry.* A log is *timestamped* — it records samples over time. A registry holds *long-lived configuration* (sensor specs, clock definitions, frame conventions) — content-addressed and versioned, not timestamped.

**See also:** [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs), [The Five Questions § Spatial / Temporal](The-Five-Questions#spatial--where-did-this-happen).

---

## Manifest

The per-log JSON sidecar at the root of every log's directory. Carries the identity references the segment payloads need — `peer_id`s, registry refs (`sensor`, `clock`, `frame`, ...), session and app IDs — plus the writer-local properties (`segment_duration_ns`, `retention_ns`).

Post-#216, every manifest splits its fields into two groups:

- **Canonical** (preserved across materializations): `source_peer_id`, registry refs, `source`, `writer_mode`, `expected_rate_hz`.
- **Writer-local** (set by this file's writer): `writer_peer_id`, `app_id`, `session_id`, `segment_duration_ns`, `retention_ns`.

**In code:** `auki-manifests` — `SensorLogManifest`, `PoseLogManifest`, `TimeTransformLogManifest`, `DetectionLogManifest`. Encoded as JCS-canonical JSON via `auki-jcs` so the manifest's content hash is byte-stable.

**Common confusions:**
- *Manifest vs catalog row.* The manifest is on disk and contains the *full* canonical + writer-local state. The catalog row is the wire-level advertisement of *what's available* — it carries the canonical fields plus an `available` snapshot, but not the writer-local rollover params (those are the writer's choice and don't affect interpretation).
- *Manifest vs segment.* The manifest describes what the segments contain; segments are the payload bytes.

**See also:** [Concept: Peer-Owned Logs § Manifest](Concept-Peer-Owned-Logs#mapping-the-concept-to-code-and-wire), [`crates/auki-manifests/README.md`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-manifests/README.md).

---

## Registry

A content-addressed catalog of long-lived entities. Four registries exist in the SDK:

- **Sensor Registry** — what a sensor *is* (intrinsics, format, frame_rate)
- **Clock Registry** — what a clock *is* (unit, monotonicity, scope, epoch)
- **Frame Registry** — what a frame *is* (handedness, axes, units)
- **Detector Registry** — what a detector *is* (model, output types)

Each entry is keyed by `(peer_id, id, hash)` — the hash is XXH3-128 over the entry's JCS-canonical JSON. The hash *is* the version; refining an entry produces a new sibling row under the same id.

**In code:** `auki-registry` — `SensorRegistryEntry`, `ClockRegistryEntry`, `FrameRegistryEntry`, `DetectorRegistryEntry`. Each post-#216 entry carries an explicit `peer_id` top-level field. App-facing: `Session::register_{sensor,clock,frame,detector}` returns a `RegistryRef { peer_id, id, hash }`.

**Common confusions:**
- *Registry vs log.* Registries hold *what something is*; logs hold *what happened*. A `SensorRegistryEntry` says "this camera is 1920×1200 at 30 Hz"; a `SensorLog` carries the frames the camera produced.
- *Registry ID format.* IDs reject `>`, `@`, and whitespace because those characters are used as separators in `resource_id` derivation (`from->to`, `detector@sensor`). Enforced at `Session::register_*` time.

**See also:** [Quickstart](Quickstart) (registering each entry type), [`crates/auki-registry/README.md`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-registry/README.md).

---

## Frame

A coordinate system — a node in the SDK's transform graph. A frame's **convention** (handedness, what each axis points toward, length unit) is declared explicitly in the Frame Registry; the SDK never assumes a canonical frame.

Four presets cover most cases: `FrameDef::ros_body()` (REP-103 body: x=forward, y=left, z=up, right-handed, meters), `FrameDef::ros_optical()` (REP-103 optical: x=right, y=down, z=forward, right-handed, meters), `FrameDef::opengl()` (x=right, y=up, z=back, right-handed, meters), `FrameDef::unity()` (x=right, y=up, z=forward, left-handed, meters). Custom conventions are spelled out in full in the registry entry.

**In code:** `auki_registry::FrameRegistryEntry`, `auki_session::FrameDef`. Registered via `Session::register_frame(frame_id, FrameDef)` → `RegistryRef`.

**Common confusions:**
- *Frame vs frame (data).* "Frame" in the SDK means a *coordinate system*, not a video frame. Camera samples in a Sensor Log are sometimes called "frames" colloquially — but `auki_datatypes::camera::CameraFrame` is the segment payload, distinct from a `FrameRegistryEntry`.
- *Frame tree structure.* The registry declares what each frame *is in isolation*. The edges between frames live in Pose Logs — one log per `(from_frame, to_frame)` pair. The "frame tree" is the graph of those edges.

**See also:** [Quickstart](Quickstart), [`crates/auki-geometry/README.md`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-geometry/README.md) for convention conversion.

---

## Clock

A named time source — described by a `ClockRegistryEntry` with explicit `unit`, `monotonic`, `epoch`, and `scope` (`device-local`, `global`). The SDK refuses to canonicalize on any one clock; every timestamp ships with a named clock identity, and conversions between clocks are first-class data products called TimeTransform Logs.

**In code:** `auki_registry::ClockBody::{MonotonicClock, UtcClock}` and related variants. Registered via `Session::register_clock(clock_id, ClockBody)`. Live cluster-wide estimates compose through `auki_time::estimate_domain_clock` (heartbeat-driven, in-memory).

**Common confusions:**
- *"What time is it?" has no canonical answer.* There is no `current_time_ns()` global. Each timestamp is paired with the clock it was measured against; `convert_time` (planned) bridges them.
- *Domain time vs wall clock.* The cluster-wide domain clock estimate (live, in-memory, heartbeat-driven) is a separate concept from the per-peer `local_clock_read` TimeTransform Log (1 Hz, persisted, monotonic ↔ realtime within one device).

**See also:** [The Five Questions § Temporal](The-Five-Questions#temporal--when-did-this-happen), [`crates/auki-time/README.md`](https://github.com/aukilabs/auki-sdk/blob/develop/crates/auki-time/README.md).

---

## Catalog row

The wire-level advertisement of one peer-owned log. One catalog row per log; served by the peer over `/auki/resources/0.2.0` as a `ResourceEntry`.

Every row carries `source_peer_id`, `writer_peer_id`, `resource_id`, `state` (`"live"` / `"sealed"`), a `variant` discriminator (`sensor_log` / `pose_log` / `time_transform_log` / `detection_log`), and a variant-specific `manifest` block with the registry refs a consumer needs.

**In code:** `auki_network::resources_protocol::ResourceEntry`. Produced by `Session::catalog()`; the full row shape is documented in [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md).

**Common confusions:**
- *Catalog row vs manifest.* The catalog row is the **wire** shape of a log's advertisement; the manifest is the **on-disk** shape. They overlap heavily (both carry the canonical registry refs and identity fields) but the catalog row adds an `available` snapshot and omits the writer-local rollover params.
- *Variant axis vs sensor.kind axis.* `variant` (closed enum, 4 values) discriminates *what kind of log this is*. `sensor.kind` (closed enum, 5 values: camera / rangefinder / rf / audio / joint_encoders) lives inside the `sensor` block on `sensor_log` rows only. The two are orthogonal.

**See also:** [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md), [Concept: Peer-Owned Logs § Catalog row](Concept-Peer-Owned-Logs#mapping-the-concept-to-code-and-wire).

---

## Materialization

The act of one peer copying a log it doesn't own. Park materializing Galbot's `head_left_rgb` writes a new local manifest with `source_peer_id = "galbot"` (preserved) and `writer_peer_id = "park"` (Park's identity), Park-chosen `retention_ns` and `segment_duration_ns`, and the same registry refs (pointing at Galbot's canonical entries).

Materialization is **caching with attribution**, not authorship transfer. The materialized row still says the data is Galbot's; Park is just the convenient nearby copy.

**In code:** `Session::materialize_remote_log(log_ref, retention, segment_duration)` — currently returns `NotImplemented`. Tracked as Phase 5 of [#216](https://github.com/aukilabs/auki-sdk/issues/216).

**Common confusions:**
- *Materialization vs ownership transfer.* `source_peer_id` is **never** rewritten. There is no scenario in which Park's materialized copy of Galbot's data shows Park as the source.
- *Materialization vs a single byte fetch.* Materialization persists a full local replica with its own retention. Opening a one-off stream against Galbot via `/auki/stream/0.2.0` is a different operation — no local copy, no manifest.

**See also:** [Concept: Peer-Owned Logs § Materialization preserves identity](Concept-Peer-Owned-Logs#materialization-preserves-identity-not-labels).

---

## Sensor kind / type

The two-axis taxonomy that identifies a sensor on the wire and in the registry:

- **`sensor.kind`** — closed enum, five values: `camera`, `rangefinder`, `rf`, `audio`, `joint_encoders`. Family-level.
- **`sensor.type`** — open string, kind-scoped. Common values: `camera`: `rgb`/`depth`/`ir`/`mono`/`multispectral`; `rangefinder`: `point_cloud`/`2d_lidar`/`3d_lidar`/`ultrasonic`/`radar`; `rf`: `wifi`/`bluetooth`/`uwb`; `audio`: `pcm`/`opus`; `joint_encoders`: `absolute`/`incremental`.

A consumer that wants "all lidar streams" filters on `sensor.kind = "rangefinder"`; one that wants 3D lidar specifically filters on `sensor.kind = "rangefinder" AND sensor.type = "3d_lidar"`.

**In code:** `auki_network::resources_protocol::SensorKind` (closed enum, mirrored across Rust, Python, and locked JSON fixtures). The `type` is a `String` on each `SensorBody` variant. The catalog `kind` value mirrors the registry body's serde tag — invariant enforced via locked tests.

**Common confusions:**
- *Kind vs type collapsed into one field.* The pre-#216 schema used a single `sensor.kind` field that conflated `point_cloud` (a sensor modality) with `camera` (a sensor family). #216 split them: `point_cloud` is now `kind=rangefinder, type=point_cloud`.
- *"My type isn't in the documented list."* The type is open — producers may use unlisted values; consumers MUST handle unknown types gracefully (fall back to the kind-level handler or ignore the row).

**See also:** [`dataproducts.md` § Closed sensor kinds](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md), [#216 design spec § 1](https://github.com/aukilabs/auki-sdk/blob/develop/docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md).

---

## See also

- [`GLOSSARY.md`](https://github.com/aukilabs/auki-sdk/blob/develop/GLOSSARY.md) — the authoritative one-line definitions, plus terms not yet expanded here (Domain Owner, Scenegraph, Map, Anchor, TagClaim, Discovery, etc.).
- [Concept: Peer-Owned Logs](Concept-Peer-Owned-Logs) — long-form on the SDK's central data invariant.
- [The Five Questions](The-Five-Questions) — where each term fits in the architectural frame.
- [Quickstart](Quickstart) — see most of these terms in action.

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Crate map →](Crate-Map)
