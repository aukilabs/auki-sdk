# Changelog — auki-network-browser-wasm

Append-only timeline of changes for the browser/WASM networking probe. Latest entry on top.

---

### Nils's codex · May 22, HKT, 2026

`BrowserDomainSession` now uses native info/sensors catalog parity between browser peers. After `/auki/join/0.0.1` membership, the browser full peer keeps inbound `/auki/info/0.0.1` and `/auki/sensors/0.0.1` handlers open, fetches remote browser peer info/catalogs through the same native protocols, and fills remote participant sensors from `/auki/sensors/0.0.1` instead of membership placeholders. The successful path no longer carries a browser-session sender or opens `/auki/browser-session/0.0.1`; media intent methods return `ok` after a joined snapshot exists.

Tests: `cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p`, `wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p`, `node crates/auki-network-browser-wasm/scripts/smoke_browser_full_peer_probe.mjs`.

### Nils's codex · May 22, HKT, 2026

`BrowserDomainSession.joinDomain()` now keeps a live browser session control-plane stream after the join handshake. The wasm session exposes participant metadata/sensor declaration observers, publishes local media presence intent to the Manager, consumes pushed browser roster snapshots, and returns `ok` for mic publish/listen intent after join. The two-browser Park acceptance smoke now passes: both browser peers see each other and media publish/listen calls succeed.

Tests: `cargo test -p auki-network-browser-wasm local_browser_participant --features browser_libp2p -- --nocapture`, `cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p`, `wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p`, `node crates/auki-network-browser-wasm/scripts/smoke_park_two_browser_acceptance.mjs ... http://127.0.0.1:7880`.

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

### Nils's codex · May 21, HKT, 2026

Added browser `/auki/join/0.0.1` transport support behind `browser_libp2p`. The wasm package can now fetch Discovery from `BrowserDomainSession.joinDomain()`, choose the Manager's WebRTC multiaddr, open the shared join protocol over `libp2p-stream`, and return `ok` or a typed browser Domain error. Added low-level and class-level Playwright smoke scripts.

### Nils's codex · May 21, HKT, 2026

Added a Park end-to-end browser join smoke script. It opens the live Park web-peer page, verifies Park's bundled SDK adapter installs `window.aukiBrowserPeer`, serves a CORS-enabled fake Discovery directory, and asserts the page-created browser peer can join a native WebRTC join listener.

### Nils's codex · May 21, HKT, 2026

`BrowserDomainSession.joinDomain()` now returns join metadata on success: domain name, Manager peer id, and Manager-supplied membership JSON. The Park end-to-end smoke script now checks that Park emits a visible joined snapshot, not just that join returned `ok`.

### Nils's codex · May 21, HKT, 2026

Improved browser join diagnostics by extracting `.message` from JavaScript `Error`/`TypeError` objects crossing the wasm boundary. Browser fetch failures now surface their real message instead of collapsing to `unknown JavaScript error`.
