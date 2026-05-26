//! Generic segmented ring-buffer log primitive.
//!
//! On-disk format spec: [`../README.md`](../README.md).
//!
//! A `Log<T>` writes entries `(timestamp_ns, T)` to time-bounded segment files
//! under `<root>/segments/`. Segments roll over when an appended entry's
//! timestamp leaves the current segment's window. Segments outside the
//! retention window are evicted on append.
//!
//! Payload encoding is the consumer's choice via the [`LogPayload`] trait —
//! this crate handles framing only. See the trait docs for how to wire up a
//! prost / serde / hand-rolled encoder.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const MAGIC: &[u8; 4] = b"AKLG";
const VERSION: u16 = 1;
const HEADER_SIZE: usize = 16;
const SEGMENT_EXT: &str = "seg";
const FILENAME_DIGITS: usize = 20;
pub const LOG_MANIFEST_FILE: &str = "log_manifest.json";

/// Encoder-agnostic payload contract. The log primitive handles framing and
/// segment rollover; the consumer picks the payload encoding by implementing
/// this trait for their `T`. Generated payload types defined in
/// [`auki-proto`](../../auki-proto) use prost.
pub trait LogPayload: Sized {
    fn encode(&self) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> std::result::Result<Self, String>;
}

/// Opaque-byte payload used by generated bindings and by callers that already
/// own their payload encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytesPayload {
    pub bytes: Vec<u8>,
}

impl LogPayload for BytesPayload {
    fn encode(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn decode(bytes: &[u8]) -> std::result::Result<Self, String> {
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// Payload encode/decode failed. The string is the underlying error
    /// formatted by the encoder — prost decode errors, ciborium errors, etc.
    Payload(String),
    Manifest(String),
    Format(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Payload(s) => write!(f, "payload: {s}"),
            Error::Manifest(s) => write!(f, "manifest: {s}"),
            Error::Format(s) => write!(f, "format: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Entry<T> {
    pub timestamp_ns: i64,
    pub payload: T,
}

/// Parse and JCS-canonicalize a log manifest JSON string.
pub fn canonical_manifest_json_str(manifest_json: &str) -> Result<String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| Error::Manifest(format!("parsing manifest JSON: {e}")))?;
    let bytes = auki_jcs::canonicalize(&manifest);
    Ok(String::from_utf8(bytes).expect("JCS output is valid UTF-8"))
}

/// Encode opaque-byte entries into one in-memory v1 segment file.
pub fn encode_segment_bytes(start_ns: i64, entries: &[Entry<BytesPayload>]) -> Result<Vec<u8>> {
    if start_ns < 0 {
        return Err(Error::Format("start_ns must be ≥ 0".into()));
    }

    let mut bytes = Vec::new();
    bytes.write_all(MAGIC)?;
    bytes.write_all(&VERSION.to_le_bytes())?;
    bytes.write_all(&0u16.to_le_bytes())?;
    bytes.write_all(&start_ns.to_le_bytes())?;

    for entry in entries {
        if entry.timestamp_ns < 0 {
            return Err(Error::Format("timestamp_ns must be ≥ 0".into()));
        }
        let payload_len: u32 = entry
            .payload
            .bytes
            .len()
            .try_into()
            .map_err(|_| Error::Format("payload exceeds u32::MAX bytes".into()))?;
        bytes.write_all(&entry.timestamp_ns.to_le_bytes())?;
        bytes.write_all(&payload_len.to_le_bytes())?;
        bytes.write_all(&entry.payload.bytes)?;
    }

    Ok(bytes)
}

/// Decode opaque-byte entries from one in-memory v1 segment file.
pub fn decode_segment_bytes(bytes: &[u8]) -> Result<Vec<Entry<BytesPayload>>> {
    let mut out = Vec::new();
    let mut reader = bytes;
    read_segment_entries_from_reader(&mut reader, "<memory>", &mut out)?;
    Ok(out)
}

/// Convert opaque-byte entries to canonical JSON with hex-encoded payloads.
pub fn bytes_entries_to_json(entries: &[Entry<BytesPayload>]) -> String {
    let values = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "timestamp_ns": entry.timestamp_ns,
                "payload_hex": hex_encode(&entry.payload.bytes),
            })
        })
        .collect::<Vec<_>>();
    String::from_utf8(auki_jcs::canonicalize(&serde_json::Value::Array(values)))
        .expect("JCS output is valid UTF-8")
}

/// Parse canonical segment-entry JSON used by generated bindings.
pub fn bytes_entries_from_json(entries_json: &str) -> Result<Vec<Entry<BytesPayload>>> {
    let value: serde_json::Value = serde_json::from_str(entries_json)
        .map_err(|e| Error::Format(format!("parsing entries JSON: {e}")))?;
    let entries = value
        .as_array()
        .ok_or_else(|| Error::Format("entries JSON must be an array".into()))?;
    entries
        .iter()
        .map(|entry| {
            let timestamp_ns = entry
                .get("timestamp_ns")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Format("entry missing integer timestamp_ns".into()))?;
            let payload_hex = entry
                .get("payload_hex")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Format("entry missing string payload_hex".into()))?;
            Ok(Entry {
                timestamp_ns,
                payload: BytesPayload {
                    bytes: hex_decode(payload_hex)?,
                },
            })
        })
        .collect()
}

pub struct Log<T> {
    root: PathBuf,
    manifest: serde_json::Value,
    segment_duration_ns: i64,
    retention_ns: i64,
    segment_starts: BTreeSet<i64>,
    current: Option<CurrentSegment>,
    _phantom: PhantomData<fn(T)>,
}

struct CurrentSegment {
    start_ns: i64,
    end_ns: i64,
    writer: BufWriter<File>,
}

