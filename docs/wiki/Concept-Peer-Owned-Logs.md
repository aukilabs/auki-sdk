# Concept: Peer-Owned Logs

The Auki SDK rests on one invariant. Internalize this and the rest of the API stops looking arbitrary.

> A peer owns data products.
> A data product has exactly one canonical `peer_id`.
> Materialized copies preserve that `peer_id`.

— from the [#216 design spec](https://github.com/aukilabs/auki-sdk/blob/develop/docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md)

## Why ownership matters

The Auki network is peer-to-peer. No central server holds "the truth" about a robot's camera feed. Instead, each peer holds — and exposes — the data products it produced. When another peer needs that data, it dials the producer and pulls.

That means every data product needs an unambiguous answer to: **whose data is this?** The answer is its canonical `peer_id`. It is set the moment the data product is created and never changes. Even when the data is copied — *materialized* — onto another peer for caching, latency, or longer retention, the canonical `peer_id` of the data stays with the data.

In code, a "data product" is a **log**: a sensor log, pose log, time-transform log, or detection log. See [The Five Questions](The-Five-Questions) for where each fits architecturally.

## Source vs writer — two peer IDs, one log

Every log header — both on disk and in the catalog row — carries two peer identities:

| Field | Meaning |
|-------|---------|
| `source_peer_id` | Canonical origin. The peer whose physical sensor or actuator produced the data. Set once at creation. Preserved across all materializations. |
| `writer_peer_id` | The peer that physically wrote *this* manifest and *these* segment bytes. Equals `source_peer_id` for an origin log. Differs when another peer materialized a copy. |

For an **origin log** — Galbot writing its own camera feed — the two are equal:

```json
{ "source_peer_id": "galbot", "writer_peer_id": "galbot", ... }
```

For a **materialized log** — Park caching Galbot's camera feed with 5-minute local retention — the source stays, the writer changes:

```json
{ "source_peer_id": "galbot", "writer_peer_id": "park", ... }
```

That's it. No third "ownership" field. **Source is ownership.** Writer is the practical detail of who currently holds the bytes you want to read.

## Why two fields, not one

A consumer cares about both questions:

- **"Whose data is this?"** is answered by `source_peer_id`. This is what stays consistent — what a downstream consumer correlates across versions, archives, and replicas.
- **"Who do I dial to fetch bytes?"** is answered by `writer_peer_id`. That changes per replica.

Collapsing them into a single field hides one of the two questions. Keeping them split makes both first-class on every catalog row, every manifest, every disk artifact.

## Materialization preserves identity, not labels

Materialization is when a peer copies a log it does not own. Park materializing Galbot's `head_left_rgb` does not claim authorship — it claims caching:

```text
Galbot's origin log:
  source_peer_id: galbot
  writer_peer_id: galbot

Park's materialized copy:
  source_peer_id: galbot       ← unchanged; points at the origin
  writer_peer_id: park         ← changed; Park's file, Park's bytes
```

The materialized manifest is a *new* file with a new `writer_peer_id`, but `source_peer_id` is unchanged. The catalog row Park advertises for this materialization also reports `source_peer_id: galbot`. A consumer fetching from Park gets bytes-on-Park, but always sees the source identity Galbot started with.

Park chooses its own `retention_ns` and `segment_duration_ns` — those are writer-local properties. But the registry refs inside the manifest (`sensor`, `clock`, `frame`) still point at *Galbot's* canonical registry entries (`peer_id = "galbot"` inside the `RegistryRef`). Consumers resolve sensor intrinsics, clock metadata, and frame conventions against the source's authoritative records, never the materializer's.

## Why no `materialized_by_peer_id`

An early proposal carried `peer_id` (origin) plus an optional `materialized_by_peer_id` on materialized rows only. We rejected it for three reasons:

1. **Two implicit shapes for one concept.** Origin rows and materialized rows would have different field sets. Consumers would branch on `materialized_by_peer_id.is_some()`.
2. **"Who wrote this file?" became optional.** With `writer_peer_id` always present, the question is always answerable from any row, with no `Option` to unwrap.
3. **It biased the schema toward the origin case.** Every additional materialization layer (Park materializes Galbot; some third peer materializes Park) would need another optional field. The source/writer split scales — only two identities ever matter for any given file.

Today's schema collapses both cases under a uniform shape: every row, every manifest, has exactly `source_peer_id` and `writer_peer_id`. Origin = they're equal. Materialized = they differ. No special cases.

## Mapping the concept to code and wire

### Manifest (on disk)

```rust
pub struct SensorLogManifest {
    pub source_peer_id: PeerId,
    pub writer_peer_id: PeerId,
    pub app_id:     String,
    pub session_id: String,
    pub sensor: RegistryRef,
    pub clock:  RegistryRef,
    pub frame:  Option<RegistryRef>,
    pub segment_duration_ns: i64,
    pub retention_ns: i64,
}
```

The split is in `auki_manifests::*Manifest` for all four log variants (`SensorLogManifest`, `PoseLogManifest`, `TimeTransformLogManifest`, `DetectionLogManifest`).

### Catalog row (over `/auki/auth/1/resources/0.2.0`)

```json
{
  "source_peer_id": "galbot",
  "writer_peer_id": "park",
  "resource_id": "head_left_rgb",
  "variant": "sensor_log",
  ...
}
```

Defined as `auki_network::resources_protocol::ResourceEntry`. The full row shape is documented in [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md).

### Stream request (over `/auki/auth/1/stream/0.2.0`)

```rust
pub struct StreamRequest {
    pub source_peer_id: PeerId,    // canonical origin, not "the peer I'm dialing"
    pub resource_id: String,
    pub from: ReadFrom,
}
```

The stream request identifies the *data* by `source_peer_id`. `writer_peer_id` is implicit in the libp2p connection — you're already talking to whoever serves you the file.

### LogRef — the canonical handle

```rust
pub struct LogRef {
    pub source_peer_id: PeerId,
    pub resource_id: String,
}
```

When the SDK refers to "a log" abstractly — a `Session::materialize_remote_log` argument, the input of a detection log, anywhere a log is named without saying *which copy* — it uses `LogRef`, which carries only the canonical identity. The writer is irrelevant when discussing logs as abstract entities; it only matters once you're actually moving bytes.

## What this enables

- **Caching without claiming authorship.** Park serving Galbot's camera feed is a legitimate Auki citizen, not a forger.
- **Cross-session correlation.** A robot reboot resets `session_id`, but `peer_id` is durable per device — sensor data across sessions stays anchored to the same canonical owner.
- **Multi-tier networks.** A peer can materialize from another materializer; the source identity propagates through arbitrarily long replica chains.

## See also

- [Quickstart](Quickstart) — register your first peer-owned sensor log
- [#216 design spec](https://github.com/aukilabs/auki-sdk/blob/develop/docs/superpowers/specs/2026-05-27-216-schema-and-api-placement-design.md) — the canonical record of this design
- [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md) — catalog row reference with worked examples per variant
- [The Five Questions](The-Five-Questions) — where ownership fits in the architectural frame

---

[← Back to: For SDK Consumers](For-SDK-Consumers) · [The Five Questions →](The-Five-Questions)
