# Parking lot — auki-logs

---

## Per-entry checksums

Segment files have no per-entry checksum. A `LogPayload::decode` failure on non-truncation corruption surfaces as `Error::Payload` rather than being attributable to a specific entry, and adjacent entries past the corrupt one stop being readable. Should we add a CRC32C per entry? Tradeoff: ~4 bytes/entry overhead (~0.4% on typical 1 KB payloads) vs. better diagnosis of mid-segment corruption.

## Reader streaming for unbounded captures

`LogReader::entries()` eagerly loads every entry across every segment. For long captures — especially with `retention_ns = 0` — this can be very large. Add a streaming iterator API (yields `Entry<T>` one at a time without buffering all segments), or leave it to consumers (renderer, analysis tools) to read individual segments themselves?

## Re-opening a log can't extend a partially-filled segment _(filed 2026-05-08, surfaced by the `tail()` test pass)_

`Log::<T>::open` on an existing log directory followed by `append` whose timestamp falls inside the latest existing segment's window fails with `Err(Io(AlreadyExists))`. The cause is `start_segment` using `OpenOptions::create_new(true)` for safety against two writers sharing a segment file — but it forgets that the *same* writer might be re-attaching after a daemon restart. The currently-running tests get away with this because they keep a single `Log<T>` alive across the full run; a real restart sequence (Park / Boosterapp daemon stops, restarts, resumes appending into the segment that was open at shutdown) hits this immediately.

Forward paths:
- **(a) Re-attach to the existing segment on open.** If the latest segment's filename window contains "now" (or just any timestamp the next append will land in), open it with `OpenOptions::write(true).append(true)` and pre-load the seek position to `len()`. Drop `create_new` for the latest segment only; new segments past rollover still use `create_new`. Two-writers-share-segment risk reappears for the latest segment but is mitigated by exclusive `flock` (Unix) or `LockFileEx` (Windows) on segment open.
- **(b) Start a new segment immediately on re-open.** Less surgery: don't try to share the latest existing segment at all; the next `append` always opens a fresh segment file (using a `start_ns` ≥ the latest existing segment's `start_ns + segment_duration_ns`, even if the timestamp would naturally place it earlier). Wastes some segment-file granularity (a daemon that restarts every 30s with 1-second segments ends up with mostly-empty 30s offset segments) but no race risk.
- **(c) Keep the current behaviour, document it.** Force callers to never restart a writer mid-segment. Practically that means: daemons close and rotate before restart, OR they keep their `Log<T>` alive across the whole process lifetime. Real-world risk: a hard crash leaves the segment closed cleanly (Drop fsyncs the writer), but a restart inside the original segment window can't continue.

Lean: (a). The append-only filesystem semantics already match what we want; the only blocker is `create_new`. Pin file-locking-via-`flock` on the latest segment as the v1 race fix; allow the second writer to fail loudly if it can't acquire the lock.

Don't gate the keystone work on this — the production `tail()` consumer reads from the same long-lived `Log<T>` writer (Park's runtime owns the writer for the lifetime of the session). File so it surfaces when daemon-restart-resume becomes a concrete user story.

## `tail` semantics deferred for the v1 landing _(filed 2026-05-08, alongside the `tail()` PR)_

The first `tail()` landing pinned the simplest viable shape: starts at current EOF, polls the segments dir at a configurable cadence, blocks on `next()`, returns `Ok(None)` from `try_next()` on torn reads. Three sub-decisions punted to follow-on PRs once a real consumer needs them:

- **`tail_from(timestamp_ns)` — replay-from-checkpoint shape.** A consumer that crashed mid-stream wants to resume from the last detection it produced, not lose the bytes appended while it was down. Easy to add: pre-seek to the segment that brackets `timestamp_ns` and skip entries until `entry.timestamp_ns ≥ checkpoint`. File when a Detector's persistence model needs it.
- **EOF detection.** No portable way today to tell "writer detached" from "writer is just slow." Polling forever is fine for the long-lived Detector use case; if a CLI/replay tool wants `for entry in tail() { ... }` to terminate naturally when the recording is done, we'd need an explicit signal — manifest field marking the log as closed, or an end-of-stream sentinel entry. Defer until a non-streaming consumer asks.
- **Notify-based backend instead of polling.** The default 10ms poll is fine for sensor-log frequencies (30 Hz cameras, 1 Hz time transforms). A high-frequency consumer (1 kHz IMU?) might want filesystem-event-driven instead. Adds a `notify` crate dependency + per-platform fallbacks. Defer until polling shows up in a real profile.