# Changelog — docs/superpowers/plans

Append-only timeline of implementation plan changes. Latest entry on top.

---

### Nils's codex · May 20, HKT, 2026

Revised the browser Domain peer symmetry plan to prevent SDK drift: browser support is now framed as wasm-targeting the same `auki-network` and `auki-domain` Rust logic, with TypeScript kept as facade glue and runtime-specific code limited to concrete browser constraints.

### Nils's codex · May 20, HKT, 2026

Rewrote the browser Domain WebRTC follow-up plan around peer symmetry: any runtime can create, join, manage, publish, fail, and recover through the same Domain role rules; transport reachability is now treated as an SDK routing concern rather than a role distinction.

### Nils's codex · May 19, HKT, 2026

Created the browser Domain WebRTC join implementation plan, sequencing production native WebRTC Direct Manager advertisement, browser wasm raw SDK substreams, `auki-domain-browser` join wiring, and the next Chrome smoke test after the probe stream passed.

### Nils's codex · May 19, HKT, 2026

Created the browser WebRTC probe stream implementation plan, scoped to a native `auki-network` WebRTC Direct listener, a browser wasm `dialBrowserProbe` export, and a Playwright smoke test that proves one SDK-owned named protocol stream.

### Nils's codex · May 19, HKT, 2026

Created the wasm libp2p browser transport compile-probe implementation plan, scoped to `auki-network-browser-wasm` scaffolding, canonical PeerId wasm export, JS import smoke testing, and a rust-libp2p browser feature compile check before native dial work.

### Nils's codex · May 19, HKT, 2026

Created the browser Domain peer adapter first-tranche implementation plan, scoped to package scaffold, Park-compatible contract, identity/Discovery seams, idle snapshots, and explicit transport-unavailable behavior before real browser networking.
