# auki-logs

Generic segmented ring-buffer log primitive. Two of the SDK's four logs — the Sensor Log and the TimeTransform Log — are typed instantiations of this primitive. The schemas for each live with the crate that owns the entry type ([`auki-datatypes`](../auki-datatypes) for `PinholeCameraLogEntry` since 2026-05-08, [`auki-time-transforms`](../auki-time-transforms) for `TimeTransformEntry` until Step 6 of the [migration](../auki-datatypes/src/sprint.md)).

Payload encoding is the consumer's choice via the [`LogPayload`](src/lib.rs) trait — this crate handles framing only. Prost types in [`auki-datatypes`](../auki-datatypes) get a blanket impl through the `impl_log_payload!` macro; mid-migration CBOR types implement it directly.

The rest of this README is the **on-disk format spec, version 1** — implementations in any language must read and write segment files that conform to it.

## Filesystem layout

```
<log_root>/
  manifest.json             RFC 8785 (JCS) canonical bytes
  tags.jsonl                append-only TagClaim sidecar; optional, see ../../tags.md
  segments/
    <padded-ns>.seg         one file per segment
```

`<padded-ns>` is the segment-start timestamp in nanoseconds, formatted as a 20-digit zero-padded decimal. Lexicographic order on filenames equals chronological order on segments.

`tags.jsonl` is a reserved filename for the [`TagClaim`](../../tags.md) sidecar. The auki-logs writer does not produce or consume it (TagClaim handling lives outside this crate); the file is documented here so any tooling that enumerates a log directory accounts for it. Absent until something writes one.

`<log_root>` itself is chosen by the caller. In practice the SDK's session shape places it at:

- `<session>/timetransform_logs/<from_id>__<to_id>/` — one TT log per session
- `<session>/sensorlogs/<sensor_log_id>/` — one log per sensor stream
- `<session>/poselogs/<pose_log_id>/` — one log per pose source

See [`auki-layout`](../auki-layout) for path helpers and the full session shape.

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
| N    | `payload`       | Consumer-defined bytes per the `LogPayload` impl     |

There is no entry count, no per-entry checksum, and no trailer. Readers parse entries until EOF or a short read.

## Endianness

All multi-byte integers in the segment header and entry framing are little-endian. The payload bytes are opaque to the framing and pass through byte-for-byte; their internal endianness is the encoder's concern.

## Crash safety

- The writer flushes and `fsync`s a segment when it rolls over to a new one.
- A crash mid-segment leaves a truncated tail. Readers MUST stop on the first short read past the last fully-decoded entry and treat preceding entries as valid.
- Segment writes use `O_CREAT | O_EXCL` (Rust: `create_new`) so two writers cannot accidentally share a segment file.
- The manifest is written via temp-file + `rename` for atomicity.

## Tailing

A reader can also follow a log live: [`Log::tail`](src/lib.rs) returns a `TailIter<T>` that yields newly-appended entries as they become readable. The iterator starts at the **current EOF** of the log — entries on disk before `tail()` was called are not replayed (use `read().entries()` for historical). It polls the segments directory at a configurable cadence (default 10ms); each `Iterator::next` call blocks until a new entry is readable. Drop the iterator to stop tailing.

This is the read side of the [subscription-as-materialization keystone](../../parking_lot.md): the same `Log<T>::tail` call works whether the bytes were captured here, materialized from a peer's stream, or opened from a recording on disk. The transport differs (zero-hop, libp2p, file source); the tail call doesn't.

`TailIter::try_next` is the non-blocking variant — `Ok(Some(entry))` if one is ready, `Ok(None)` if not, `Err(_)` only on real I/O or payload decode failure. Mid-write torn reads (timestamp + length + payload are three separate writes) surface as `Ok(None)`, not `Err`; the next poll picks up the entry once the writer flushes.

## Eviction

Driven by data timestamps, not wall clock. On every `append(timestamp_ns, …)` call, segments whose end (`start_ns + segment_duration_ns`) is `≤ timestamp_ns - retention_ns` become eligible for deletion. The currently-open segment is never evicted, even if its window has aged out.

When `retention_ns == 0`, eviction is disabled entirely: every segment is kept for the lifetime of the log. Use this for unbounded captures.

### Runtime mutability

A running log's `retention_ns` can be changed via `Log::set_retention(new_value)` without closing the log. The implementation rewrites `manifest.json` atomically (`.tmp` → fsync → rename) so the change survives daemon restart, then updates the in-memory state. Disk-first ordering means a failed write leaves the log unchanged. Eviction itself runs only as part of `append`, so a quiescent log retains its current segments until something appends after the change. The use case is operator-driven endpoints — `PATCH /api/buffer` in the [Control API spec](../../docs/control-api.md) — that extend (or shrink) a recording's retention while it's running, without forcing a close-and-reopen cycle that would drop streaming data during the window. The application owns the policy decision; the SDK exposes the mechanism.

## Versioning

Format version is **1**. Bump for any incompatible change to the header layout, framing, or payload encoding (e.g. CBOR → another scheme). Readers MUST reject unknown versions.

## Why encoding-agnostic

Earlier drafts pinned the payload encoding to CBOR via ciborium. The migration in [`auki-datatypes/src/sprint.md`](../auki-datatypes/src/sprint.md) replaces it with prost (protobuf) for cross-language schema enforcement. Rather than swap one hardcoded encoder for another, the crate exposes a [`LogPayload`](src/lib.rs) trait — `encode(&self) -> Vec<u8>` and `decode(&[u8]) -> Result<Self, String>` — and stays out of the encoder's way. Consumers pick the encoder; the framing primitive doesn't care. Decision pinned 2026-05-08 (Step 1 of the migration); see [`parking_lot.md`](parking_lot.md).
