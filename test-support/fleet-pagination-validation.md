# Fleet and portal pagination validation — 2026-09-18

Consumer validation for [#385](https://github.com/aukilabs/auki-sdk/issues/385),
rebased onto SDK `84e10ad7` (including merged #406, #407 and #408). Provider:
[DDS #569](https://github.com/aukilabs/domain-service/pull/569), `a0276f4`, based
on `1ce8d30b`. See the [Fleet contract](../docs/reference/fleet.md) and
[portal page contract](../docs/how-to/domain-data.md#read-portal-pages).

This PR collects paged node/Domain-robot inventories and exposes explicit portal
pages. It uses the existing authentication contracts. The proposed human token,
direct ZITADEL routes, permission discovery and DMS grant changes were removed.
[#384](https://github.com/aukilabs/auki-sdk/issues/384) remains open; neither this
report nor these fixture tests establish a working restricted-viewer rollout.

All runtime checks used local fixtures. No deployed provider was exercised and
no shared data, discovery records, relay bookings or jobs were created.

## Pagination validation before the latest rebase

Commands run from the repository root unless a directory is specified.
Multiplatform checks ran at `cb04a513`; after incorporating #408, the affected
native suites and Fleet/SDK Clippy were rerun. The only merge conflict was in
the Fleet reference; both DDS pagination and #408's DMS compatibility guidance
are preserved. No binding or authentication code changed during that merge:

| Check | Result |
| --- | --- |
| `cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-fleet -p auki-tasks -p auki-dms -p auki-sdk-swift` | All passed: 97 Auth, 29 Domain client, 133 SDK, 30 Fleet, 27 Tasks, 36 DMS, 19 Swift adapter and 1 documentation test. |
| `cargo build --locked -p auki-sdk` | Passed. |
| `cargo clippy --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-fleet -p auki-dms -p auki-sdk-py -p auki-sdk-swift --all-targets -- -D warnings` | Passed. |
| `cargo clippy --locked --target wasm32-unknown-unknown -p auki-auth -p auki-domain-client -p auki-sdk -p auki-sdk-web --features auki-sdk-web/finite-protocols,auki-sdk-web/message,auki-sdk-web/stream --lib -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. |
| `bash test-support/run-domain-data-browser-tests.sh` | 15 tests passed in Chromium through WASM/Fetch, including Fleet continuation and portal page acknowledgement. Used `wasm-bindgen-test-runner` 0.2.121 to match Cargo.lock. |
| `npm run check` in `core/examples/portable-echo/web` | WASM compilation and TypeScript checks passed. |
| `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml`, in a virtual environment | Default-feature native extension built successfully. |
| `.venv/bin/python -m pytest core/bindings/python/auki-sdk-py/python_tests -q` | Full suite passed: 107 tests, including Fleet pagination and both portal page bindings. |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | Device ARM64 and simulator ARM64/x86_64 builds, generated UniFFI Swift/header typecheck and XCFramework validation passed. |
| `bash core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh` | Generated Swift client exercised both portal page methods, legacy acknowledgement, renewal, imported-session persistence and cleanup against loopback HTTP. |
| `npm run build`, `npm test`, `npm run test:domain-data`, `npm run test:fleet` in `core/bindings/expo` | Package/WASM build and bridge tests passed. |
| `bash core/bindings/expo/scripts/run-domain-data-expo-web.sh` | All 8 Expo Web case groups passed, including imported-session portal pages. |

Regression coverage includes later-page activity joins, preserved query filters,
legacy complete responses, empty final pages, later denial with retained rows,
the 20-page bound, duplicate/unordered IDs, repeated cursors, malformed/missing
acknowledgements, mixed-provider responses and cancellation/awaited close during
continuation. Existing imported-session validation, refresh/persistence and
permission tests remain passing. Core authentication/session and DMS job code
match the base branch; portal requests still use the existing API exchange and
DDS Domain grant.

The earlier Python runner expectation failure was fixed separately in merged
#406. The full Python suite above now passes without changing task semantics in
this PR.

## Limits

No live SDK-to-DDS acceptance or shared-environment test was run. Deploy pagination
consistently across DDS replicas before relying on continuation. Legacy complete
responses remain supported within the SDK's existing byte/deadline bounds.
Domain Server pose/data-metadata pagination remains separate work in #385.

Swift iOS and Expo iOS simulator runtime checks were not completed: this host's
CoreSimulator service is incompatible with the installed Xcode. The XCFramework
and native Swift host checks above passed; the Expo iOS app build was not rerun
after this cleanup. Android remains the documented unsupported-platform stub.

## Relay CI regression and rebase

The relay test failure also reproduced on the latest `develop`: the unchanged
case failed on its fourth local repeat. The libp2p incoming queue drops excess
simultaneous arrivals for the same protocol, and this test also released each
stream immediately after its own round trip instead of ensuring overlapping
route ownership.

The test now opens two distinct application protocols concurrently over the same
exact route, retains both streams through the one-circuit assertion, and proves
that the second stream still carries data after the first closes. Both closes
must succeed, and both application-server tasks are awaited. Runtime transport,
authentication, retry rules and CI jobs are unchanged.

`cargo test --locked -p auki-p2p --test relay_transport concurrent_exact_route_opens_single_flight_one_circuit -- --exact`
passed once after compilation and then **100 consecutive runs**.

The following checks also passed on the rebased branch:

- `cargo test --locked -p auki-auth -p auki-domain-client -p auki-dms -p auki-fleet -p auki-p2p -p auki-relay-booking -p auki-tasks -p auki-sdk`: **514 tests passed**, including all 13 relay integration tests.
- `cargo clippy --locked -p auki-p2p -p auki-sdk --all-targets -- -D warnings`.
- `cargo build --locked -p auki-sdk`.
- `cargo fmt --all -- --check`, `git diff --check` and changed Markdown link checks.

GitHub's Rust native, Python and Web/Expo jobs validate the final published head.
Swift/iOS runtime checks were not rerun for this test-fixture change.
