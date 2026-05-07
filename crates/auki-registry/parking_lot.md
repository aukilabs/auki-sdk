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

## Atomic-write tmp file cleanup

If a process crashes mid-write, the `.<filename>.tmp` sidecar is left behind. There's no TTL or startup-cleanup pass. In a long-lived session, can these accumulate enough to matter? Should `write_sensor` / `write_clock` opportunistically remove stale tmp files at the start of each call, or run a cleanup pass on log/registry open?
