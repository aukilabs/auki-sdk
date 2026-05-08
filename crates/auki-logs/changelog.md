# Changelog — auki-logs

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 8, 19:46 HKT, 2026

**`Log<T>::tail()` — read side of the [subscription-as-materialization keystone](../../parking_lot.md).** New `pub fn Log::<T>::tail(root: &Path) -> Result<TailIter<T>>` returns an iterator that yields newly-appended entries as they become readable. Starts at the **current EOF** of the log (existing entries are not replayed; use `read().entries()` for historical). `Iterator::next` blocks at the configurable poll cadence (default 10ms); `TailIter::try_next` is non-blocking. Drop the iterator to stop tailing.

**Same call regardless of where the bytes came from.** The Detector loop is `for entry in Log::<SensorLogEntry>::tail(&path)? { ... }` whether the log is being written by a local sensor driver, materialized from a peer's stream, or opened from a recording on disk. The transport differs (zero-hop, libp2p, file source); the tail call doesn't.

**Robustness:** the iterator handles segment rollover (jumps to the next `.seg` when the current one ends), torn reads from a writer mid-`append` (timestamp + length + payload are three separate writes — a tail that lands between them surfaces as `Ok(None)`, not `Err`, and recovers on the next poll), and segment eviction (advances past evicted segments without erroring). Reads are stateless per `try_next` call (file open + seek + one entry + close) so eviction or rollover between calls is fine.

**No EOF detection in v1.** The iterator tails forever — no portable way to detect that all writers have closed. Callers needing clean shutdown drop the iterator or use `try_next` in their own polling loop with a stop condition. Filed as a parking-lot follow-up.

