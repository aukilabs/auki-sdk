# Changelog - diagnostic-app

Append-only changelog for the native diagnostic example app.

Latest entry on top.

---

### Nils's codex · May 21, HKT, 2026

**Domain flash mode now uses live cluster domain time.** The runtime snapshot reads `ClusterManager::domain_clock_estimate()` and `ClusterManager::domain_time_now()`, carries domain status/offset/uncertainty into the UI, and enables Domain flash mode only when an explicit domain-time reading exists. The flash panel now uses domain time for the Domain mode instead of showing the old placeholder unavailable state, with no wall-clock fallback.

Tests: `cargo test -p auki-diagnostic-app`.