impl<T> Log<T> {
    fn close_current(&mut self) -> Result<()> {
        if let Some(cur) = self.current.take() {
            let mut writer = cur.writer;
            writer.flush()?;
            let file = writer
                .into_inner()
                .map_err(|e| Error::Io(io::Error::other(e.to_string())))?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn start_segment(&mut self, start_ns: i64) -> Result<()> {
        let path = self.root.join("segments").join(segment_filename(start_ns));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&0u16.to_le_bytes())?;
        file.write_all(&start_ns.to_le_bytes())?;
        let writer = BufWriter::new(file);
        let end_ns = start_ns.saturating_add(self.segment_duration_ns);
        self.current = Some(CurrentSegment {
            start_ns,
            end_ns,
            writer,
        });
        self.segment_starts.insert(start_ns);
        Ok(())
    }

    fn evict_older_than(&mut self, threshold_ns: i64) -> Result<()> {
        let current_start = self.current.as_ref().map(|c| c.start_ns);
        let to_remove: Vec<i64> = self
            .segment_starts
            .iter()
            .copied()
            .filter(|&start| {
                let end = start.saturating_add(self.segment_duration_ns);
                end <= threshold_ns && Some(start) != current_start
            })
            .collect();
        for start in to_remove {
            let path = self.root.join("segments").join(segment_filename(start));
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Io(e)),
            }
            self.segment_starts.remove(&start);
        }
        Ok(())
    }

    pub fn manifest(&self) -> &serde_json::Value {
        &self.manifest
    }

    /// Flush and fsync the current segment without closing the log.
    pub fn flush(&mut self) -> Result<()> {
        if let Some(cur) = self.current.as_mut() {
            cur.writer.flush()?;
            cur.writer.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Update this log's retention window. Affects future appends —
    /// the next call to [`append`][Self::append] evicts any segment
    /// fully outside the new window. Persists to [`LOG_MANIFEST_FILE`]
    /// atomically so the change survives daemon restart.
    ///
    /// Use case: an operator-driven endpoint like `PATCH /api/buffer`
    /// that lets a daemon's running ring-buffer extend (or shrink)
    /// while it's recording, without forcing a close-and-reopen cycle
    /// that would drop streaming data during the window.
    ///
    /// `retention_ns` must be `≥ 0`. Zero disables eviction (matching
    /// the [`open`][Self::open] semantics).
    ///
    /// **Note:** setting retention does not retroactively trigger
    /// eviction. Eviction runs as part of `append`. To force immediate
    /// effect on a quiescent log, the caller can `flush()` and then
    /// drive any subsequent `append`.
    ///
    /// **Failure semantics:** the on-disk manifest is rewritten
    /// *before* the in-memory `retention_ns` field is updated. If the
    /// disk write fails, the log is left unchanged (in-memory state
    /// stays consistent with the on-disk source of truth).
    pub fn set_retention(&mut self, retention_ns: i64) -> Result<()> {
        if retention_ns < 0 {
            return Err(Error::Manifest("retention_ns must be ≥ 0".into()));
        }

        let mut new_manifest = self.manifest.clone();
        match new_manifest {
            serde_json::Value::Object(ref mut map) => {
                map.insert(
                    "retention_ns".into(),
                    serde_json::Value::Number(retention_ns.into()),
                );
            }
            _ => {
                return Err(Error::Manifest("manifest is not a JSON object".into()));
            }
        }

        let bytes = auki_jcs::canonicalize(&new_manifest);
        atomic_write(&self.root.join(LOG_MANIFEST_FILE), &bytes)?;

        self.manifest = new_manifest;
        self.retention_ns = retention_ns;
        Ok(())
    }
}

impl<T> Log<T>
where
    T: LogPayload,
{
    /// Open or create a log directory at `root`. If [`LOG_MANIFEST_FILE`] is missing,
    /// `manifest` is canonicalized (RFC 8785) and written. If present, the
    /// on-disk manifest is the source of truth and `manifest` is ignored.
    pub fn open(root: &Path, manifest: serde_json::Value) -> Result<Self> {
        fs::create_dir_all(root)?;
        fs::create_dir_all(root.join("segments"))?;

        let manifest_path = root.join(LOG_MANIFEST_FILE);
        let manifest = if manifest_path.exists() {
            let bytes = fs::read(&manifest_path)?;
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|e| Error::Manifest(format!("parsing existing manifest: {e}")))?
        } else {
            let bytes = auki_jcs::canonicalize(&manifest);
            atomic_write(&manifest_path, &bytes)?;
            manifest
        };

        let (segment_duration_ns, retention_ns) = required_durations(&manifest)?;
        let segment_starts = list_segments(&root.join("segments"))?;

        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            segment_duration_ns,
            retention_ns,
            segment_starts,
            current: None,
            _phantom: PhantomData,
        })
    }

    /// Append an entry. Rolls the segment over when `timestamp_ns` leaves the
    /// current segment's window, and evicts segments fully outside retention.
    pub fn append(&mut self, timestamp_ns: i64, payload: &T) -> Result<()> {
        if timestamp_ns < 0 {
            return Err(Error::Format("timestamp_ns must be ≥ 0".into()));
        }

        let needs_new = match &self.current {
            None => true,
            Some(cur) => timestamp_ns >= cur.end_ns || timestamp_ns < cur.start_ns,
        };
        if needs_new {
            self.close_current()?;
            let start_ns = (timestamp_ns / self.segment_duration_ns) * self.segment_duration_ns;
            self.start_segment(start_ns)?;
        }

        let payload_bytes = payload.encode();
        let payload_len: u32 = payload_bytes
            .len()
            .try_into()
            .map_err(|_| Error::Format("payload exceeds u32::MAX bytes".into()))?;

        let cur = self.current.as_mut().expect("current segment set above");
        cur.writer.write_all(&timestamp_ns.to_le_bytes())?;
        cur.writer.write_all(&payload_len.to_le_bytes())?;
        cur.writer.write_all(&payload_bytes)?;

        if self.retention_ns > 0 {
            self.evict_older_than(timestamp_ns.saturating_sub(self.retention_ns))?;
        }
        Ok(())
    }

    /// Read manifest + every entry across every segment in chronological order.
    pub fn read(root: &Path) -> Result<LogReader<T>> {
        let manifest_bytes = fs::read(root.join(LOG_MANIFEST_FILE))?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| Error::Manifest(format!("parsing manifest: {e}")))?;
        let segment_starts: Vec<i64> = list_segments(&root.join("segments"))?.into_iter().collect();
        Ok(LogReader {
            root: root.to_path_buf(),
            manifest,
            segment_starts,
            _phantom: PhantomData,
        })
    }

    /// Tail a log directory: yield newly-appended entries as they
    /// become readable. The iterator starts at the **current end** of
    /// the log — entries already on disk are not replayed (use
    /// [`read`][Log::read] + [`LogReader::entries`] for historical).
    ///
    /// Polls the segments directory at a fixed cadence (default 10ms);
    /// each call to [`Iterator::next`] blocks until a new entry is
    /// readable. Drop the iterator to stop tailing.
    ///
    /// **Read side of the [subscription-as-materialization keystone].**
    /// The detector loop is `for entry in Log::<T>::tail(&path)? { ... }`
    /// — same call regardless of whether the log is being written by a
    /// local sensor driver, materialized from a peer's stream, or
    /// opened from a recording on disk.
    ///
    /// **No EOF detection.** The iterator tails forever — there is no
    /// portable way to detect that all writers have closed. Callers
    /// that need clean shutdown either drop the iterator or use
    /// [`TailIter::try_next`] in a polling loop with their own stop
    /// condition.
    ///
    /// **Catch-up note.** Entries appended *between* `tail()` returning
    /// and the first `next()` call are visible. Entries that were on
    /// disk before `tail()` was called are not. To get historical
    /// entries plus future ones, call `read().entries()` first, then
    /// `tail()`.
    ///
    /// [subscription-as-materialization keystone]: https://github.com/aukilabs/auki-sdk/blob/develop/parking_lot.md
    pub fn tail(root: &Path) -> Result<TailIter<T>> {
        let segments_dir = root.join("segments");
        // Establish the starting position — current EOF of the latest
        // segment, or "no segments yet" if the log is empty.
        let starts = list_segments(&segments_dir)?;
        let (current_segment, current_offset) = match starts.iter().next_back() {
            Some(&latest) => {
                let path = segments_dir.join(segment_filename(latest));
                let len = fs::metadata(&path)?.len();
                // If the segment is fresh and its header hasn't been
                // fsynced yet, len could be < HEADER_SIZE. Clamp up so
                // we don't try to seek before the first entry.
                let offset = std::cmp::max(len, HEADER_SIZE as u64);
                (Some(latest), offset)
            }
            None => (None, 0),
        };
        Ok(TailIter {
            root: root.to_path_buf(),
            poll_interval: Duration::from_millis(10),
            current_segment,
            current_offset,
            _phantom: PhantomData,
        })
    }
}

