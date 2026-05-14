# Parking lot — auki-registry

---

## Log-payload sections in the README during migration

The README currently documents four log payload types (`SensorLogEntry`, `PointCloudLogEntry`, `AudioLogEntry`, `PoseLogEntry`) in detail, all of which are migrating to [`auki-datatypes`](../auki-datatypes) per [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md). Each section now carries a "Departing" callout pointing at the migration step. Open question: should the per-type details (manifest tables, CBOR payload shapes, design rationale) **stay here** until each type physically moves — keeping the README the single source of truth during transition — or **move to `auki-datatypes`'s README in a stub form now** so consumers stop reading two places?

Lean: stay here until physically moved. The detailed shapes (CBOR encoding, `DynamicIntrinsics` placement, RGB(A) normalization, `PoseSource` design rationale) describe what's on disk **today** under `auki-registry`; moving the docs ahead of the code would create the opposite drift problem. As each migration step lands, that section moves to `auki-datatypes` as part of the same PR — sequenced doc move with code move.

Trigger to revisit: if any external reader gets confused by the layout. None reported yet.

## UTC clock epoch encoding

`ClockMeta.epoch` is `Option<String>`. For monotonic clocks the value is `null`. For UTC clocks the epoch is non-null but the format isn't specified — RFC 3339 (`"1970-01-01T00:00:00Z"`)? Unix seconds (`"0"`)? Free-text? Pin the format before any cross-language reader has to parse it.

## Formalize the sensor_id naming convention?

The README documents `<platform-tag>-<machine-id>/<sensor-name>` as a *recommended* (non-enforced) pattern for sensor and clock IDs. Boosterapp uses this shape (e.g. `K1-AABBCCDDEEFF/head_rgb`); we expect future integrators to follow it for cross-app readability.

Open question: should the SDK formalize this — e.g. provide a `SensorId` newtype with `parse`/`format` methods, or a tiny `make_sensor_id(platform, machine_id, sensor_name)` helper — or stay out of string-building entirely and rely on the documented convention? Trade-off is enforcement and parseability vs. SDK surface area.

**2026-05-08 update — the convention is now load-bearing for cross-peer recording provenance.** Per the [root subscription-as-materialization decision](../../parking_lot.md#subscription-as-materialization-the-unified-detector-ingestion-architecture-filed-by-dobby-2026-05-08), a recording stays self-provenant only because `sensor_id`'s device-encoding prefix (e.g. `K1-AABBCCDDEEFF/...` where the MAC names the producing device) survives subscription, archival, and replay. An integrator who names their sensor `my-cool-camera` produces a recording that no longer carries any provenance signal — and the architecture has no way to catch it. This raises the urgency of formalizing the convention. Lean: keep the SDK out of string-building (no `SensorId` newtype, no enforcement) but **harden the README's status from "recommended" to "REQUIRED for cross-peer recording provenance,"** with a worked example showing how an unprefixed ID degrades the unified-ingestion case. Pin before the unified subscription primitive lands.

## Atomic-write tmp file cleanup

If a process crashes mid-write, the `.<filename>.tmp` sidecar is left behind. There's no TTL or startup-cleanup pass. In a long-lived session, can these accumulate enough to matter? Should `write_sensor` / `write_clock` opportunistically remove stale tmp files at the start of each call, or run a cleanup pass on log/registry open?

---

## JointEncoders sensor body — decisions filed at landing

### Decided 2026-05-09 — `joint_names` placement on the producer

Decided: not on the registry entry. URDF lives with the consumer (Park, future analyses). Reason: the producer doesn't read URDF; making it declare names asks it to be authoritative for a schema it doesn't own. Joint ordering is a producer-defined invariant per log, agreed by hand-coordination at integration time. Revisit when ≥2 robot models share a Park instance — at that point either `urdf_id` for explicit coupling or `joint_name_hash` for opaque sanity-check.

### Decided 2026-05-09 — `SensorBody::JointEncoders` minimalism (`joint_count` only)

Decided: no `joint_name_hash`, no `urdf_id`, no per-joint metadata. `joint_count` is the deserialization invariant (matches `Audio::channels`); anything richer is interpretation, not deserialization. Revisit when a real cross-robot mismatch shows up that `joint_count` alone doesn't catch.
