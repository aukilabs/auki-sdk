# SDK-wide UniFFI Bindings Migration Plan

> **For agentic workers:** Use the existing `auki-uniffi-test` standard as the template. The current PyO3 packages under `bindings/python/auki-*-py` are legacy compatibility surfaces for this migration and should not block the new crate-owned UniFFI Python path.

**Goal:** Convert SDK Rust crates to the repo's multiplatform binding standard: crate-owned UniFFI bindings for Python and Swift, plus wasm-bindgen bindings for JavaScript/WebAssembly where the crate has a web-safe surface.

**Architecture:** Product behavior stays binding-free in `core.rs`. Native binding adapters live in `ffi.rs` behind the `uniffi` feature. Browser/JavaScript adapters live in `wasm.rs` behind the `wasm` feature. Each crate owns its binding policy in `bindings.toml` and owns language package assets under `crates/<crate>/bindings/`.

**Tech Stack:** Rust 2024, UniFFI 0.31, wasm-bindgen, wasm-pack, SwiftPM/XCFramework generation on macOS, Python package generation through the generic repo binding generator, JavaScript ESM packages with smoke tests.

---

## Binding Standard

Every migrated crate should converge on this shape:

- `src/core.rs` for shared Rust behavior with no UniFFI, wasm-bindgen, Python, Swift, JavaScript, Tokio, or browser-specific assumptions unless those are part of the crate's real core.
- `src/ffi.rs` for the UniFFI API used by Python and Swift generation.
- `src/wasm.rs` for the wasm-bindgen API used by JavaScript/WebAssembly generation.
- `src/lib.rs` as feature-gated module wiring only.
- `src/bin/uniffi-bindgen.rs` with `uniffi::uniffi_bindgen_main()`.
- `crate-type = ["staticlib", "cdylib", "rlib"]`.
- `default = ["uniffi"]`, `cli = ["uniffi", "uniffi/cli"]`, and `wasm = [...]` features.
- Native-only dependencies under `target.'cfg(not(target_arch = "wasm32"))'.dependencies`.
- Generator dependencies optional and feature-gated.
- `bindings.toml` defining enabled Python, Swift, and JavaScript outputs.
- crate-owned templates and smoke tests under `crates/<crate>/bindings/{python,swift,javascript}/`.
- Rust dependents that do not need generated bindings should use `default-features = false`.

Use `auki-uniffi-test` as the mechanical reference and `auki-identity` / `auki-network` as production examples already partly migrated.

## Migration Order

### Phase 0: Harness Hardening

- [ ] Add a generator contract test that can validate every UniFFI-enabled crate, not only `auki-uniffi-test`.
- [ ] Add a repo-local helper that lists crates with `bindings.toml`, enabled languages, features, and missing package assets.
- [ ] Keep PyO3 packages buildable during transition, but document them as legacy surfaces in binding docs once their UniFFI replacements exist.

### Phase 1: Foundation Crates

- [ ] `auki-hash`
- [ ] `auki-jcs`
- [ ] `auki-layout`
- [ ] Verify `auki-identity` fully matches the standard.

These are the lowest-risk crates. They prove byte vectors, string/path helpers, JSON canonicalization, simple DTOs, and deterministic smoke tests before larger crates adopt the pattern.

### Phase 2: Data Contract Crates

- [ ] `auki-manifests`
- [ ] `auki-registry`
- [ ] `auki-logs`

These introduce record/enums, JSON-shaped adapters, file IO, opaque bytes, and log round trips. For wasm, expose only web-safe behavior. Do not expose filesystem APIs in browser wasm unless there is a real browser storage adapter.

### Phase 3: Transform Primitives

- [ ] `auki-geometry`
- [ ] `auki-time`

These are closest to the SDK's core operations: `convert_pose` support and `convert_time` support. Bind pure math first. Gate native clocks, sampler threads, and filesystem-backed log production behind native features.

### Phase 4: Protobuf Boundary

