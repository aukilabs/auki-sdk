# Parking Lot — auki-network-browser-wasm

Open questions and blockers for the browser/WASM networking probe.

## Items

- **2026-05-19 — Native browser-compatible listener.** Now that the `browser_libp2p` wasm feature compiles, choose and implement the matching native SDK listener/probe fixture: WebRTC Direct first, WebTransport second, Secure WebSocket only as fallback.
