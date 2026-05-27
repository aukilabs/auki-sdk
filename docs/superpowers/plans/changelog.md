# Changelog — docs/superpowers/plans

Append-only timeline of implementation plan changes. Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

Marked Task 3 of the SDK signaled WebRTC transport plan complete after adding the transport-neutral signaled peer core and no-default verification fix in `auki-network`.

### Nils's codex · May 27, HKT, 2026

Marked Task 2 of the SDK signaled WebRTC transport plan complete after exposing Discovery `/signals` send/poll through `auki-network` native bindings.

### Nils's codex · May 27, HKT, 2026

Marked Task 1 of the SDK signaled WebRTC transport plan complete after adding canonical signaled address helpers and wasm exports in `auki-network`.

### Nils's codex · May 27, HKT, 2026

Added the SDK signaled WebRTC transport implementation plan, sequencing SDK-owned address helpers, Discovery signal bindings, transport-neutral peer routing, generated JavaScript/Swift adapters, the signaled Domain facade, and iOS/Overwatch migration.

### Nils's codex · May 27, HKT, 2026

Recorded automated verification status for the Native iOS Producer Peer plan: Rust binding surface, Swift binding generation/smoke, JavaScript protobuf generation, Overwatch tests/build, and iOS simulator tests passed; physical iPhone smoke remains pending.

### Nils's codex · May 27, HKT, 2026

Created the Native iOS Producer Peer implementation plan, sequencing the `auki-domain` auto-advertise Swift bootstrap helper, Overwatch native `CameraFrame` decode path, and the `AukiCameraStreamer` iOS app that logs and streams camera frames.

### Nils's codex · May 26, HKT, 2026

Created the Overwatch Park UI implementation plan, sequencing a source-level Park frontend port into `examples/overwatch` plus SDK browser/WASM replacements for Park's backend data, registry, and stream modules while preserving the no app `/api/*` invariant.

### Nils's codex · May 25, HKT, 2026

Marked Phase 10 of the full `auki-network` and `auki-domain` bindings plan complete after updating README/source documentation, sprint notes, changelog propagation, and the final verification command list.

### Nils's codex · May 25, HKT, 2026

Marked Phase 9 of the full `auki-network` and `auki-domain` bindings plan complete after adding generated Python, Swift, and JavaScript/Wasm smoke tests for the generated package surfaces.

### Nils's codex · May 25, HKT, 2026

Marked Phase 8 of the full `auki-network` and `auki-domain` bindings plan complete after adding browser-safe `auki-domain` DTO validators, the generated JavaScript `AukiDomainClient` facade over `auki-network` browser request framing, and generated package tests.

### Nils's codex · May 25, HKT, 2026

Marked Phase 7 of the full `auki-network` and `auki-domain` bindings plan complete after adding native `auki-domain` catalog/resource/registry providers, catalog and registry fetches, byte-stream producer/consumer controls, and regenerated Python/Swift binding surfaces.

### Nils's codex · May 25, HKT, 2026

Marked Phase 6 of the full `auki-network` and `auki-domain` bindings plan complete after adding the native domain cluster-control, diagnostics, membership snapshot, clock estimate, and regenerated Python binding surfaces.

### Nils's codex · May 25, HKT, 2026

Marked Phase 5 of the full `auki-network` and `auki-domain` bindings plan complete after adding the native Discovery/app-instance facades, browser wasm protocol helpers, JavaScript-owned libp2p peer methods, and generated package tests.

### Nils's codex · May 25, HKT, 2026

Marked Phase 4 of the full `auki-network` and `auki-domain` bindings plan complete after adding native byte-stream binding records, `AukiStreamSubscription`, host-driven stream provider events, and protobuf-byte stream smoke tests.

### Nils's codex · May 25, HKT, 2026

Marked Phase 3 of the full `auki-network` and `auki-domain` bindings plan complete after adding two-runtime native binding smoke tests for join, participant-info, catalog, registry, and diagnostic flows.

### Nils's codex · May 25, HKT, 2026

Marked the API-surface tasks in Phase 3 of the full `auki-network` and `auki-domain` bindings plan complete after adding native event records, responder-token registries, drain methods, JSON request wrappers, and response methods; two-runtime smoke tests remain open.

### Nils's codex · May 25, HKT, 2026

Marked Phase 2 of the full `auki-network` and `auki-domain` bindings plan complete after adding the native `AukiNetworkRuntime` runtime-control UniFFI facade and active binding-surface tests.

### Nils's codex · May 25, HKT, 2026

Marked Phase 1 of the full `auki-network` and `auki-domain` bindings plan complete after adding surface inventories, marker tests, and the root binding-surface checker.

### Nils's codex · May 25, HKT, 2026

Created the full `auki-network` and `auki-domain` bindings implementation plan, defining binding-safe parity for native UniFFI, browser wasm/JavaScript, request/response protocols, streams, providers, generated-language smoke tests, and the explicit exclusion of `auki-ros-adapter`.

### Nils's codex · May 24, HKT, 2026

Marked the SDK-wide UniFFI plan's browser-probe smoke coverage item complete after the generated JavaScript package successfully dialed the native WebRTC Direct `/auki/browser-probe/0.0.1` listener.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI plan status to reflect completed crate migrations, the legacy PyO3 out-of-scope decision, and the JavaScript-owned browser transport split while leaving real browser-probe interop smoke coverage open.

### Nils's codex · May 24, HKT, 2026

Marked the SDK-wide UniFFI plan's generator inventory helper and all-crate binding contract test items complete after widening `scripts/test-binding-generator-contract.sh`.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan to stop the current pass at `auki-domain` and defer `auki-ros-adapter` to a separate adapter-specific binding decision.

### Nils's codex · May 24, HKT, 2026

Created the SDK-wide UniFFI bindings migration plan, sequencing crate-owned Python/Swift UniFFI and JavaScript/WASM adoption while treating existing PyO3 packages as legacy compatibility surfaces.

### Nils's codex · May 22, HKT, 2026

Created the iOS Auki network UniFFI test app implementation plan, sequencing binding generator feature selection, Rust `auki-network` message-node exports, generated Swift packages, the iOS host app, and browser-to-iOS `/auki/message/0.0.1` smoke testing.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation implementation plan so only the generated Rust `auki-proto` crate is committed; JavaScript/TypeScript, Swift, and Python generation scripts create ignored local outputs under `bindings/`.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation implementation plan to skip the Python package rename and keep generated Python protobuf output uncommitted until a separate packaging decision.

### Nils's codex · May 22, HKT, 2026

Created the Auki proto generation implementation plan, sequencing root schema relocation, generated Rust `auki-proto`, `auki-datatypes` shim deprecation, Rust consumer migration, and JavaScript/Swift/Python protobuf generation.

### Nils's codex · May 21, HKT, 2026

Created the `auki-uniffi-test` multiplatform bindings implementation plan, covering `install-toolchain`, shared core extraction, UniFFI preservation for Swift/Python, wasm-bindgen JavaScript generation, and final verification.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain peer adapter first-tranche implementation plan so its TypeScript contract uses the current SDK sensor-kind and stream-state vocabulary Park expects.

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