- [ ] Keep `auki-proto` as generated protobuf output, not a normal UniFFI wrapper by default.
- [ ] Test it through `just generate-proto` and per-language protobuf smoke vectors.
- [ ] Use generated protobuf packages beside generated UniFFI packages for Python, Swift, and JavaScript consumers.

`auki-proto` is a schema-generated package family, not a hand-authored SDK crate. Treat it as a sibling binding track.

### Phase 5: Network Runtime

- [ ] Finish `auki-network` as the production reference for async/native plus wasm split.
- [ ] Enable UniFFI Python generation for `auki-network`; the existing `auki-network-py` PyO3 package is legacy.
- [ ] Keep browser transport JavaScript-owned where needed, with Rust wasm exposing canonical identity and protocol bytes.
- [ ] Add real browser-to-native smoke coverage for the browser-probe path.

### Phase 6: Domain Orchestration

- [ ] `auki-domain`

Do this after network, time, registry, and log surfaces are stable. The first binding should be a bounded `ClusterManager` surface through UniFFI, not the entire internal runtime. Browser wasm should expose only behavior the runtime can actually support.

### Phase 7: Adapter Crates

- [ ] `auki-ros-adapter`

Bind translation-only APIs first. Keep `ros2` / `r2r` subscriber code native-only and feature-gated. There is no meaningful browser wasm surface for ROS2 runtime code.

## Per-crate Test Gate

Every migrated crate should pass this baseline before landing:

```bash
cargo test -p <crate> --no-default-features
cargo test -p <crate>
cargo check -p <crate> --target wasm32-unknown-unknown --no-default-features --features wasm
python3 scripts/bindings/generate_bindings.py plan python <crate>
python3 scripts/bindings/generate_bindings.py plan swift <crate>
python3 scripts/bindings/generate_bindings.py plan javascript <crate>
```

Generated package checks:

```bash
AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings <crate>
just generate-javascript-bindings <crate>
just generate-swift-bindings <crate>
```

`just generate-swift-bindings` is macOS-only because the XCFramework path uses Apple targets, `lipo`, and `xcodebuild`.

## Cross-language Vectors

Each crate should pin behavior with vectors that can be asserted from Rust, generated Python, generated Swift, and generated JavaScript where that language is enabled:

- `auki-hash`: fixed bytes to fixed XXH3-128 lowercase hex.
- `auki-jcs`: fixed JSON value to canonical UTF-8 bytes.
- `auki-layout`: fixed IDs to path helper outputs.
- `auki-identity`: seed to wallet id, peer id, signature, verification, and child derivation.
- `auki-manifests`: manifest builders to JSON field sets, canonical bytes, and hashes.
- `auki-registry`: entry constructors, hashes, and read/write round trips.
- `auki-logs`: opaque-byte append/read/tail behavior and segment-file locked vectors.
- `auki-geometry`: fixed frame conventions, point/vector/pose conversions, and error cases.
- `auki-time`: NTP sample math, offset selection, domain-clock composition, and overflow/error cases.
- `auki-network`: protocol IDs, identity/protobuf export bytes, message/probe framing, and browser probe.
- `auki-domain`: membership JSON, manager election facts, domain-clock estimate behavior, and cluster lifecycle smoke tests.

## CI Shape

- Linux lane: Rust tests, Python UniFFI generation/import smoke, JavaScript wasm smoke.
- macOS lane: Swift generation, XCFramework build, SwiftPM build, and iOS simulator smoke where a crate has an iOS-facing runtime surface.
- Optional release lane: cross-built Python native libraries for supported targets.
- Integration lane: Discovery-backed and browser/native networking tests, isolated from fast unit/binding checks.

## Landing Discipline

- Land the harness updates first.
- Migrate one crate per PR once the first foundation crate proves the pattern.
- Do not delete PyO3 packages in the same PR as the UniFFI replacement unless the consumer migration has already happened.
- Keep crate docs current: update `README.md`, `src/readme.md`, `src/sprint.md`, and changelogs with every crate migration.
- Add parking-lot items only for real unresolved questions, such as JavaScript package target policy or a native-only API that lacks a web-safe shape.
