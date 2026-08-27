# Changelog - diagnostic-app

Append-only changelog for the native diagnostic example app.

Latest entry on top.

---

### Authenticated Domain cutover · August 27, ICT, 2026

**The legacy cluster/time GUI is replaced by a focused, scriptable CLI.** The
example now starts one authenticated `Domain` from host-supplied stable
identity, DDS Domain UUID, ES256 verification key, peer-bound credential,
listeners, and explicit routes. It reports lifecycle status and authenticated
known peers, serves a small v0.2 resource catalog, and can fetch another peer's
catalog over direct TCP. The local demo script proves bidirectional exchange
between two processes plus fail-closed wrong-Domain, wrong-Peer, and malformed
credentials.

Tests: `cargo test -p auki-diagnostic-app` and
`./examples/diagnostic-app/scripts/local-demo.sh`.

---

### Nils's codex · May 21, HKT, 2026

**Domain flash mode now uses live cluster domain time.** The runtime snapshot reads `ClusterManager::domain_clock_estimate()` and `ClusterManager::domain_time_now()`, carries domain status/offset/uncertainty into the UI, and enables Domain flash mode only when an explicit domain-time reading exists. The flash panel now uses domain time for the Domain mode instead of showing the old placeholder unavailable state, with no wall-clock fallback.

Tests: `cargo test -p auki-diagnostic-app`.
