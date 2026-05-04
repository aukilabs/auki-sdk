# `auki-registry/src/`

Sensor + Clock Registry entries with content-addressed multi-version-by-hash on-disk storage.

## What's here

A single source file: [`lib.rs`](lib.rs).

## Storage layout

```
<app_root>/registries/sensors/<id-with-slashes-replaced-by-__>/<hash>.json
<app_root>/registries/clocks/<id-with-slashes-replaced-by-__>/<hash>.json
```

Paths come from [`auki-session`](../../auki-session) (`sensor_entry_path` / `clock_entry_path`) — this crate doesn't compute them itself. The `app_root` argument to `write_*` / `read_*` is the integrator's app root, shared across all sessions of that app.

The hash *is* the version. There are no version counters. Re-writing identical content is a no-op (`WriteOutcome::AlreadyExists`); writing different content under the same `id` produces a sibling file (`WriteOutcome::Created` with a different hash).

`/` in IDs is replaced with `__` so namespaced IDs like `K1-AABBCCDDEEFF/head_left_cam` become a single filesystem-safe directory segment.

## Entry types

### `SensorRegistryEntry`

```rust
pub struct SensorRegistryEntry {
    pub sensor_id: String,
    #[serde(flatten)]
    pub body: SensorBody,
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum SensorBody {
    RgbCamera(RgbCamera),     // serializes as "type":"rgb_camera"
}

pub struct RgbCamera {
    pub width: u32,
    pub height: u32,
    pub frame_rate_hz: u32,
    pub pixel_format: String,
    pub color_space: String,
    pub intrinsics_model: String,
    pub distortion_model: String,
}
```

The tagged-enum body shape is the extension point for future sensor types (depth, IMU, lidar, etc.) — each gets its own variant + struct under the same envelope.

### `ClockRegistryEntry`

```rust
pub struct ClockRegistryEntry {
    pub clock_id: String,
    #[serde(flatten)]
    pub body: ClockBody,
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClockBody {
    MonotonicClock(ClockMeta),
    UtcClock(ClockMeta),
}

pub struct ClockMeta {
    pub unit: String,
    pub monotonic: bool,
    pub epoch: Option<String>,    // null is meaningful (monotonic clocks have no epoch)
    pub scope: Scope,
}

#[serde(rename_all = "kebab-case")]
pub enum Scope {
    DeviceLocal,    // "device-local"
    DomainLocal,    // "domain-local"
    Global,         // "global"
}
```

`epoch` is intentionally `Option<String>` *without* `skip_serializing_if`. Monotonic clocks must serialize as `"epoch":null` because the absence of an epoch is meaningful information.

### `SensorBody::PointCloud`

```rust
pub struct PointCloud {
    pub fields: Vec<PointField>,
    pub point_step: u32,
    pub is_bigendian: bool,
    pub frame_rate_hz: u32,
}

pub struct PointField {
    pub name: String,
    pub offset: u32,
    pub datatype: PointFieldDataType,
    pub count: u32,
}

#[serde(rename_all = "snake_case")]
pub enum PointFieldDataType {
    Int8, Uint8, Int16, Uint16, Int32, Uint32, Float32, Float64,
}
```

`PointFieldDataType::byte_width()` returns the per-element width in bytes (1, 2, 4, or 8). Used by translation code (e.g. `auki-ros-adapter`) to compute output `point_step` after RGB normalization.

### `SensorBody::Microphone`

```rust
pub struct Microphone {
    pub sample_rate_hz: u32,        // e.g. 48000
    pub channels: u32,              // 1 mono, 2 stereo, N for arrays
    pub sample_format: String,      // "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le"
    pub channel_layout: String,     // "mono" | "stereo" | "5.1" | "7.1" | "ambisonic_b" | "n_channel"
}
```

Multi-mic arrays are one sensor with `channels = N` (not N independent sensors). Compressed `sample_format` values (`flac`, `opus`, ...) get added when those are needed; the struct shape doesn't change.

## Log payload types

```rust
pub struct SensorLogEntry {
    pub dynamic_intrinsics: DynamicIntrinsics,
    #[serde(with = "serde_bytes")]
    pub frame: Vec<u8>,
}

pub struct PointCloudLogEntry {
    pub width: u32,        // organized: cols; unorganized: total point count
    pub height: u32,       // organized: rows; unorganized: 1
    pub is_dense: bool,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

pub struct AudioLogEntry {
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,     // interleaved samples per the registry's sample_format and channels
}
```

