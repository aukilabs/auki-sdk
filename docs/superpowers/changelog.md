# Changelog — docs/superpowers

Append-only timeline of Superpowers design artifacts. Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

Updated the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md) to mark Task 5 complete after generated JavaScript switched to shared signaled address helpers.

### Nils's codex · May 27, HKT, 2026

Updated the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md) to mark Task 4 complete after framed and stream router support landed.

### Nils's codex · May 27, HKT, 2026

Updated the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md) to mark Task 3 complete after the transport-neutral signaled peer core landed.

### Nils's codex · May 27, HKT, 2026

Updated the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md) to mark Task 2 complete after native Discovery signaling bindings landed.

### Nils's codex · May 27, HKT, 2026

Updated the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md) to mark Task 1 complete after the network signaled address helper landed.

### Nils's codex · May 27, HKT, 2026

Added the SDK signaled WebRTC transport implementation plan under [`plans/`](plans/2026-05-27-sdk-signaled-webrtc-transport.md), sequencing reusable network/domain modules and generated binding adapters before app migration.

### Nils's codex · May 27, HKT, 2026

Added the SDK signaled WebRTC transport design under [`specs/`](specs/2026-05-27-sdk-signaled-webrtc-transport-design.md), covering SDK-owned Discovery signaling, bidirectional data-channel routing, and generated native/browser binding adapters.

### Nils's codex · May 27, HKT, 2026

Updated the Native iOS Producer Peer implementation plan under [`plans/`](plans/2026-05-27-ios-camera-streamer.md) to record completed automated verification and the remaining physical-device smoke gap.

### Nils's codex · May 27, HKT, 2026

Added the Native iOS Producer Peer implementation plan under [`plans/`](plans/2026-05-27-ios-camera-streamer.md), covering the domain auto-advertise Swift bootstrap helper, Overwatch native camera-frame decoding, and the generated-binding `AukiCameraStreamer` app.

### Nils's codex · May 27, HKT, 2026

Added the iOS camera streamer design under [`specs/`](specs/2026-05-27-ios-camera-streamer-design.md), defining a generated Swift binding app that joins an Auki cluster, logs typed camera frames, and streams them for Overwatch.

### Nils's codex · May 26, HKT, 2026

Added the Overwatch Park UI implementation plan under [`plans/`](plans/2026-05-26-overwatch-park-ui.md), covering the Park frontend source port and SDK browser/WASM replacements for the backend-facing data modules.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 10 complete after documenting the generated binding surfaces and propagating sprint/changelog updates.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 9 complete after adding generated Python, Swift, and JavaScript/Wasm smoke coverage for the generated package surfaces.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 8 complete after adding browser-safe `auki-domain` DTO validators and the generated JavaScript `AukiDomainClient` facade over the `auki-network` browser transport.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 7 complete after adding native `auki-domain` provider callbacks, catalog/resource/registry fetches, and byte-stream bindings.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 6 complete after adding native `auki-domain` cluster-control, diagnostics, membership snapshot, and clock estimate bindings.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 5 complete after adding native Discovery/app-instance binding facades and generated browser JavaScript runtime tests.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 4 complete after adding native byte-stream bindings and protobuf-byte stream smoke coverage.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 3 complete after adding two-runtime native binding smoke tests for request/response and diagnostic flows.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to record partial Phase 3 completion for the native network event and request/response binding API surface, with two-runtime smoke tests still open.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 2 complete after adding the native network runtime-control UniFFI facade.

### Nils's codex · May 25, HKT, 2026

Updated the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md) to mark Phase 1 complete after adding measurable surface inventories and marker tests.

### Nils's codex · May 25, HKT, 2026

Added the full `auki-network` and `auki-domain` bindings implementation plan under [`plans/`](plans/2026-05-25-full-network-domain-bindings.md), covering native UniFFI runtime parity, browser wasm/JavaScript facades, request/response protocols, streams, providers, and generated-language smoke tests while keeping `auki-ros-adapter` out of scope.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan under [`plans/`](plans/2026-05-24-sdk-wide-uniffi-bindings.md) to mark browser-probe interop smoke coverage complete after generated JavaScript passed against the native WebRTC Direct listener.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan under [`plans/`](plans/2026-05-24-sdk-wide-uniffi-bindings.md) to mark completed crate migrations and the JavaScript-owned browser transport split, while keeping browser-probe interop smoke coverage open.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan under [`plans/`](plans/2026-05-24-sdk-wide-uniffi-bindings.md) to record the completed generator inventory helper and all-crate binding contract test.

