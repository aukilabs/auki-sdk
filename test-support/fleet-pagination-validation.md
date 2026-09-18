# Fleet pagination validation — 2026-09-18

> Historical first-slice evidence. The expanded DDS/SDK changes and current
> passing Python suite are recorded in [DDS discovery validation](dds-discovery-validation.md).

First slice of [#385](https://github.com/aukilabs/auki-sdk/issues/385): DDS
compute-node and Domain-robot inventory. This consumer branch is based on SDK
`140b7fd9`, including merged #400/#401. The provider adds opt-in UUID keyset pages
and an explicit versioned acknowledgement in
[DDS #569](https://github.com/aukilabs/domain-service/pull/569)
(`878dc943`, based on `1ce8d30b` after DDS #568 merged). See the
[Fleet contract](../docs/reference/fleet.md).

All runtime checks used local fixtures. No deployed provider was exercised and
no shared data, discovery records, relay bookings or jobs were created.

## Passed

Commands run from the repository root unless a directory is specified:

| Check | Result |
| --- | --- |
| `cargo test --locked -p auki-auth -p auki-fleet -p auki-sdk -p auki-sdk-swift` | 96 Auth, 26 Fleet, 131 SDK unit, 19 Swift adapter, 2 facade and 1 documentation tests passed. Includes the legacy-provider source-code assertion. |
| `cargo build --locked -p auki-sdk` | Passed. |
| `cargo clippy --locked -p auki-auth -p auki-fleet -p auki-sdk --all-targets -- -D warnings` | Passed. |
| `cargo check --locked --target wasm32-unknown-unknown -p auki-auth -p auki-fleet -p auki-sdk` | Passed. |
| `cargo clippy --locked --target wasm32-unknown-unknown -p auki-auth -p auki-fleet -p auki-sdk --lib -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. |
| `bash test-support/run-domain-data-browser-tests.sh` | 15 tests passed in Chromium through WASM/Fetch, including four Fleet tests. Used `wasm-bindgen-test-runner` 0.2.121 to match Cargo.lock. |
| `npm ci` then `npm run check` in `core/bindings/web/auki-sdk-web` | WASM compilation and TypeScript checks passed. |
| `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml`, in a virtual environment | Default-feature extension built successfully. |
| `.venv/bin/python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_fleet.py -q` | All 3 tests passed against the native extension and loopback HTTP fixture. |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | Device ARM64 and simulator ARM64/x86_64 builds, generated UniFFI Swift/header typecheck and XCFramework validation passed with Xcode 27. |
| `bash core/bindings/swift/auki-sdk-swift/run-fleet-bindings-test.sh` | Generated Swift client and typed Fleet models collected both robot and node pages against loopback HTTP. |
| `npm ci --ignore-scripts`, `npm run test:fleet`, `npm run build` in `core/bindings/expo` | Fleet bridge/Web adapter tests, WASM build and TypeScript package build passed. |

Regression coverage includes later-page activity joins, preserved query filters,
legacy complete responses, empty final pages, later denial with retained rows,
the 20-page bound, duplicate/unordered IDs, repeated cursors, malformed/missing
acknowledgements, mixed-provider responses and cancellation/awaited close during
continuation. Existing imported-session validation, refresh/persistence and
permission tests remain passing.

## Full Python suite failure

`.venv/bin/python -m pytest core/bindings/python/auki-sdk-py/python_tests -q`
reported **106 passed, 1 failed** after rebasing onto `140b7fd9`. The failing test is
`test_managed_events_drain_in_order_and_failure_preserves_artifact_metadata[run]`
in `test_tasks.py`, timing out while expecting continuous `run()` to raise after
a handler failure. Isolating both parameter cases reproduced the failure for
`run`; `run_once` passed.

The base commit [#403](https://github.com/aukilabs/auki-sdk/pull/403) changes the
continuous Rust runner to keep claiming work after `TaskError::Handler`. The
Python test still expects the previous behavior. Neither that runtime nor the
failing test is modified in this pagination change. The full Python suite is
therefore not green; its expectation needs a separate follow-up.

## Not exercised

No live SDK-to-DDS rollout test, Swift iOS simulator app or Expo Web/iOS app was
run for this change. The generated Swift/native host, XCFramework builds and
Expo bridge/build checks above are the platform evidence; they do not establish
deployed provider support. Deploy the provider consistently across DDS replicas
before relying on paged inventory. Portal/pose/data metadata and remaining
management lists are still tracked by #385.
