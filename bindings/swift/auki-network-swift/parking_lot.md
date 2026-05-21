# Parking lot — auki-network-swift

Open questions specific to the Swift/UniFFI bindings. Cross-cutting questions about the underlying primitives belong in [`auki-network/parking_lot.md`](../../../crates/auki-network/parking_lot.md).

---

## Async-shaped Swift API vs the `-py` sync precedent

`auki-network-py` deliberately keeps a **sync** public API and owns the tokio runtime internally, because Python callers live in a GIL world.

This crate exports **async** methods (`#[uniffi::export(async_runtime = "tokio")]`). Rationale: Swift has first-class `async`/`await`, and on iOS the calling thread is frequently the main thread — a blocking Discovery HTTP call there would jank the UI / risk watchdog kills. So the idiomatic and safe Swift shape is async.

This is a deliberate divergence from the sibling precedent, made to fit the language, not an oversight. Confirmed acceptable for v0 iosapp. Reversing it later is a breaking API change.

## Where do generated Swift bindings + the XCFramework live?

`build-xcframework.sh` produces `auki_network_swiftFFI` (the generated `.swift` + a `.xcframework`). Open: are these build artifacts (gitignored, built by iosapp's CI / a release job) or committed into this repo (a `swift/` dir, or a separate `aukilabs/auki-sdk-swift` SwiftPM-package repo for `Package.swift` consumption)? The `-py` crates ship as `maturin` wheels and do **not** commit generated code; the analogous Swift answer (committed SwiftPM package vs. pip-equivalent build step) is a distribution decision the SDK owners should make. Not blocking v0 (host build/test and XCFramework generation are green).

## `with_http` not exposed — TLS / proxy / timeout knob shape

The Rust `DiscoveryClient::with_http(base_url, reqwest::Client)` lets callers configure custom timeouts, proxies, and custom TLS roots. The Swift binding uses `reqwest::Client::new()` defaults only — `with_http` is not surfaced.

This is fine for v0 iosapp (Auki cluster on a known LAN), but the right Swift-friendly FFI shape isn't obvious: do we expose a string proxy URL? A CA-bundle as `Data`? A timeout in milliseconds? Best to wait for a concrete iosapp deployment that actually needs the knob, so we design once against a real requirement rather than guess. `auki-network-py` has the same standing item.

## Heartbeat-detail variants not forwarded

`SwiftPeerLivenessEvent` is a 3-variant v0 subset (Connected, Disconnected, HeartbeatAlive). The drain task in `spawn_for_swift` drops the two heartbeat-detail upstream variants (`HeartbeatReceived` and `HeartbeatNtpSampleObserved`) rather than forwarding them. Widen the enum if iosapp needs heartbeat-timing observation (e.g. for domain-clock sync on the Swift side).

## `uniffi::custom_type!` reachability across binding crates

`PeerId` and `Multiaddr` are registered as UniFFI custom types in this crate via `uniffi::custom_type!` with the `remote` keyword. `auki-domain-swift` (PR C) depends on this crate and will also need `PeerId`/`Multiaddr` on its FFI seam. Confirm the registrations cross the crate boundary without redeclaration; if UniFFI 0.31 requires each binding crate to re-register independently, factor the custom-type registrations into a shared helper crate rather than duplicating.

## Single shared tokio runtime

Each binding crate (`auki-identity-swift`, `auki-network-swift`, and the upcoming `auki-domain-swift`) drives its own tokio runtime. For v0 this is fine. Consolidate into a single shared runtime if profiling shows thread-pool or shutdown-ordering pain.

## Two-call SwiftStreamProvider protocol

The `SwiftStreamProvider` callback interface uses a two-call protocol: (1) `dispatch_decision(peer_id, request_bytes) -> SwiftStreamDecision`; (2) on each Accept variant, the matching `*_source(peer_id, request_bytes) -> Box<dyn Swift*Source>` call. The Swift implementation must keep both calls consistent for the same `(peer_id, request_bytes)` pair — if `dispatch_decision` returns `AcceptAudio`, the runtime will call `audio_source` with the same arguments and expects a valid source back.

This split is forced by a UniFFI 0.31 constraint (trait objects cannot be fields inside `uniffi::Enum` variants). Revisit if a single-call shape becomes possible after a UniFFI version bump that adds trait-object-in-enum support.
