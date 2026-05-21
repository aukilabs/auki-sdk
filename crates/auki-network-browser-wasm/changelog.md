# Changelog — auki-network-browser-wasm

Append-only timeline of changes for the browser/WASM networking probe. Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

Added the browser-to-native WebRTC probe smoke harness. `scripts/browser_probe_smoke.html` loads `pkg-web` and calls `dialBrowserProbe`; `scripts/smoke_browser_probe.mjs` serves the crate locally, launches Chrome through `playwright-core`, and asserts the `/auki/browser-probe/0.0.1` payload round-trip. Verified against the native `browser_probe_listener`: `ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar`.

### Nils's codex · May 19, HKT, 2026

Added the browser-side `dialBrowserProbe(seed, address, payload)` wasm export behind `browser_libp2p`. It builds a browser libp2p peer from the SDK wallet-derived identity, parses the native `/p2p/<peer-id>` multiaddr, opens `/auki/browser-probe/0.0.1` through `libp2p-webrtc-websys`, and returns a UI-friendly success/error result.

### Nils's codex · May 19, HKT, 2026

Enabled the `getrandom` 0.3 `wasm_js` feature required by the libp2p/yamux browser build path. The `browser_libp2p` feature now compiles for `wasm32-unknown-unknown`, and `wasm-pack --target web --features browser_libp2p` produces the browser package.

### Nils's codex · May 19, HKT, 2026

Ran the rust-libp2p browser feature compile probe for `wasm32-unknown-unknown`. Outcome: blocked, with the exact first blocker recorded in `parking_lot.md`. The crate still keeps `auki-domain-browser` fail-closed for Domain join until a browser peer can dial a native SDK probe.

### Nils's codex · May 19, HKT, 2026

Added a Node import smoke script for the wasm package. `wasm-pack --target nodejs` output can be imported from JavaScript and reproduces the locked PeerId vector through the wasm boundary.

### Nils's codex · May 19, HKT, 2026

Added the canonical `peerIdFromSeed(seed)` wasm export. The Rust test pins seed `[3u8; 32]` to the SDK's locked libp2p PeerId vector, and wrong-length seeds return a typed error instead of panicking.

### Nils's codex · May 19, HKT, 2026

Created the `auki-network-browser-wasm` crate scaffold for the rust-libp2p browser transport spike. The crate starts as an importable wasm shell; identity and libp2p browser feature probes follow in separate commits.
