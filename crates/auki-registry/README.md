# auki-registry

Sensor / Clock / Frame identity catalogs and their on-disk IO. Content-addressed: the XXH3-128 hash of the RFC 8785 canonical JSON is the entry's version, and refining an entry is a sibling-write under the same id.

The crate's scope is identity catalogs only — log payload types departed for [`auki-datatypes`](../auki-datatypes) during the 2026-05-08 migration. Spatial sensor bodies (`Camera`, `PointCloud`) pin exact `frame_id` + `frame_hash` references to the Frame Registry.

**Status:** Shipped.

## Public surface

- `SensorRegistryEntry`, `SensorBody` (`Camera` / `PointCloud` / `JointEncoders` / `Audio`)
- `ClockRegistryEntry`
- `FrameRegistryEntry` + preset constructors (`ros_body`, `ros_optical`, `opengl`, `unity`)
- `write_sensor` / `read_sensor`, `write_clock` / `read_clock`, `write_frame` / `read_frame`
- Locked vector pins the canonical-JSON + XXH3-128 chain for `FrameRegistryEntry::ros_body`.

## Depends on

- [`auki-jcs`](../auki-jcs) — for canonicalizing entries before hashing.
- [`auki-hash`](../auki-hash) — for the content hash.
- [`auki-layout`](../auki-layout) — for the on-disk path of each entry.