impl<T> Drop for Log<T> {
    fn drop(&mut self) {
        let _ = self.close_current();
    }
}

pub struct LogReader<T> {
    root: PathBuf,
    manifest: serde_json::Value,
    segment_starts: Vec<i64>,
    _phantom: PhantomData<fn() -> T>,
}

impl<T> LogReader<T>
where
    T: LogPayload,
{
    pub fn manifest(&self) -> &serde_json::Value {
        &self.manifest
    }

    pub fn segment_starts(&self) -> &[i64] {
        &self.segment_starts
    }

    /// Eagerly load every entry from every segment in chronological order.
    pub fn entries(&self) -> Result<Vec<Entry<T>>> {
        let mut out = Vec::new();
        for &start in &self.segment_starts {
            let path = self.root.join("segments").join(segment_filename(start));
            read_segment_entries(&path, &mut out)?;
        }
        Ok(out)
    }
}

/// Iterator returned by [`Log::tail`]. Yields entries as they are
/// appended to the log. See [`Log::tail`] for full semantics.
pub struct TailIter<T> {
    root: PathBuf,
    poll_interval: Duration,
    /// Segment start_ns currently being tailed. `None` means the log
    /// was empty when `tail()` was called and we haven't yet picked
    /// up the first segment.
    current_segment: Option<i64>,
    /// Byte offset within the current segment file. Pointed at the
    /// start of the next entry to read (or at EOF if caught up).
    current_offset: u64,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: LogPayload> TailIter<T> {
    /// Override the poll cadence (default 10ms). Lower values reduce
    /// detection latency; higher values reduce filesystem load when
    /// many tailers run on the same node.
    pub fn with_poll_interval(mut self, dur: Duration) -> Self {
        self.poll_interval = dur;
        self
    }

    /// Non-blocking. Returns `Ok(Some(entry))` if one is ready right
    /// now, `Ok(None)` if no entry is available yet (no I/O wait
    /// beyond a single segment-listing). `Err(_)` only on real I/O or
    /// payload decode failure.
    pub fn try_next(&mut self) -> Result<Option<Entry<T>>> {
        loop {
            // (a) If we have no current segment, try to pick one up.
            if self.current_segment.is_none() {
                let starts = list_segments(&self.root.join("segments"))?;
                match starts.iter().next() {
                    Some(&first) => {
                        self.current_segment = Some(first);
                        self.current_offset = HEADER_SIZE as u64;
                    }
                    None => return Ok(None),
                }
            }

            // (b) Try to read one entry from the current segment.
            let segment_start = self.current_segment.expect("current segment set above");
            let path = self
                .root
                .join("segments")
                .join(segment_filename(segment_start));

            match read_one_entry_at::<T>(&path, self.current_offset)? {
                ReadOne::Got { entry, next_offset } => {
                    self.current_offset = next_offset;
                    return Ok(Some(entry));
                }
                ReadOne::EndOfSegment => {
                    // Current segment exhausted (cleanly or truncated
                    // mid-write). Look for a newer segment.
                    let starts = list_segments(&self.root.join("segments"))?;
                    let next = starts.iter().copied().find(|&s| s > segment_start);
                    match next {
                        Some(next_start) => {
                            self.current_segment = Some(next_start);
                            self.current_offset = HEADER_SIZE as u64;
                            // Loop and try to read from the new segment.
                            continue;
                        }
                        None => return Ok(None),
                    }
                }
                ReadOne::SegmentMissing => {
                    // The segment was evicted while we were tailing
                    // (retention window collapsed past us). Treat as
                    // "find the next surviving segment" — bump
                    // forward, and if there isn't one, return None.
                    let starts = list_segments(&self.root.join("segments"))?;
                    let next = starts.iter().copied().find(|&s| s > segment_start);
                    match next {
                        Some(next_start) => {
                            self.current_segment = Some(next_start);
                            self.current_offset = HEADER_SIZE as u64;
                            continue;
                        }
                        None => {
                            // No surviving segments at all. Reset to
                            // "empty log" state and return None.
                            self.current_segment = None;
                            self.current_offset = 0;
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }
}

impl<T: LogPayload> Iterator for TailIter<T> {
    type Item = Result<Entry<T>>;

    /// Blocks until a new entry is readable. Polls at the configured
    /// cadence (default 10ms). Returns `Some(Err(_))` only on real I/O
    /// or payload decode failure — not on transient "nothing yet."
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.try_next() {
                Ok(Some(entry)) => return Some(Ok(entry)),
                Ok(None) => thread::sleep(self.poll_interval),
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

enum ReadOne<T> {
    Got { entry: Entry<T>, next_offset: u64 },
    EndOfSegment,
    SegmentMissing,
}

/// Read exactly one entry from `path` starting at byte `offset`. The
/// per-entry framing matches `read_segment_entries`: 8-byte timestamp,
/// 4-byte length, payload bytes.
///
/// Returns `EndOfSegment` if any read short-falls (truncated mid-write
/// or clean EOF) — the caller decides whether to stay on this segment
/// or move on. Returns `SegmentMissing` if the file was evicted between
/// `try_next` calls.
fn read_one_entry_at<T: LogPayload>(path: &Path, offset: u64) -> Result<ReadOne<T>> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ReadOne::SegmentMissing),
        Err(e) => return Err(Error::Io(e)),
    };
    file.seek(SeekFrom::Start(offset))?;

    let mut ts_buf = [0u8; 8];
    match file.read_exact(&mut ts_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(ReadOne::EndOfSegment);
        }
        Err(e) => return Err(Error::Io(e)),
    }
    let timestamp_ns = i64::from_le_bytes(ts_buf);

    let mut len_buf = [0u8; 4];
    match file.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(ReadOne::EndOfSegment);
        }
        Err(e) => return Err(Error::Io(e)),
    }
    let payload_len = u32::from_le_bytes(len_buf) as usize;

    let mut payload = vec![0u8; payload_len];
    match file.read_exact(&mut payload) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(ReadOne::EndOfSegment);
        }
        Err(e) => return Err(Error::Io(e)),
    }

