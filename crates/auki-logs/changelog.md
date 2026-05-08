# Changelog — auki-logs

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

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
