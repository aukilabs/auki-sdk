# auki-logs

Generic segmented ring-buffer log primitive. Two of the SDK's four logs — the Sensor Log and the TimeTransform Log — are typed instantiations of this primitive. The schemas for each live with the crate that owns the entry type ([`auki-registry`](../auki-registry) for `SensorLogEntry`, [`auki-time-transforms`](../auki-time-transforms) for `TimeTransformEntry`).

The rest of this README is the **on-disk format spec, version 1** — implementations in any language must read and write segment files that conform to it.

## Filesystem layout

```
<log_root>/
  manifest.json             RFC 8785 (JCS) canonical bytes
  segments/
    <padded-ns>.seg         one file per segment
```

`<padded-ns>` is the segment-start timestamp in nanoseconds, formatted as a 20-digit zero-padded decimal. Lexicographic order on filenames equals chronological order on segments.

`<log_root>` itself is chosen by the caller. In practice the SDK's session shape places it at:

- `<session>/timetransform_logs/<from_id>__<to_id>/` — one TT log per session
- `<session>/sensorlogs/<recording_uuid>/<sensor_id>/` — one sensor log per recording

See [`auki-session`](../auki-session) for path helpers and the full session shape.

## Manifest

JCS-canonical UTF-8 JSON. The format requires two keys; everything else is the caller's payload.

| Key                    | Type     | Required | Notes                                                  |
| ---------------------- | -------- | -------- | ------------------------------------------------------ |
| `segment_duration_ns`  | integer  | yes      | Must be > 0. Time covered by one segment.              |
| `retention_ns`         | integer  | yes      | Must be ≥ 0. Window of data kept on disk; 0 = unbounded (no eviction). |

A Sensor Log manifest will additionally carry `clock_id`, `sensor_id`, `sensor_hash`, etc. A TimeTransform Log manifest will carry `from_clock_id`, `to_clock_id`, and the corresponding hashes. `auki-logs` does not interpret these — they're caller-defined and persist verbatim.

The manifest is written **once**, when the log directory is first created. Re-opening an existing log uses the on-disk manifest as ground truth; any manifest passed by the caller at re-open time is ignored.

## Segment file

### Header — 16 bytes, little-endian

| Offset | Size | Field      | Notes                                  |
| ------ | ---- | ---------- | -------------------------------------- |
| 0      | 4    | `magic`    | ASCII `"AKLG"` (0x41 0x4B 0x4C 0x47)   |
| 4      | 2    | `version`  | u16; this document specifies version 1 |
| 6      | 2    | `reserved` | u16; MUST be 0                         |
| 8      | 8    | `start_ns` | i64; matches the filename              |

### Entries — repeating until EOF

| Size | Field           | Notes                                                |
| ---- | --------------- | ---------------------------------------------------- |
| 8    | `timestamp_ns`  | i64, little-endian                                   |
| 4    | `payload_len`   | u32, little-endian; length of `payload` in bytes     |
| N    | `payload`       | CBOR (RFC 8949) encoding of the entry payload        |

There is no entry count, no per-entry checksum, and no trailer. Readers parse entries until EOF or a short read.

## Endianness

All multi-byte integers in the segment header and entry framing are little-endian. CBOR is internally big-endian per RFC 8949; the framing wraps the CBOR payload byte-for-byte without disturbing it.

## Crash safety

- The writer flushes and `fsync`s a segment when it rolls over to a new one.
- A crash mid-segment leaves a truncated tail. Readers MUST stop on the first short read past the last fully-decoded entry and treat preceding entries as valid.
- Segment writes use `O_CREAT | O_EXCL` (Rust: `create_new`) so two writers cannot accidentally share a segment file.
- The manifest is written via temp-file + `rename` for atomicity.

## Eviction

Driven by data timestamps, not wall clock. On every `append(timestamp_ns, …)` call, segments whose end (`start_ns + segment_duration_ns`) is `≤ timestamp_ns - retention_ns` become eligible for deletion. The currently-open segment is never evicted, even if its window has aged out.

When `retention_ns == 0`, eviction is disabled entirely: every segment is kept for the lifetime of the log. Use this for unbounded captures.

## Versioning

Format version is **1**. Bump for any incompatible change to the header layout, framing, or payload encoding (e.g. CBOR → another scheme). Readers MUST reject unknown versions.

## Why CBOR

Segment files are durable artifacts and the SDK targets multiple platforms over its long arc. CBOR (RFC 8949) has decoders in every major language, so future iOS/glasses/web/analysis tooling can read these files without pulling in a Rust runtime. The throughput cost vs. bincode is negligible at the cadences this format is designed for. Revisit if profiling shows CBOR overhead matters.