    let value = T::decode(&payload).map_err(Error::Payload)?;
    let next_offset = offset + 8 + 4 + payload_len as u64;
    Ok(ReadOne::Got {
        entry: Entry {
            timestamp_ns,
            payload: value,
        },
        next_offset,
    })
}

fn required_durations(manifest: &serde_json::Value) -> Result<(i64, i64)> {
    let segment_duration_ns = manifest
        .get("segment_duration_ns")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            Error::Manifest("manifest missing or non-integer segment_duration_ns".into())
        })?;
    let retention_ns = manifest
        .get("retention_ns")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| Error::Manifest("manifest missing or non-integer retention_ns".into()))?;
    if segment_duration_ns <= 0 {
        return Err(Error::Manifest("segment_duration_ns must be > 0".into()));
    }
    if retention_ns < 0 {
        return Err(Error::Manifest("retention_ns must be ≥ 0".into()));
    }
    Ok((segment_duration_ns, retention_ns))
}

fn list_segments(dir: &Path) -> Result<BTreeSet<i64>> {
    let mut starts = BTreeSet::new();
    if !dir.exists() {
        return Ok(starts);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some(SEGMENT_EXT) {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        match stem.parse::<i64>() {
            Ok(n) => {
                starts.insert(n);
            }
            Err(_) => continue,
        }
    }
    Ok(starts)
}

fn segment_filename(start_ns: i64) -> String {
    // Spec requires start_ns ≥ 0 (filename is unsigned 20-digit zero-pad).
    format!(
        "{:0width$}.{ext}",
        start_ns,
        width = FILENAME_DIGITS,
        ext = SEGMENT_EXT
    )
}

fn read_segment_entries<T: LogPayload>(path: &Path, out: &mut Vec<Entry<T>>) -> Result<()> {
    let mut reader = BufReader::new(File::open(path)?);
    read_segment_entries_from_reader(&mut reader, &path.display().to_string(), out)
}

fn read_segment_entries_from_reader<T: LogPayload, R: Read>(
    reader: &mut R,
    label: &str,
    out: &mut Vec<Entry<T>>,
) -> Result<()> {
    let mut header = [0u8; HEADER_SIZE];
    if let Err(e) = reader.read_exact(&mut header) {
        return Err(Error::Format(format!(
            "segment {label} too short for header: {e}",
        )));
    }
    if &header[0..4] != MAGIC {
        return Err(Error::Format(format!("segment {label} has bad magic")));
    }
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    if version != VERSION {
        return Err(Error::Format(format!(
            "segment {label} has unsupported version {version}",
        )));
    }
    // header[6..8] reserved; header[8..16] start_ns (filename is the canonical source)

    loop {
        let mut ts_buf = [0u8; 8];
        match reader.read_exact(&mut ts_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Error::Io(e)),
        }
        let timestamp_ns = i64::from_le_bytes(ts_buf);

        let mut len_buf = [0u8; 4];
        if reader.read_exact(&mut len_buf).is_err() {
            // Truncated tail past last full entry — stop cleanly.
            break;
        }
        let payload_len = u32::from_le_bytes(len_buf) as usize;

        let mut payload = vec![0u8; payload_len];
        if reader.read_exact(&mut payload).is_err() {
            break;
        }

        let value = T::decode(&payload).map_err(Error::Payload)?;
        out.push(Entry {
            timestamp_ns,
            payload: value,
        });
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    let bytes = hex.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(Error::Format("payload_hex must have even length".into()));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(Error::Format(
            "payload_hex must contain only hex digits".into(),
        )),
    }
}

fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("target has no parent"))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::other("target has no file name"))?;
    let tmp = dir.join(format!(".{}.tmp", file_name.to_string_lossy()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    struct Sample {
        n: u32,
        label: String,
    }

    impl LogPayload for Sample {
        fn encode(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            ciborium::into_writer(self, &mut buf).expect("ciborium encode");
            buf
        }
        fn decode(bytes: &[u8]) -> std::result::Result<Self, String> {
            ciborium::from_reader(bytes).map_err(|e| e.to_string())
        }
    }

    fn manifest(segment_ns: i64, retention_ns: i64) -> serde_json::Value {
        json!({
            "segment_duration_ns": segment_ns,
            "retention_ns": retention_ns,
            "kind": "test"
        })
    }

    #[test]
    fn canonical_manifest_json_str_sorts_keys() {
        let canonical = canonical_manifest_json_str(
            r#"{"retention_ns":5000000000,"kind":"test","segment_duration_ns":1000000000}"#,
        )
        .unwrap();
        assert_eq!(
            canonical,
            r#"{"kind":"test","retention_ns":5000000000,"segment_duration_ns":1000000000}"#
        );
    }

    #[test]
    fn opaque_segment_bytes_round_trip() {
        let entries = vec![
            Entry {
                timestamp_ns: 100,
                payload: BytesPayload {
                    bytes: vec![0x01, 0x02, 0x03],
                },
            },
            Entry {
                timestamp_ns: 200,
                payload: BytesPayload {
                    bytes: b"hello".to_vec(),
                },
            },
        ];
        let segment = encode_segment_bytes(0, &entries).unwrap();
        assert_eq!(&segment[0..4], b"AKLG");
        assert_eq!(u16::from_le_bytes(segment[4..6].try_into().unwrap()), 1);
        assert_eq!(i64::from_le_bytes(segment[8..16].try_into().unwrap()), 0);

        let decoded = decode_segment_bytes(&segment).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].timestamp_ns, 100);
        assert_eq!(decoded[0].payload.bytes, vec![0x01, 0x02, 0x03]);
        assert_eq!(decoded[1].timestamp_ns, 200);
        assert_eq!(decoded[1].payload.bytes, b"hello");

        let json = bytes_entries_to_json(&decoded);
        assert_eq!(
            json,
            r#"[{"payload_hex":"010203","timestamp_ns":100},{"payload_hex":"68656c6c6f","timestamp_ns":200}]"#
        );
        let reparsed = bytes_entries_from_json(&json).unwrap();
        assert_eq!(bytes_entries_to_json(&reparsed), json);
    }

    #[test]
    fn open_creates_layout_and_writes_manifest() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _log: Log<Sample> =
                Log::open(dir.path(), manifest(1_000_000_000, 5_000_000_000)).unwrap();
        }
        assert!(dir.path().join("log_manifest.json").exists());
        assert!(dir.path().join("segments").is_dir());
        // Manifest is JCS-canonical: keys sorted lexicographically.
        let written = fs::read_to_string(dir.path().join("log_manifest.json")).unwrap();
        assert_eq!(
            written,
            r#"{"kind":"test","retention_ns":5000000000,"segment_duration_ns":1000000000}"#
        );
    }

    #[test]
    fn round_trip_single_segment() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut log: Log<Sample> =
                Log::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap();
            log.append(
                100,
                &Sample {
                    n: 1,
                    label: "a".into(),
                },
            )
            .unwrap();
            log.append(
                200,
                &Sample {
                    n: 2,
                    label: "b".into(),
                },
            )
            .unwrap();
        } // drop closes + fsyncs the segment

        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp_ns, 100);
        assert_eq!(entries[0].payload.label, "a");
        assert_eq!(entries[1].timestamp_ns, 200);
        assert_eq!(entries[1].payload.n, 2);
    }

    #[test]
    fn rolls_over_at_segment_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64; // 1s
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            // First segment [0, 1s)
            log.append(
                100,
                &Sample {
                    n: 1,
                    label: "s1a".into(),
                },
            )
            .unwrap();
            log.append(
                seg - 1,
                &Sample {
                    n: 2,
                    label: "s1b".into(),
                },
            )
            .unwrap();
            // Crosses into [1s, 2s)
            log.append(
                seg,
                &Sample {
                    n: 3,
                    label: "s2a".into(),
                },
            )
            .unwrap();
            // And [2s, 3s)
            log.append(
                2 * seg + 500,
                &Sample {
                    n: 4,
                    label: "s3a".into(),
                },
            )
            .unwrap();
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        assert_eq!(reader.segment_starts(), &[0, seg, 2 * seg]);
        let entries = reader.entries().unwrap();
        let labels: Vec<_> = entries.iter().map(|e| e.payload.label.clone()).collect();
        assert_eq!(labels, vec!["s1a", "s1b", "s2a", "s3a"]);
    }

    #[test]
    fn evicts_segments_older_than_retention() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        let retention = 3 * seg; // keep last 3 seconds
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, retention)).unwrap();
            for i in 0..10 {
                log.append(
                    i * seg + 100,
                    &Sample {
                        n: i as u32,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
            }
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let starts = reader.segment_starts();
        // Last append timestamp ≈ 9s+100. Threshold = 9s+100 - 3s = 6s+100.
        // Segment at start S covers [S, S+1s); evict if S+1s ≤ threshold,
        // i.e. S ≤ threshold - 1s = 5s+100, i.e. S ≤ 5s. So 0,1,2,3,4,5 evict.
        // 6,7,8,9 remain (9 is current).
        assert_eq!(starts, &[6 * seg, 7 * seg, 8 * seg, 9 * seg]);
    }

    #[test]
    fn retention_zero_disables_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 0)).unwrap();
            for i in 0..10 {
                log.append(
                    i * seg + 100,
                    &Sample {
                        n: i as u32,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
            }
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let starts: Vec<i64> = reader.segment_starts().to_vec();
        assert_eq!(starts, (0..10).map(|i| i * seg).collect::<Vec<_>>());
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 10);
    }

    #[test]
    fn segment_file_header_is_well_formed() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        let start_ts = 5 * seg + 42;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            log.append(
                start_ts,
                &Sample {
                    n: 7,
                    label: "x".into(),
                },
            )
            .unwrap();
        }

        let expected_start_ns = 5 * seg;
        let filename = format!("{:020}.seg", expected_start_ns);
        let mut bytes = Vec::new();
        File::open(dir.path().join("segments").join(&filename))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(&bytes[0..4], b"AKLG");
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 1); // version
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 0); // reserved
        assert_eq!(
            i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            expected_start_ns
        );
        assert!(bytes.len() > 16, "segment must contain entries past header");
    }

    #[test]
    fn reopen_preserves_existing_manifest_and_appends_new_segments() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            log.append(
                100,
                &Sample {
                    n: 1,
                    label: "first".into(),
                },
            )
            .unwrap();
        }
        let manifest_before = fs::read(dir.path().join("log_manifest.json")).unwrap();
        {
            // Re-open with a *different* manifest — on-disk should win.
            let mut log: Log<Sample> = Log::open(
                dir.path(),
                json!({
                    "segment_duration_ns": 9999,
                    "retention_ns": 9999,
                    "kind": "wrong"
                }),
            )
            .unwrap();
            log.append(
                3 * seg + 50,
                &Sample {
                    n: 2,
                    label: "later".into(),
                },
            )
            .unwrap();
        }
        let manifest_after = fs::read(dir.path().join("log_manifest.json")).unwrap();
        assert_eq!(manifest_before, manifest_after);

        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].payload.label, "first");
        assert_eq!(entries[1].payload.label, "later");
    }

    #[test]
    fn truncated_segment_tail_is_tolerated_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            log.append(
                10,
                &Sample {
                    n: 1,
                    label: "ok".into(),
                },
            )
            .unwrap();
            log.flush().unwrap();
        }
        // Simulate a crash: append garbage past the last full entry.
        let seg_path = dir.path().join("segments").join(format!("{:020}.seg", 0));
        let mut f = OpenOptions::new().append(true).open(&seg_path).unwrap();
        f.write_all(&[0xAA, 0xBB, 0xCC]).unwrap(); // partial timestamp_ns
        drop(f);

        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let entries = reader.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].payload.label, "ok");
    }

    #[test]
    fn manifest_missing_required_field_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = json!({"retention_ns": 1_000});
        let result: Result<Log<Sample>> = Log::open(dir.path(), bad);
        assert!(matches!(result, Err(Error::Manifest(_))));
    }

    #[test]
    fn manifest_zero_duration_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = json!({"segment_duration_ns": 0, "retention_ns": 1_000});
        let result: Result<Log<Sample>> = Log::open(dir.path(), bad);
        assert!(matches!(result, Err(Error::Manifest(_))));
    }

    #[test]
    fn set_retention_shrinks_window_for_subsequent_appends() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            // Fill 5 segments under generous (60 s) retention.
            for i in 0..5 {
                log.append(
                    i * seg + 100,
                    &Sample {
                        n: i as u32,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
            }
            // Shrink retention to 1 s — affects future appends only.
            log.set_retention(seg).unwrap();
            // This append triggers eviction with the new retention.
            log.append(
                5 * seg + 100,
                &Sample {
                    n: 5,
                    label: "new".into(),
                },
            )
            .unwrap();
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let starts: Vec<i64> = reader.segment_starts().to_vec();
        // After append at 5s+100 with retention=1s: threshold = 4s+100.
        // Evict where start+1s ≤ threshold, i.e. start ≤ 3s+100, i.e.
        // start ≤ 3s. So 0,1,2,3 evict; 4 and 5 (current) remain.
        assert_eq!(starts, vec![4 * seg, 5 * seg]);
    }

    #[test]
    fn set_retention_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            log.set_retention(3 * seg).unwrap();
            // The in-memory manifest reflects the new value.
            assert_eq!(
                log.manifest().get("retention_ns").and_then(|v| v.as_i64()),
                Some(3 * seg)
            );
        }
        // The on-disk manifest reflects the new value.
        let written = fs::read_to_string(dir.path().join("log_manifest.json")).unwrap();
        assert!(
            written.contains(r#""retention_ns":3000000000"#),
            "manifest did not persist set_retention: {written}"
        );

        // Re-open. On-disk manifest is the source of truth — the
        // `manifest` argument we pass here is ignored per `open`'s
        // contract, so we use the same one to keep the test focused.
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
            // Append-driven eviction should use the persisted 3 s, not
            // the original 60 s the manifest argument carries.
            for i in 0..10 {
                log.append(
                    i * seg + 100,
                    &Sample {
                        n: i as u32,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
            }
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let starts = reader.segment_starts();
        // Same arithmetic as `evicts_segments_older_than_retention`
        // with retention = 3 s.
        assert_eq!(starts, &[6 * seg, 7 * seg, 8 * seg, 9 * seg]);
    }

    #[test]
    fn set_retention_rejects_negative() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
        let result = log.set_retention(-1);
        assert!(matches!(result, Err(Error::Manifest(_))));
        // The in-memory state is unchanged on rejection.
        assert_eq!(
            log.manifest().get("retention_ns").and_then(|v| v.as_i64()),
            Some(60 * seg)
        );
    }

    #[test]
    fn set_retention_zero_disables_future_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        {
            // Start with a tight retention, fill some segments, then
            // disable eviction by setting retention to 0.
            let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, seg)).unwrap();
            // Two segments survive under retention=1s after the second
            // append (start=0 evicts when threshold passes 1s).
            log.append(
                100,
                &Sample {
                    n: 0,
                    label: "a".into(),
                },
            )
            .unwrap();
            log.set_retention(0).unwrap();
            // Now flood — nothing should evict.
            for i in 1..=5 {
                log.append(
                    i * seg + 100,
                    &Sample {
                        n: i as u32,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
            }
        }
        let reader: LogReader<Sample> = Log::<Sample>::read(dir.path()).unwrap();
        let starts: Vec<i64> = reader.segment_starts().to_vec();
        // All six segments retained.
        assert_eq!(starts, (0..=5).map(|i| i * seg).collect::<Vec<_>>());
    }

    // ─── tail() tests ───────────────────────────────────────────────────────

    #[test]
    fn tail_starts_at_current_eof_skipping_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        // Single writer for the whole test — Log<T> can't extend an
        // existing segment after re-open today (the first start_segment
        // uses create_new(true)). The tail-side of the keystone is
        // unaffected because the production caller is a long-lived
        // sensor driver, not a re-opening one.
        let mut log: Log<Sample> =
            Log::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap();
        log.append(
            100,
            &Sample {
                n: 1,
                label: "skip-1".into(),
            },
        )
        .unwrap();
        log.append(
            200,
            &Sample {
                n: 2,
                label: "skip-2".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path()).unwrap();
        // Nothing new yet — tail starts at EOF.
        assert!(tail.try_next().unwrap().is_none());

        // Append a new entry; tail picks it up on the next try_next.
        log.append(
            300,
            &Sample {
                n: 3,
                label: "after-tail".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let entry = tail.try_next().unwrap().expect("entry should be ready");
        assert_eq!(entry.timestamp_ns, 300);
        assert_eq!(entry.payload.label, "after-tail");
        assert!(tail.try_next().unwrap().is_none());
    }

    #[test]
    fn tail_on_empty_log_picks_up_first_entry_when_it_arrives() {
        let dir = tempfile::tempdir().unwrap();
        // Manifest exists, no segments yet.
        let mut log: Log<Sample> =
            Log::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap();

        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path()).unwrap();
        assert!(tail.try_next().unwrap().is_none());

        // Append the first-ever entry; tail picks it up.
        log.append(
            500,
            &Sample {
                n: 1,
                label: "first".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let entry = tail.try_next().unwrap().expect("first entry");
        assert_eq!(entry.timestamp_ns, 500);
        assert_eq!(entry.payload.label, "first");
    }

    #[test]
    fn tail_blocking_next_yields_entries_in_order() {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(
            Log::<Sample>::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap(),
        ));

        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path())
            .unwrap()
            .with_poll_interval(Duration::from_millis(2));

        // Spawn a writer that appends three entries with small gaps.
        let writer_done = Arc::new(AtomicBool::new(false));
        let writer_done_clone = Arc::clone(&writer_done);
        let log_clone = Arc::clone(&log);
        let writer = std::thread::spawn(move || {
            for i in 1..=3 {
                std::thread::sleep(Duration::from_millis(20));
                let mut log = log_clone.lock().unwrap();
                log.append(
                    (i as i64) * 100,
                    &Sample {
                        n: i,
                        label: format!("e{i}"),
                    },
                )
                .unwrap();
                log.flush().unwrap();
            }
            writer_done_clone.store(true, Ordering::SeqCst);
        });

        // Pull three entries via blocking next().
        let mut got = Vec::new();
        for _ in 0..3 {
            let entry = tail.next().expect("blocking next yields").unwrap();
            got.push((entry.timestamp_ns, entry.payload.label));
        }
        writer.join().unwrap();
        assert!(writer_done.load(Ordering::SeqCst));
        assert_eq!(
            got,
            vec![
                (100, "e1".to_string()),
                (200, "e2".to_string()),
                (300, "e3".to_string())
            ]
        );
    }

    #[test]
    fn tail_jumps_to_next_segment_on_rollover() {
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64; // 1s segments
        let mut log: Log<Sample> = Log::open(dir.path(), manifest(seg, 60 * seg)).unwrap();
        // Seed with one entry in segment [0, 1s) so tail starts past it.
        log.append(
            100,
            &Sample {
                n: 0,
                label: "seed".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path()).unwrap();
        assert!(tail.try_next().unwrap().is_none());

        // Append in the same segment first, then cross the rollover.
        log.append(
            500,
            &Sample {
                n: 1,
                label: "same-seg".into(),
            },
        )
        .unwrap();
        // Crossing 1s rolls over to segment [1s, 2s).
        log.append(
            seg + 100,
            &Sample {
                n: 2,
                label: "next-seg".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let e1 = tail.try_next().unwrap().expect("same-seg entry");
        assert_eq!(e1.payload.label, "same-seg");
        let e2 = tail.try_next().unwrap().expect("next-seg entry");
        assert_eq!(e2.payload.label, "next-seg");
        assert_eq!(e2.timestamp_ns, seg + 100);
        assert!(tail.try_next().unwrap().is_none());
    }

    #[test]
    fn tail_tolerates_partial_entry_during_concurrent_append() {
        // The writer's append path does timestamp / length / payload as
        // three separate write_all calls. If the tailer reads between
        // the length write and the payload write (or earlier), the
        // segment looks truncated past the last full entry. tail should
        // return Ok(None) — not Err — and recover on the next poll.
        //
        // We simulate this with a fixture written manually: a complete
        // entry plus partial bytes for a "next" entry. try_next should
        // read the complete entry and then return Ok(None) on the
        // partial remainder.

        let dir = tempfile::tempdir().unwrap();
        let mut log: Log<Sample> =
            Log::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap();
        log.append(
            100,
            &Sample {
                n: 1,
                label: "complete".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path()).unwrap();
        // Tail starts past the existing entry — nothing to read.
        assert!(tail.try_next().unwrap().is_none());

        // Append a real entry the tailer should see, then add partial
        // bytes for the next entry to simulate a mid-write tail.
        log.append(
            200,
            &Sample {
                n: 2,
                label: "ok".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();

        // First try_next reads the complete entry.
        let entry = tail.try_next().unwrap().expect("complete entry");
        assert_eq!(entry.timestamp_ns, 200);
        assert_eq!(entry.payload.label, "ok");

        // Manually append partial bytes (just a timestamp prefix, no
        // length or payload) to simulate a torn write the tailer
        // happened to observe between fsyncs.
        {
            let segments = list_segments(&dir.path().join("segments")).unwrap();
            let only = *segments.iter().next().unwrap();
            let seg_path = dir.path().join("segments").join(segment_filename(only));
            let mut f = OpenOptions::new().append(true).open(&seg_path).unwrap();
            // Drop just a timestamp prefix — no length, no payload.
            f.write_all(&300i64.to_le_bytes()).unwrap();
            f.sync_all().unwrap();
        }

        // try_next sees a partial entry → returns Ok(None) (not Err).
        assert!(tail.try_next().unwrap().is_none());

        // Note: the tailer's offset is parked at the start of the
        // partial entry. A real subsequent flushed append would land
        // past that point — but auki-logs's writer doesn't know about
        // the manual append we just did, and would write at its own
        // tracked position (overwriting our garbage on the next flush
        // path). For the partial-tolerance contract, "Ok(None) instead
        // of Err on a torn read" is the property we needed to assert.
    }

    #[test]
    fn tail_ignores_evicted_segments_and_resumes_at_newer_one() {
        // If retention evicts the segment we're tailing, try_next
        // should silently advance to the next surviving segment
        // instead of erroring.
        let dir = tempfile::tempdir().unwrap();
        let seg = 1_000_000_000i64;
        // 2-segment retention so segment [0, 1s) gets evicted when
        // segment [2s, 3s) is started.
        let manifest_2s = manifest(seg, 2 * seg);
        {
            let mut log: Log<Sample> = Log::open(dir.path(), manifest_2s.clone()).unwrap();
            log.append(
                0,
                &Sample {
                    n: 0,
                    label: "seg0".into(),
                },
            )
            .unwrap();
            log.flush().unwrap();
        }

        // Tail is positioned inside segment [0, 1s).
        let mut tail: TailIter<Sample> = Log::<Sample>::tail(dir.path()).unwrap();
        assert!(tail.try_next().unwrap().is_none());

        // Write enough to evict segment [0, 1s): need to roll over
        // such that [0, 1s) ends ≤ retention threshold.
        let mut log: Log<Sample> = Log::open(dir.path(), manifest_2s.clone()).unwrap();
        log.append(
            seg + 100,
            &Sample {
                n: 1,
                label: "seg1".into(),
            },
        )
        .unwrap();
        log.append(
            3 * seg + 100,
            &Sample {
                n: 3,
                label: "seg3".into(),
            },
        )
        .unwrap();
        log.flush().unwrap();
        drop(log);

        // Confirm the original segment was evicted.
        let surviving = list_segments(&dir.path().join("segments")).unwrap();
        assert!(!surviving.contains(&0i64), "segment 0 should be evicted");

        // tail should advance past the evicted segment without error.
        let mut got = Vec::new();
        while let Some(entry) = tail.try_next().unwrap() {
            got.push(entry.payload.label);
        }
        assert!(
            got.contains(&"seg1".to_string()) || got.contains(&"seg3".to_string()),
            "tail recovers from eviction and yields surviving entries; got {got:?}"
        );
    }

    #[test]
    fn tail_with_poll_interval_overrides_default() {
        // Smoke test that the with_poll_interval builder is wired up.
        let dir = tempfile::tempdir().unwrap();
        let _log: Log<Sample> =
            Log::open(dir.path(), manifest(1_000_000_000, 60_000_000_000)).unwrap();
        let tail: TailIter<Sample> = Log::<Sample>::tail(dir.path())
            .unwrap()
            .with_poll_interval(Duration::from_millis(50));
        assert_eq!(tail.poll_interval, Duration::from_millis(50));
    }
}
