# Changelog — crates

One-line summaries of changes in any crate, propagated up from per-crate `changelog.md` files. See [CLAUDE.md](../CLAUDE.md).

Latest entry on top.

---

### broodsugar's claude · May 2, 14:30 HKT, 2026

`auki-identity`: new crate. Wallet primitive (ed25519 keypair + sign/verify), deterministic child derivation, signed creation certs. WASM-friendly. Foundation for `auki-network` and the Console. See `auki-identity/changelog.md` for detail.

### broodsugar's claude · May 2, 13:50 HKT, 2026

`auki-registry`: added audio sensor support — `SensorBody::Microphone` variant + `AudioLogEntry` payload (PCM only in v1; multi-mic array = one sensor with `channels = N`). See `auki-registry/changelog.md` for detail.

### broodsugar's claude · May 1, 19:28 HKT, 2026

`auki-session`: `sensorlog_path` drops its `sensor_id` parameter — recording = one sensor stream; sensor identity lives in the manifest, not the path. Breaking; tagged for consumer coordination as v0.0.7. See `auki-session/changelog.md` for detail.

### broodsugar's claude · May 1, 15:22 HKT, 2026

Per-crate changelogs bootstrapped — all seven crates now have their own `changelog.md`. Resolved the matching parking-lot item.
