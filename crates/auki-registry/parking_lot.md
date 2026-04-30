# Parking lot — auki-registry

---

## Frame Registry shape

`FrameRegistryEntry` is listed as "coming" but the schema isn't defined. Fields likely include: handedness (right/left), axes (e.g. forward/up/right convention), units (meters?), rotation semantics (quaternion order; intrinsic vs extrinsic). What's the minimal viable shape, and does it need extension points like the tagged-enum pattern in `SensorRegistryEntry`?

## UTC clock epoch encoding

`ClockMeta.epoch` is `Option<String>`. For monotonic clocks the value is `null`. For UTC clocks the epoch is non-null but the format isn't specified — RFC 3339 (`"1970-01-01T00:00:00Z"`)? Unix seconds (`"0"`)? Free-text? Pin the format before any cross-language reader has to parse it.

## Atomic-write tmp file cleanup

If a process crashes mid-write, the `.<filename>.tmp` sidecar is left behind. There's no TTL or startup-cleanup pass. In a long-lived session, can these accumulate enough to matter? Should `write_sensor` / `write_clock` opportunistically remove stale tmp files at the start of each call, or run a cleanup pass on log/registry open?
