# Parking Lot — auki-network-browser-wasm

Open questions and blockers for the browser/WASM networking probe.

## Items

- **2026-05-19 — browser_libp2p compile blocker.** `cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p` failed while enabling rust-libp2p browser features for `wasm32-unknown-unknown`. First failing error: `The wasm32-unknown-unknown targets are not supported by default; you may need to enable the "wasm_js" configuration flag.` The transitive path is `libp2p -> libp2p-yamux -> yamux -> rand -> rand_core -> getrandom 0.3.4`. Classification: dependency/version. Next action: decide whether the wasm crate owns the `getrandom_backend="wasm_js"`/Rust flags configuration for browser feature builds or whether to reduce the initial feature set before rerunning the WebRTC/WebTransport/WebSocket probe.
- **2026-05-19 — Native browser-compatible listener.** Once the `browser_libp2p` wasm feature compiles, choose and implement the matching native SDK listener/probe fixture: WebRTC Direct first, WebTransport second, Secure WebSocket only as fallback.