**Resolves [`detectors`](https://github.com/aukilabs/detectors) phase-2 blocker #1** ("`Log<SensorLogEntry>::tail()`"). Phase-2 blocker #3 (`DetectionLogEntry`) shipped in [Step 8 of the `auki-datatypes` migration](../auki-datatypes/src/sprint.md). Remaining: blocker #2 (Detector binding API) and blocker #4 (`auki-sdk-py` Python binding).

**Filed alongside in [`parking_lot.md`](parking_lot.md):**
- **`Log::open` can't extend an existing partially-filled segment after re-open** (`OpenOptions::create_new(true)` fails on the existing segment file). Surfaced when the test pass tried daemon-restart-style write patterns. Lean: re-attach to the latest segment in `OpenOptions::write(true).append(true)` mode + `flock` for race safety. Not blocking the keystone work — production `tail()` consumers read from the same long-lived `Log<T>` writer.
- **`tail` follow-on shapes** punted to future PRs: `tail_from(timestamp_ns)` for replay-from-checkpoint; EOF detection for non-streaming consumers; notify-based backend instead of polling for high-frequency streams.

**Tests**: 13 → 21 (+8 — `tail_starts_at_current_eof_skipping_existing_entries`, `tail_on_empty_log_picks_up_first_entry_when_it_arrives`, `tail_blocking_next_yields_entries_in_order` (concurrent writer thread), `tail_jumps_to_next_segment_on_rollover`, `tail_tolerates_partial_entry_during_concurrent_append`, `tail_ignores_evicted_segments_and_resumes_at_newer_one`, `tail_with_poll_interval_overrides_default`).

**No new deps.** `std::thread::sleep` + `std::time::Duration` from the standard library; the polling backend is intentionally simple. `notify` (filesystem events) is a future enhancement, gated behind a real profiling reason.

Will land in v0.0.25.

### broodsugar's claude · May 8, 11:30 HKT, 2026

**`Log<T>` is encoder-agnostic** — Step 1 of the [`auki-datatypes` migration](../auki-datatypes/src/sprint.md) lands the resolution to the open question added 2026-05-07. New `pub trait LogPayload { encode(&self) -> Vec<u8>; decode(&[u8]) -> Result<Self, String>; }`; `Log<T>` and `LogReader<T>` bounds change from `T: Serialize + DeserializeOwned` to `T: LogPayload`. Encoder choice moves to the consumer.

**ciborium dropped from production deps**; promoted to dev-dep for the in-crate test scaffolding's `Sample` `LogPayload` impl. `Error::Cbor(String)` → `Error::Payload(String)` (breaking but trivial — no production matchers).

**Why encoder-agnostic**: locking out a hardcoded encoder mixes concerns the migration is otherwise pulling apart. `auki-datatypes` owns prost-encoded segment payloads; `auki-time-transforms` is still on ciborium for `TimeTransformEntry` until Step 6; future encoders (Cap'n Proto, FlatBuffers, anything we haven't thought of) plug in the same way. The consumer impls one method pair per type.

**Production code in `auki-datatypes`** ships an `impl_log_payload!` macro that gives every prost-generated message a one-line impl; mid-migration ciborium types in `auki-time-transforms` (`TimeTransformEntry`) and `auki-manifests`'s `TestEntry` test scaffold write theirs directly.

**On-disk format unchanged** — segment header, length-prefixed entry framing, atomic writes, retention semantics all identical. Only the payload-byte encoding moves out of this crate's responsibility. Test count 14 → 14 (the in-crate `Sample` ciborium round-trip still passes through the trait).

**Doc updates**: README's "Why CBOR" section becomes "Why encoding-agnostic"; `src/readme.md` public API section + Why subsection rewritten; entry-framing tables drop the `CBOR (RFC 8949)` line. Will land in v0.0.24.

### broodsugar's dobby · May 7, 22:30 HKT, 2026

**[`parking_lot.md`](parking_lot.md): encoder-aware vs encoder-agnostic `Log<T>` post-migration.** New open question framing the generic-bound shape of `Log<T>` after the [`auki-datatypes`](../auki-datatypes) migration switches segment payload encoding from CBOR-via-ciborium to prost. Lean: encoding-agnostic — `auki-logs` is supposed to be format-neutral framing; bolting prost into the bound mixes concerns and locks out future encoders. Decided in step 1 of the migration. Doc-only.

### broodsugar's claude · May 7, 17:30 HKT, 2026

README path-placeholder rename to track the [Control API rewrite](../../docs/control-api.md): `<session>/sensorlogs/<recording_uuid>/` → `<session>/sensorlogs/<sensor_log_id>/`. Same listing also gains a sibling line `<session>/poselogs/<pose_log_id>/` (was missing despite poselogs being a real `auki-logs` consumer since v0.0.10). Doc-only; no code changes.

### broodsugar's claude · May 5, 11:00 HKT, 2026

`Log<T>::set_retention(&mut self, retention_ns: i64) -> Result<()>` added — closes the long-flagged gap that left Sentinel's `PATCH /api/buffer` endpoint returning `501 Not Implemented`. Updates the in-memory `retention_ns` and rewrites `manifest.json` atomically (the existing `atomic_write` helper: `.tmp` → fsync → rename) so the change survives daemon restart. **Disk-write-first ordering**: the manifest is persisted before the in-memory field is updated, so a failed write leaves the log unchanged (in-memory state stays consistent with the on-disk source of truth). Validation matches `required_durations` from `open` — negative values rejected with `Error::Manifest`; zero is permitted and disables future eviction (same semantics as opening a log with `retention_ns: 0`). **Eviction is not retroactive** — it runs as part of `append`, not `set_retention`, so a quiescent log retains its current segments until something appends after the change. Caller can `flush()` and drive any subsequent `append` to force immediate effect. The application owns the policy decision (when, why, what value, who's authorized); the SDK exposes the mechanism because the underlying `retention_ns` field IS owned by `Log<T>`. Without an SDK-side path the only alternative would be close-the-log + edit-manifest-on-disk + reopen, which has unacceptable data-loss windows for the actual `PATCH /api/buffer` use case (operator extends a buffer to capture a transient event — dropping data during the change defeats the workflow). 4 new tests: `set_retention_shrinks_window_for_subsequent_appends` (fill 5 segments under 60 s retention, shrink to 1 s, next append evicts old segments using new value), `set_retention_persists_across_reopen` (set + drop log + reopen + verify on-disk manifest drives eviction not the original arg), `set_retention_rejects_negative` (— 1 → `Error::Manifest`; in-memory state unchanged), `set_retention_zero_disables_future_eviction` (mid-run switch to 0 stops further eviction). Test count 10 → 14. Operator UX in `auki-logs/README.md`'s Eviction section gets a new \"Runtime mutability\" subsection; `src/README` adds the function to the public API and grows the tests table.

### broodsugar's claude · May 4, 09:24 HKT, 2026

Filesystem layout diagram now lists `tags.jsonl` as a reserved sibling to `manifest.json`, with a one-paragraph note that the auki-logs writer doesn't produce or consume it (TagClaim handling lives outside the crate boundary). Spec gap fix only — the sidecar is fully described in the root [`tags.md`](../../tags.md) but was previously invisible from the per-crate spec, so directory-enumerating tooling could silently miss it. No code changes.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
