# Fleet validation

Implementation for [#380](https://github.com/aukilabs/auki-sdk/issues/380),
built on [#397](https://github.com/aukilabs/auki-sdk/pull/397) at `83fc4dd7`
and rebased onto its `develop` merge `3ab593fe`. The additional registry-Python
changes on `develop` do not alter the fleet dependency or binding sources.
Checks below were run on 2026-09-17 in the isolated `feat/380-auki-fleet`
worktree. All fleet requests used loopback HTTP or intercepted browser Fetch
with synthetic credentials. No shared service, machine, Domain data, discovery,
relay, task submission or deployment was changed.

## Provider assessment

The [fleet reference](../docs/reference/fleet.md) records DDS `b27c0804` and DMS
`06bd863a` source contracts, grant differences and the bounded-read behavior.
This assessment and #397's jobs validation do not establish deployed fleet
compatibility. DDS inventory is unpaginated (#385); multi-page jobs remain
explicitly partial because of #396. DMS busy is also unpaginated and byte-bounded.

## Shared Rust and WASM

Commands run from the repository root unless otherwise specified.

| Command | Result |
| --- | --- |
| `cargo build --locked -p auki-sdk` | Passed |
| `cargo test --locked -p auki-auth -p auki-dms --features auki-dms/jobs -p auki-fleet -p auki-sdk -p auki-domain-client -p auki-tasks -q` | 329 passed, including 20 fleet contract tests |
| `cargo clippy --locked -p auki-sdk -p auki-auth -p auki-dms -p auki-fleet -p auki-domain-client -p auki-tasks --features auki-dms/jobs --all-targets -- -D warnings` | Passed; fleet Clippy rerun after the final two regression cases also passed |
| `cargo check --locked -p auki-sdk -p auki-auth -p auki-dms -p auki-fleet -p auki-domain-client --no-default-features --features auki-dms/jobs --target wasm32-unknown-unknown` | Passed |
| `cargo fmt --all -- --check`, `git diff --check`, changed-document local link/path checks | Passed |

The fleet cases cover Domain assignment versus candidate membership, task joins,
unresolved workers, canceled jobs with active tasks, custom capability matching,
infrastructure exclusion, stale leases, conflicting assignments, offline and
unrecognized presence, denied/unsupported/oversized sources, opaque pagination,
and prevention of unrelated job-reference disclosure. They also cover User/App
and imported viewer/scoped User restrictions, invalid issuer/audience/expiry/
organization, a renewed busy grant changing organization coverage, retained
credential recovery after persistence failure, timeout, cancellation, and awaited
close without closing the session. Existing auth/jobs/data/task suites ran too.

## Python and Web

Python used a dedicated Python 3.12 virtual environment with the binding's test
requirements. Web used the locked npm dependencies and wasm-pack 0.13.1.

| Command | Result |
| --- | --- |
| Venv `maturin develop --locked --no-default-features --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` | Passed; fleet and jobs Python fixtures: 4 passed |
| Venv `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` | Passed with default features |
| Venv `python -m pytest core/bindings/python/auki-sdk-py/python_tests --ignore=core/bindings/python/auki-sdk-py/python_tests/test_process_exit.py -q` | 96 passed |
| Venv `python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_process_exit.py -q` | 9 passed |
| `PYO3_PYTHON=<venv>/bin/python cargo test --locked -p auki-sdk-py --lib -q` | 34 passed |
| `cargo clippy --locked -p auki-sdk-swift --all-targets -p auki-sdk-py --no-default-features -- -D warnings` | Passed |
| Web binding: `npm ci --ignore-scripts`, `npm run check` | WASM build and public TypeScript checks passed |
| `cargo clippy --locked -p auki-sdk-web --target wasm32-unknown-unknown --all-features --all-targets -- -D warnings` | Passed |
| `WASM_BINDGEN_TEST_RUNNER=<0.2.121 runner> bash test-support/run-domain-data-browser-tests.sh` | 12 Chromium tests passed, including 2 fleet tests |

Python exercises the actual extension against
[`fleet_fixture.py`](fleet_fixture.py). The browser cases exercise the actual
WASM client with intercepted Fetch, structured partial results, an aborted
in-flight request, and close/session reuse. Browser fixture traffic is synthetic;
this does not test deployed CORS or a live provider.

## Swift and Expo

| Command | Result |
| --- | --- |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | All three Apple slices, generated Swift typecheck and XCFramework validation passed |
| `bash core/bindings/swift/auki-sdk-swift/run-fleet-bindings-test.sh` | macOS generated-UniFFI/native HTTP fixture passed, including typed models, filters, cancellation, close and shared-session reuse |
| Expo binding: `npm ci --ignore-scripts`, `npm run build`, `npm run typecheck`, `npm test`, `npm run test:domain-data`, `npm run test:jobs`, `npm run test:fleet` | Passed; fleet covers the JS bridge and actual Expo Web adapter against an injected client |
| Expo binding: `IPHONEOS_DEPLOYMENT_TARGET=17.0 bash scripts/sync-ios-xcframework.sh` | Rebuilt and synced the matching framework and generated wrappers successfully |
| Expo example: `npm ci --ignore-scripts`, `npx expo prebuild --platform ios --no-install`; `example/ios`: `pod install` after framework sync | Passed; CocoaPods includes `Fleet.swift`, generated `auki_sdk_swift.swift`, `FleetRegistry.swift`, and the XCFramework |
| Expo `example/ios`: `xcodebuild -quiet -workspace ZitadelHandoffTest.xcworkspace -scheme AukiSdkExpo -configuration Release -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' -derivedDataPath <repo>/target/fleet-expo-ios-native CODE_SIGNING_ALLOWED=NO build` | Passed against the generated framework; arm64 Swift source list includes the generated API and typed fleet wrapper |

The first CocoaPods resolution preceded the framework copy and produced a build
using the missing-framework fallback. That result is excluded: native validation
requires resolving pods again after sync and verifying the generated fleet
sources are compiled. The final result above refers to that second build.

Swift adds `AukiSdkError.Fleet`; exhaustive error switches need the new case.
Generated bindings and native libraries must be upgraded together. Generated
frameworks, npm output, virtual environments and credentials are not committed.

## Remaining live and device checks

No fleet operation ran against a shared deployment. Before release, obtain the
approved environment, account/Domain and read scope, verify aligned API/DDS/DMS
builds and rollout flags, and exercise User, trusted App and imported session
permission differences. Check real robot assignment/status, public and dedicated
compute pools, Domain task joins and busy-feed denial without widening access.
Verify browser CORS against the chosen deployment.

The Swift fixture ran on macOS; iOS slices and the Expo module were built. No
fleet fixture ran inside an iOS Simulator app, no Expo JavaScript-to-native fleet
round trip was executed on a device, and no physical-device behavior is claimed.
Those runtime/live checks remain separate from the local coverage above.
