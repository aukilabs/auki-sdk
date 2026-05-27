# Changelog — docs

Append-only timeline of documentation changes under `docs/`. Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

Updated the Superpowers SDK signaled WebRTC transport plan to mark the native Discovery signaling bindings task complete.

### Nils's codex · May 27, HKT, 2026

Updated the Superpowers SDK signaled WebRTC transport plan to mark the signaled address helper task complete.

### Nils's codex · May 27, HKT, 2026

Added the Superpowers implementation plan for SDK-owned signaled WebRTC transport across `auki-network`, `auki-domain`, generated bindings, iOS, and Overwatch.

### Nils's codex · May 27, HKT, 2026

Added the Superpowers design for SDK-owned signaled WebRTC transport across native Swift and browser bindings.

### Nils's codex · May 27, HKT, 2026

Updated the Superpowers Native iOS Producer Peer implementation plan to record passing automated verification and the outstanding physical-device smoke requirement.

### Nils's codex · May 27, HKT, 2026

Added the Superpowers implementation plan for the Native iOS Producer Peer, sequencing the generated Swift binding app, domain auto-advertise bootstrap, camera logging/streaming, and Overwatch native camera-frame preview compatibility.

### Nils's codex · May 27, HKT, 2026

Added the Superpowers iOS camera streamer design for a generated Swift binding app that joins an Auki cluster, logs typed camera frames, and streams them to Overwatch.

### Nils's codex · May 26, HKT, 2026

Added the Superpowers implementation plan for porting Park's frontend into Overwatch and replacing Park's backend-facing data modules with generated SDK browser/WASM bindings.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 10 complete after documenting generated native/browser binding surfaces and propagating sprint/changelog updates.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 9 complete after adding generated Python, Swift, and JavaScript/Wasm smoke coverage for `auki-network` and `auki-domain`.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 8 complete after adding browser-safe `auki-domain` DTO validators and the generated JavaScript domain client facade over the `auki-network` browser transport.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 7 complete after adding native `auki-domain` provider callbacks, catalog/resource/registry fetches, and byte-stream binding controls.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 6 complete after adding native `auki-domain` cluster-control, diagnostics, membership snapshot, and clock estimate bindings.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 5 complete after adding native `auki-network` Discovery/app-instance bindings and generated browser JavaScript runtime tests.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 4 complete after adding native `auki-network` byte-stream binding APIs and protobuf-byte stream smoke tests.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 3 complete after adding two-runtime native binding smoke tests for `auki-network` request/response and diagnostic flows.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to record partial Phase 3 completion for native `auki-network` event draining, responder-token registries, JSON request wrappers, and response methods.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 2 complete after adding the native `auki-network` runtime-control UniFFI facade.

### Nils's codex · May 25, HKT, 2026

Updated the Superpowers full network/domain binding plan to mark Phase 1 complete after adding measurable binding surface inventories, marker tests, and the checker script.

### Nils's codex · May 25, HKT, 2026

Added the Superpowers implementation plan for fully exposing `auki-network` and `auki-domain` through binding-safe native UniFFI and browser wasm/JavaScript surfaces, with generated-language smoke tests and `auki-ros-adapter` excluded from this pass.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan status to mark generated JavaScript browser-probe smoke coverage complete.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan status to mark completed crate migrations and the JavaScript-owned browser transport split, while keeping browser-probe interop smoke coverage open.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan to record the completed generator inventory helper and all-crate binding contract test.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan to defer `auki-ros-adapter` out of the current pass and treat it as a later adapter-specific binding decision.

### Nils's codex · May 24, HKT, 2026

Added the Superpowers implementation plan for migrating SDK crates to the crate-owned UniFFI Python/Swift and wasm-bindgen JavaScript binding standard, with existing PyO3 wrappers treated as legacy compatibility surfaces.

### Nils's codex · May 22, HKT, 2026

Added the Superpowers implementation plan for the iOS Auki network UniFFI test app, sequencing generated Swift binding consumption, Rust-owned `auki-network` messaging, and browser-to-iOS `/auki/message/0.0.1` smoke testing.

### Nils's codex · May 22, HKT, 2026

Corrected the Superpowers iOS Auki network UniFFI design so the planned artifact is an iOS test app that imports generated Swift bindings from the Rust crates rather than a hand-written Swift networking layer.

