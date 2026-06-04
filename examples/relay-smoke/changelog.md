# Changelog - relay-smoke

Append-only changelog for the relay-backed libp2p decision-gate smoke harness.

Latest entry on top.

---

### Nils's codex · May 28, HKT, 2026

**Relay-backed libp2p decision-gate smoke harness added.** The example now has a
Rust native target smoke that reserves a relay circuit through `auki-network`
and a js-libp2p browser-side dialer that only accepts browser-usable `/ws` or
`/wss` relay paths before `/p2p-circuit`.

Tests: `cargo check -p auki-network --features swarm --example relay_native_target_smoke`; `npm install --prefix examples/relay-smoke`; red-gate `cargo run -p auki-network --features swarm --example relay_native_target_smoke` plus `node examples/relay-smoke/browser-smoke.mjs`.