### Nils's codex · May 24, HKT, 2026

Updated the SDK-wide UniFFI bindings migration plan under [`plans/`](plans/2026-05-24-sdk-wide-uniffi-bindings.md) so the current pass stops at `auki-domain` and leaves `auki-ros-adapter` for a later adapter-specific decision.

### Nils's codex · May 24, HKT, 2026

Added the SDK-wide UniFFI bindings migration plan under [`plans/`](plans/2026-05-24-sdk-wide-uniffi-bindings.md), covering crate order, generated Python/Swift/JavaScript testing, and PyO3 legacy coexistence.

### Nils's codex · May 22, HKT, 2026

Added the iOS Auki network UniFFI test app implementation plan under [`plans/`](plans/2026-05-22-ios-auki-network-uniffi-test-app.md), covering generated Swift bindings from Rust crates plus browser-to-iOS message interop.

### Nils's codex · May 22, HKT, 2026

Corrected the iOS Auki network UniFFI design to center the generated-binding iOS test app: Rust crates export behavior through UniFFI, generated Swift packages are consumed by the app, and the app does not hand-write SDK networking behavior.

### Nils's codex · May 22, HKT, 2026

Added the iOS Auki network UniFFI design under [`specs/`](specs/2026-05-22-ios-auki-network-uniffi-design.md), defining Swift as an app/protobuf layer over Rust `auki-network` rather than a swift-libp2p implementation.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation artifacts so the initial migration commits only the generated Rust `auki-proto` crate; JavaScript/TypeScript, Swift, and Python protobuf outputs are generated locally under ignored `bindings/` paths.

### Nils's codex · May 22, HKT, 2026

Updated the Auki proto generation artifacts so the initial migration commits Rust, JavaScript/TypeScript, and Swift generated protobuf packages only; Python generation remains on-demand and uncommitted.

### Nils's codex · May 22, HKT, 2026

Added the Auki proto generation design and implementation plan under [`specs/`](specs/2026-05-22-auki-proto-generation-design.md) and [`plans/`](plans/2026-05-22-auki-proto-generation.md), defining generated per-platform `auki-proto` packages and the `auki-datatypes` deprecation path.

### Nils's codex · May 21, HKT, 2026

Added the `auki-uniffi-test` multiplatform bindings implementation plan under [`plans/`](plans/2026-05-21-auki-uniffi-test-multiplatform-bindings.md), sequencing the shared-core split, UniFFI native package preservation, wasm-bindgen JavaScript package generation, and setup tooling.

### Nils's codex · May 21, HKT, 2026

Refined the stream naming cleanup Superpowers artifacts so active plans/specs describe the final `CameraFrame`, `DetectionFrame`, and `Camera` vocabulary without mechanical self-renames.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup implementation plan under [`plans/`](plans/2026-05-21-stream-naming-cleanup.md), sequencing the full no-compatibility rename across datatypes, registry, network, bindings, domain, ROS adapter, docs, and Park.

### Nils's codex · May 21, HKT, 2026

Added the stream naming cleanup design under [`specs/`](specs/2026-05-21-stream-naming-cleanup-design.md), specifying the no-compatibility rename to `CameraFrame`, `DetectionFrame`, and `SensorBody::Camera`.

### Nils's codex · May 21, HKT, 2026

Updated the browser Domain adapter plan/spec under [`plans/`](plans/changelog.md) and [`specs/`](specs/changelog.md) so the browser contract cannot drift from the current SDK sensor-kind and stream-state vocabulary.

### Nils's codex · May 20, HKT, 2026

Renamed the heartbeat time-sync plan's `DomainClockSource.manager_peer_id` field to `backing_peer_id`, making the source record describe clock provenance instead of Manager role.