### Nils's codex · May 22, HKT, 2026

Added the Superpowers iOS Auki network UniFFI design, capturing the browser-to-iOS `/auki/message/0.0.1` milestone and the rule that native Swift clients reuse Rust `auki-network` through UniFFI instead of swift-libp2p.

### Nils's codex · May 22, HKT, 2026

Updated the Superpowers Auki proto generation docs so only the generated Rust `auki-proto` crate is committed; JavaScript/TypeScript, Swift, and Python protobuf outputs live as ignored generated artifacts under `bindings/`.

### Nils's codex · May 22, HKT, 2026

Updated the Superpowers Auki proto generation plan to skip committed Python protobuf output for now; Python generation remains an on-demand artifact outside the initial `auki-proto` migration.

### Nils's codex · May 22, HKT, 2026

Added the Superpowers design and implementation plan for replacing `auki-datatypes` with generated per-platform `auki-proto` packages sourced from root `proto/auki` schemas.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers implementation plan for turning `auki-uniffi-test` into a shared-core multiplatform binding proving crate.

### Nils's codex · May 21, HKT, 2026

Refined the Superpowers stream naming cleanup docs after implementation so they describe the final vocabulary directly.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers implementation plan for the SDK-wide stream naming cleanup.

### Nils's codex · May 21, HKT, 2026

Added the Superpowers stream naming cleanup design spec for the SDK-wide full rename of camera, detection, and camera registry vocabulary.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter planning docs so Park and the browser SDK package use the same current SDK vocabulary: `audio`, `camera`, `point_cloud`, `joint_encoders`, `detection`, plus UI stream states `declined` and `error`.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to use `DomainClockSource.backing_peer_id` for domain-clock source provenance instead of naming the field after the Manager role.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to use `DomainClockSource.cluster_name` instead of introducing a separate domain id concept.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so `DomainClockSource` carries an explicit `domain_id` alongside the derived domain clock id.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so `NetworkRuntime::spawn(...)` requires SDK-owned heartbeat timestamps directly as part of the default runtime contract.

### Nils's codex · May 20, HKT, 2026

Walked back the heartbeat time-sync planning docs so heartbeat frames stay sender-clock-only; domain-clock identity remains in `DomainClockSource` and future TimeTransform manifests.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs to keep NTP estimation in `auki-time` and limit `ClusterManager` to domain-clock source authority.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync planning docs so future heartbeat timestamp wiring consumes `SessionClock` as the single source of clock identity and readings.

### Nils's codex · May 20, HKT, 2026

Updated Superpowers timekeeping docs to target the renamed `auki-time` crate instead of `auki-time-transforms`.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain plan to prevent browser/native SDK drift: browser crates are now bindings/facades over shared Rust `auki-network` and `auki-domain` logic, with runtime-specific code limited to concrete wasm/browser constraints.

### Nils's codex · May 20, HKT, 2026

Rewrote the browser Domain WebRTC plan around true peer symmetry: browser peers can be Managers, Discovery records PeerIds rather than platform classes, and reachability is an SDK transport concern.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers SDK timekeeping foundation plan and marked heartbeat time sync as dependent on a reusable `SessionClock` primitive.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers implementation plan for syncing cluster peers to a Manager-authored domain clock over heartbeat-carried TimeTransform samples.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers native pointcloud design spec and propagated the docs-level changelog chain for the SDK pointcloud refactor planning artifact.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain WebRTC join implementation plan after the browser probe smoke passed, covering production Manager WebRTC advertisement, browser wasm raw SDK substreams, and `auki-domain-browser` join wiring.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser WebRTC probe stream implementation plan and propagated the docs-level changelog chain for the native listener plus browser wasm dial proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport compile-probe implementation plan and propagated the docs-level changelog chain for the first SDK browser networking spike slice.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers wasm libp2p browser transport spike spec and propagated the docs-level changelog chain for the SDK-owned browser networking proof.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter implementation plan and propagated the docs-level changelog chain for the first SDK package tranche.

### Nils's codex · May 19, HKT, 2026

Added the Superpowers browser Domain peer adapter design spec and propagated the docs-level changelog chain for the SDK package Park needs to load real browser peers.
