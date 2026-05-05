# `auki-logs/src/`

Generic segmented ring-buffer log primitive. On-disk format spec: this crate's [outer `README.md`](../README.md).

## What's here

A single source file: [`lib.rs`](lib.rs).

## On-disk layout

```
<root>/
├── manifest.json    ← user-supplied JSON, written JCS-canonical via auki-jcs
└── segments/
    ├── 00000000001234567890.seg
    ├── 00000000002000000000.seg
    └── ...
```

Filenames are zero-padded 20-digit `start_ns` timestamps so lexicographic sort = chronological sort.

## Segment file binary format

```
offset  size  field
─────────────────────
0       4     "AKLG"      magic
4       2     version     u16 little-endian (current: 1)
6       2     reserved    u16 little-endian (current: 0)
8       8     start_ns    i64 little-endian (also encoded in filename)
16+     ...   entries     repeated until EOF
```

Each entry:

```
offset  size  field
─────────────────────
0       8     timestamp_ns    i64 little-endian
8       4     payload_len     u32 little-endian
12      N     payload         CBOR-encoded T (length = payload_len)
```

The header carries no entry count — readers iterate until `UnexpectedEof`. This makes truncated tails (from a crash mid-write) tolerable: the truncated trailing entry is skipped silently and earlier entries are returned cleanly.

## Public API

```rust
pub struct Log<T> { ... }

impl<T: Serialize + DeserializeOwned> Log<T> {
    pub fn open(root: &Path, manifest: serde_json::Value) -> Result<Self>;
    pub fn append(&mut self, timestamp_ns: i64, payload: &T) -> Result<()>;
    pub fn read(root: &Path) -> Result<LogReader<T>>;
}

impl<T> Log<T> {
    pub fn manifest(&self) -> &serde_json::Value;
    pub fn flush(&mut self) -> Result<()>;
    pub fn set_retention(&mut self, retention_ns: i64) -> Result<()>;
}

pub struct LogReader<T> { ... }
impl<T: DeserializeOwned> LogReader<T> {
    pub fn manifest(&self) -> &serde_json::Value;
    pub fn segment_starts(&self) -> &[i64];
    pub fn entries(&self) -> Result<Vec<Entry<T>>>;
}

pub struct Entry<T> {
    pub timestamp_ns: i64,
    pub payload: T,
}
```

## Manifest contract

The user's manifest JSON **must** include two integer fields:

- `segment_duration_ns` — segment file rollover boundary in ns
- `retention_ns` — how far back from the most recent timestamp to keep

Both must be > 0. Other fields are user-defined; the log preserves them verbatim.

If `manifest.json` already exists at `root`, it is the source of truth and the `manifest` argument to `open()` is ignored. This means re-opening a log with a different manifest doesn't break the on-disk state.

## Behavior

**Append.**
- Negative `timestamp_ns` → `Error::Format`.
- If the timestamp is outside the current segment's window (or no segment is open), the current segment is closed (flush + fsync) and a new segment is started, named after the segment-aligned start time.
- Entry is written to the current segment.
- After every successful append, segments fully outside the retention window from the *appended* timestamp are evicted (`unlink`). The current segment is never evicted, even if it falls outside.

**Read.** Walks every `.seg` file in the segments dir in chronological order, reads each entry until EOF, returns them all. Truncated tails are silently tolerated.

**`set_retention`.** Updates the in-memory `retention_ns` and rewrites `manifest.json` atomically (`.tmp` → fsync → rename) so the change survives daemon restart. Affects future appends only — eviction runs as part of `append`, not as part of `set_retention` itself, so a quiescent log retains its current segments until something appends. Disk-write-first ordering: the manifest is persisted before the in-memory field is updated, so a failed write leaves the log unchanged. Use case: operator-driven endpoints like Sentinel's `PATCH /api/buffer` that extend (or shrink) a recording's retention while it's running, without forcing a close-and-reopen cycle that would drop streaming data during the window.

**Drop.** `Log<T>::drop` best-effort-closes the current segment so the SDK doesn't depend on explicit `flush()` calls for crash safety.

## Errors

```rust
pub enum Error {
    Io(io::Error),
    Cbor(String),       // CBOR encode/decode failure
    Manifest(String),   // missing or malformed manifest fields
    Format(String),     // timestamps, payload size limits, segment header issues
}
```

## Why CBOR for payloads

Chosen jointly with Nils after weighing throughput vs cross-language readability. CBOR (via `ciborium`) is binary-efficient like bincode but has a stable inter-language ecosystem — important because the SDK's design ambition includes future iOS/phone/browser composition where readers may not be Rust. Revisit if profiling shows overhead at scale.

## Tests (14 total)

| Test | Asserts |
|------|---------|
| `open_creates_layout_and_writes_manifest` | First open produces `manifest.json` (JCS-canonical) + `segments/` dir |
| `round_trip_single_segment` | Two appends in the same segment round-trip cleanly through read |
| `rolls_over_at_segment_boundary` | Crossing `segment_duration_ns` triggers a new segment file |
| `evicts_segments_older_than_retention` | After 10s of appends with 3s retention, only the last 4 segments remain |
| `retention_zero_disables_eviction` | `retention_ns = 0` keeps every segment for the lifetime of the log |
| `segment_file_header_is_well_formed` | Magic/version/reserved/start_ns bytes match the spec |
| `reopen_preserves_existing_manifest_and_appends_new_segments` | On-disk manifest wins over the new arg; new segments append correctly |
| `truncated_segment_tail_is_tolerated_on_read` | Garbage at end-of-segment doesn't break the read |
| `manifest_missing_required_field_errors` / `manifest_zero_duration_errors` | Manifest validation |
| `set_retention_shrinks_window_for_subsequent_appends` | Shrinking retention during a run takes effect on the next `append` |
| `set_retention_persists_across_reopen` | Manifest rewrite survives drop + reopen; persisted value drives eviction |
| `set_retention_rejects_negative` | Negative argument → `Error::Manifest`; in-memory state unchanged |
| `set_retention_zero_disables_future_eviction` | Switching from a tight retention to `0` mid-run stops further eviction |

## Consumers in this workspace

- `auki-time-transforms` — `Log<TimeTransformEntry>` for the 1 Hz sampler
- `auki-ros-adapter` — `Log<SensorLogEntry>` for the ring-buffered camera frame log
- `auki-renderer` — read-only consumer for the sensor log
