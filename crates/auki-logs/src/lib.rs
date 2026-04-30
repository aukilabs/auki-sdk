//! Generic segmented ring-buffer log primitive.
//!
//! On-disk format spec: [`docs/segment-format.md`](../../../docs/segment-format.md).
//!
//! A `Log<T>` writes entries `(timestamp_ns, T)` to time-bounded segment files
//! under `<root>/segments/`. Segments roll over when an appended entry's
//! timestamp leaves the current segment's window. Segments outside the
//! retention window are evicted on append.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

const MAGIC: &[u8; 4] = b"AKLG";
const VERSION: u16 = 1;
const HEADER_SIZE: usize = 16;
const SEGMENT_EXT: &str = "seg";
const FILENAME_DIGITS: usize = 20;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Cbor(String),
    Manifest(String),
    Format(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Cbor(s) => write!(f, "cbor: {s}"),
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
}

impl<T> Log<T>
where
    T: Serialize + DeserializeOwned,
{
    /// Open or create a log directory at `root`. If `manifest.json` is missing,
    /// `manifest` is canonicalized (RFC 8785) and written. If present, the
    /// on-disk manifest is the source of truth and `manifest` is ignored.
    pub fn open(root: &Path, manifest: serde_json::Value) -> Result<Self> {
        fs::create_dir_all(root)?;
        fs::create_dir_all(root.join("segments"))?;

        let manifest_path = root.join("manifest.json");
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
            let start_ns =
                (timestamp_ns / self.segment_duration_ns) * self.segment_duration_ns;
            self.start_segment(start_ns)?;
        }

        let mut payload_bytes = Vec::new();
        ciborium::into_writer(payload, &mut payload_bytes)
            .map_err(|e| Error::Cbor(e.to_string()))?;
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
        let manifest_bytes = fs::read(root.join("manifest.json"))?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| Error::Manifest(format!("parsing manifest: {e}")))?;
        let segment_starts: Vec<i64> = list_segments(&root.join("segments"))?
            .into_iter()
            .collect();
        Ok(LogReader {
            root: root.to_path_buf(),
            manifest,
            segment_starts,
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
    T: DeserializeOwned,
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
    format!("{:0width$}.{ext}", start_ns, width = FILENAME_DIGITS, ext = SEGMENT_EXT)
}

fn read_segment_entries<T: DeserializeOwned>(path: &Path, out: &mut Vec<Entry<T>>) -> Result<()> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut header = [0u8; HEADER_SIZE];
    if let Err(e) = reader.read_exact(&mut header) {
        return Err(Error::Format(format!(
            "segment {} too short for header: {e}",
            path.display()
        )));
    }
    if &header[0..4] != MAGIC {
        return Err(Error::Format(format!(
            "segment {} has bad magic",
            path.display()
        )));
    }
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    if version != VERSION {
        return Err(Error::Format(format!(
            "segment {} has unsupported version {version}",
            path.display()
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

        let value: T = ciborium::from_reader(&payload[..])
            .map_err(|e| Error::Cbor(e.to_string()))?;
        out.push(Entry {
            timestamp_ns,
            payload: value,
        });
    }
    Ok(())
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
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    struct Sample {
        n: u32,
        label: String,
    }

    fn manifest(segment_ns: i64, retention_ns: i64) -> serde_json::Value {
        json!({
            "segment_duration_ns": segment_ns,
            "retention_ns": retention_ns,
            "kind": "test"
        })
    }

    #[test]
    fn open_creates_layout_and_writes_manifest() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _log: Log<Sample> =
                Log::open(dir.path(), manifest(1_000_000_000, 5_000_000_000)).unwrap();
        }
        assert!(dir.path().join("manifest.json").exists());
        assert!(dir.path().join("segments").is_dir());
        // Manifest is JCS-canonical: keys sorted lexicographically.
        let written = fs::read_to_string(dir.path().join("manifest.json")).unwrap();
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
        let manifest_before = fs::read(dir.path().join("manifest.json")).unwrap();
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
        let manifest_after = fs::read(dir.path().join("manifest.json")).unwrap();
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
}
