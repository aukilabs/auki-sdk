# Changelog — auki-registry

Append-only changelog for this crate. See [CLAUDE.md](../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### broodsugar's claude · May 4, 10:22 HKT, 2026

Sensor Log family manifest gains a required `session_id: string` field — UUIDv4 minted by the integrator at app boot, same value as the parent session directory name and `/api/state`'s `session_uuid`. Mirrors the `app_id` shape from earlier today; together they make every manifest self-identifying about which app run produced it. Spec-only; implementation/tests pending. Companion to the lifecycle formalization in `auki-session/README.md`.

### broodsugar's claude · May 4, 08:52 HKT, 2026

Sensor Log family manifest gains a required `app_id: string` field, carrying the same identifier as the daemon's `/api/info` `app` value (e.g. `boosterapp`, `sentinel`). Applies to Sensor Log, Point Cloud Log, and Audio Log — they share the manifest shape. Mandatory addition; breaking against existing on-disk logs (acceptable under v0.x). Implementation/tests still pending.

### broodsugar's claude · May 2, 13:50 HKT, 2026

Added audio sensor support: new `SensorBody::Microphone` variant with fields `sample_rate_hz`, `channels`, `sample_format`, `channel_layout`; new `AudioLogEntry { data: bytes }` payload type with `serde_bytes` so CBOR encodes the sample buffer as a byte string (major type 2). Modelled multi-mic arrays as one sensor with `channels = N` rather than N independent sensors — right for physically-synchronized arrays sharing a clock and origin. v1 spec covers PCM only (`pcm_s16le`/`s24le`/`s32le`/`f32le`/`f64le`); compressed formats (`flac`, `opus`) extend `sample_format` when they earn it without changing the struct shape. Locked canonical bytes + locked hash (`6e0a195364866f18834d2db8e2a0699f`) for an M1 example mic-array entry. 3 new tests; auki-registry now at 21 tests.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Changelog initialized. Prior history lives in git log; this file tracks changes from this point forward.