All three byte buffers are tagged `#[serde(with = "serde_bytes")]` so CBOR encodes them as byte strings (major type 2) rather than arrays of u8 — same on-disk semantics, ~half the byte cost on typical payloads.

## Public functions

```rust
pub fn write_sensor(app_root: &Path, entry: &SensorRegistryEntry) -> Result<WriteOutcome>;
pub fn write_clock(app_root: &Path,  entry: &ClockRegistryEntry)  -> Result<WriteOutcome>;
pub fn read_sensor(app_root: &Path, sensor_id: &str, hash: &str) -> Result<Option<SensorRegistryEntry>>;
pub fn read_clock(app_root: &Path,  clock_id: &str,  hash: &str) -> Result<Option<ClockRegistryEntry>>;

pub fn build_sensor_log_manifest(
    app_id: &str,
    session_id: &str,
    sensor_id: &str,
    sensor_hash: &str,
    clock_id: &str,
    clock_hash: &str,
    segment_duration: Duration,
    retention: Duration,
) -> serde_json::Value;
```

Both entry types also expose `canonical_bytes()` and `hash()` directly for callers that want to compute identity without writing.

`build_sensor_log_manifest` produces a `serde_json::Value` containing all eight required Sensor Log family manifest fields (the run-identifying `app_id` / `session_id`, the sensor and clock bindings, and `auki-logs`'s required `segment_duration_ns` / `retention_ns`). Same shape for Sensor Log, Point Cloud Log, and Audio Log — the `(sensor_id, sensor_hash)` pair resolves to a `SensorRegistryEntry` whose `body` variant tells a reader which payload type the segments hold.

## `WriteOutcome`

```rust
pub enum WriteOutcome {
    Created(String),       // hash; file did not exist, was just written
    AlreadyExists(String), // hash; file already existed, no-op
}
```

Both variants carry the hash, accessible via `.hash()`. Callers only care which file ended up authoritative; the discriminant is informational.

## Errors

```rust
pub enum Error {
    Io(io::Error),
    Json(String),
    IdMismatch { expected: String, found: String },
}
```

`IdMismatch` fires on read when the on-disk file's `sensor_id` / `clock_id` doesn't match the requested ID. This catches misplaced or tampered files — content addressing is meant to make tampering detectable.

## Atomic writes

Writes go to `.<filename>.tmp` first, fsync, then rename. A crash mid-write leaves either nothing or the complete file; never a half-written one.

## Tests (23 total)

| Test | Asserts |
|------|---------|
| `sensor_entry_serializes_to_canonical_bytes_matching_m1_example` | Byte-exact JCS output for the M1 example sensor entry |
| `monotonic_clock_canonical_bytes_match_m1_example` | Same, monotonic clock |
| `utc_clock_canonical_bytes_match_m1_example` | Same, UTC clock |
| `sensor_entry_hash_is_locked` | `e8cb3879fcfa7f716047aa0892b0c0c0` |
| `monotonic_clock_hash_is_locked` | `1f2176888b1a6621315033f22659b9f3` |
| `utc_clock_hash_is_locked` | `89f84f4c2e09bef81d385b2af1d17e6c` |
| `write_then_read_sensor_round_trip` | Write produces a file readable as the same value |
| `write_then_read_clock_round_trip` | Same, clock side |
| `multi_version_same_content_is_no_op` | Writing the same entry twice produces one file |
| `multi_version_different_content_writes_alongside` | Mutating a field produces a sibling file at a new hash |
| `slash_in_id_becomes_double_underscore` | Path encoding produces a flat dir, not nested |
| `read_missing_returns_none` | Absent files are `Ok(None)`, not an error |
| `read_with_id_mismatch_errors` | Misplaced files surface as `IdMismatch` |
| `write_outcome_hash_accessor` | `.hash()` works on both variants |
| `build_sensor_log_manifest_contains_all_required_fields` | Builder produces all 8 manifest fields with correct types |
| `sensor_log_manifest_opens_a_log_round_trip` | Manifest round-trips through `auki_logs::Log::open` + `read` (integration; uses dev-dep on `auki-logs`) |

The locked hashes serve as cross-cutting regression guards: if any of `auki-jcs`, `auki-hash`, or this crate's serde shape drifts, three tests fail at once.

## Consumers in this workspace

- `auki-k1-binary` — writes one Sensor entry + two Clock entries at startup
- `auki-renderer` — reads the Sensor entry to recover pixel format / dimensions / color space
- `auki-ros-adapter` — re-exports this crate; `build_rgb_camera_registry_entry` constructs entries
