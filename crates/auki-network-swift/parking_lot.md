# Parking lot — auki-network-swift

Open questions specific to the Swift/UniFFI bindings. Cross-cutting questions about the underlying primitives belong in [`auki-network/parking_lot.md`](../auki-network/parking_lot.md).

---

## Async-shaped Swift API vs the `-py` sync precedent _(flagged for human confirmation)_

`auki-network-py` deliberately keeps a **sync** public API and owns the tokio runtime internally ("Out Of Scope: async public methods" in its `src/sprint.md`), because Python callers live in a GIL world.

This crate instead exports **async** methods (`#[uniffi::export(async_runtime = "tokio")]`). Rationale: Swift has first-class `async`/`await`, and on iOS the calling thread is frequently the main thread — a blocking Discovery HTTP call there would jank the UI / risk watchdog kills. So the idiomatic and safe Swift shape is async.

This is a deliberate divergence from the sibling precedent, made to fit the language, not an oversight. Surfacing per CLAUDE.md "do not resolve ambiguities unilaterally": **confirm** this is the wanted shape (vs. a sync façade that internally `block_on`s, matching `-py` 1:1). Lean: keep async — reversing it later is a breaking API change for iosapp.

## Where do generated Swift bindings + the XCFramework live?

`build-xcframework.sh` produces `auki_network_swiftFFI` (the generated `.swift` + a `.xcframework`). Open: are these build artifacts (gitignored, built by iosapp's CI / a release job) or committed into this repo (a `swift/` dir, or a separate `aukilabs/auki-sdk-swift` SwiftPM-package repo for `Package.swift` consumption)? The `-py` crates ship as `maturin` wheels and do **not** commit generated code; the analogous Swift answer (committed SwiftPM package vs. pip-equivalent build step) is a distribution decision the SDK owners should make. Not blocking Stage 1 (host build/test is green without it).

## `with_http` not exposed — TLS / proxy / timeout knob shape

The Rust `DiscoveryClient::with_http(base_url, reqwest::Client)` lets callers configure custom timeouts, proxies, and custom TLS roots. The Swift binding at Stage 1 uses `reqwest::Client::new()` defaults only — `with_http` is not surfaced.

This is fine for v0 iosapp (Auki cluster on a known LAN), but the right Swift-friendly FFI shape isn't obvious: do we expose a string proxy URL? A CA-bundle as `Data`? A timeout in milliseconds? Best to wait for a concrete iosapp deployment that actually needs the knob, so we design once against a real requirement rather than guess. `auki-network-py` has the same standing item.

## Stream payload parity with `auki-network`

When Stage 2 lands the stream surface, keep payload parity: when Rust adds a `StreamDispatch` variant, the Swift side (and the swift-protobuf `.proto` consumption) must follow in the same release — same standing rule `auki-network-py`'s parking lot records for itself.