### Nils's codex · May 20, HKT, 2026

Replaced the proposed `DomainClockSource.domain_id` field with the existing `cluster_name` concept in the heartbeat time-sync plan; the domain clock id derives from `<cluster-name>/domain-clock`.

### Nils's codex · May 20, HKT, 2026

Made `DomainClockSource.domain_id` explicit in the heartbeat time-sync plan; the domain clock id now derives from `<domain-id>/domain-clock` instead of requiring consumers to parse `clock_id`.

### Nils's codex · May 20, HKT, 2026

Corrected the heartbeat time-sync plan so `NetworkRuntime::spawn(...)` requires `HeartbeatTimestampSource` directly as the default heartbeat timestamp path.

### Nils's codex · May 20, HKT, 2026

Walked back the heartbeat time-sync plan so `/auki/heartbeat/0.0.1` carries only sender timestamp clock identity plus NTP echo fields; the domain-clock id/hash now live in `DomainClockSource` and future TimeTransform manifests. Also refreshed Python binding paths to `bindings/python/auki-domain-py`.

### Nils's codex · May 20, HKT, 2026

Revised the heartbeat time-sync plan so NTP math, filtering, and local TimeTransform production live in `auki-time`; `ClusterManager` remains only the domain-clock source authority.

### Nils's codex · May 20, HKT, 2026

Updated the heartbeat time-sync plan to depend on the implemented `SessionClock` foundation and to describe heartbeat timestamps as sourced from `SessionClock`, not `DaemonInfo` clock fields.

### Nils's codex · May 20, HKT, 2026

Renamed the SDK timekeeping foundation plan's target crate from `auki-time-transforms` to `auki-time`, matching the crate rename and the broader timekeeping responsibility.

### Nils's codex · May 20, HKT, 2026

Updated the browser Domain peer symmetry plan under [`plans/`](plans/changelog.md) so `auki-network-browser-wasm` and `auki-domain-browser` are bindings/facades over shared Rust SDK logic rather than parallel browser implementations.

### Nils's codex · May 20, HKT, 2026

Reframed the browser Domain WebRTC implementation plan under [`plans/`](plans/changelog.md) so browser peers are full role-symmetric Domain peers, including Manager eligibility, with reachability handled as SDK transport state.

### Nils's codex · May 19, HKT, 2026

Added the SDK timekeeping foundation implementation plan under [`plans/`](plans/2026-05-19-sdk-timekeeping-foundation.md), making heartbeat time sync depend on a reusable `SessionClock` primitive first.

### Nils's codex · May 19, HKT, 2026

Added the domain-clock heartbeat time-sync implementation plan under [`plans/`](plans/2026-05-19-domain-clock-heartbeat-time-sync.md), scoping Manager-authored domain-clock transforms over `/auki/heartbeat/0.0.1`.

### Nils's codex · May 19, HKT, 2026

Added the native pointcloud SDK refactor design under [`specs/`](specs/changelog.md), documenting the approved breaking pointcloud contract for implementation planning.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain WebRTC join implementation plan under [`plans/`](plans/changelog.md), following the passing browser-to-native probe with production Manager WebRTC advertisement and browser join/info wiring.

### Nils's codex · May 19, HKT, 2026

Added the browser WebRTC probe stream implementation plan under [`plans/`](plans/changelog.md), sequencing the first browser-to-native SDK-owned protocol stream after the wasm libp2p feature compile probe passed.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport compile-probe implementation plan under [`plans/`](plans/changelog.md), sequencing the first measurable SDK browser networking spike before native dial work.

### Nils's codex · May 19, HKT, 2026

Added the wasm libp2p browser transport spike spec under [`specs/`](specs/changelog.md), turning the browser transport question into a rust-libp2p Wasm proof plan before Domain join/audio.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter implementation plan under [`plans/`](plans/changelog.md), keeping first-tranche SDK package work separate from the later browser transport/audio plan.

### Nils's codex · May 19, HKT, 2026

Added the browser Domain peer adapter spec under [`specs/`](specs/changelog.md), documenting the SDK-side package and transport work needed for Park's browser-peer Milestone 0.
