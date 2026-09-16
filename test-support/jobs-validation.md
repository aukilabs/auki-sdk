# Domain jobs validation

This records the SDK implementation for [#379](https://github.com/aukilabs/auki-sdk/issues/379).
All job requests in these checks use local HTTP or intercepted Fetch fixtures and
synthetic credentials. No shared jobs were submitted, workers provisioned, or
backend/deployment changes made.

## Provider contract

Source and public dev build metadata were checked on 2026-09-16. DMS main
`06bd863a8b8537dabd761b826818844a72593d2c` has the same source tree as dev build
`8088111`; DDS main `b27c0804a7ff6c99b228c3b8f36665fb3ebea10f` has the same tree
as dev `v0.14.6`. Cluster image digests were not rechecked. See the
[jobs reference](../docs/reference/jobs.md) for the authentication contract,
third-party dedicated pricing, worker availability, cancellation semantics,
and provider limitations.

## Offline checks

Commands run from the repository root unless a binding directory is specified.

| Check | Result |
| --- | --- |
| `cargo build --locked -p auki-sdk` | Passed |
| `cargo test --locked -p auki-auth -p auki-dms --features auki-dms/jobs -p auki-sdk -p auki-domain-client -p auki-tasks --quiet` | 308 passed after rebasing onto `develop` `d646de1d` |
| `cargo test --locked -p auki-dms --no-default-features --features jobs --test jobs_contract` | 19 passed |
| `cargo clippy --locked -p auki-sdk -p auki-auth -p auki-dms -p auki-domain-client -p auki-tasks --features auki-dms/jobs --all-targets -- -D warnings` | Passed |
| `cargo check --locked -p auki-sdk -p auki-auth -p auki-dms -p auki-domain-client --no-default-features --features auki-dms/jobs --target wasm32-unknown-unknown` | Passed |
| `wasm-pack test --node core/auki-dms --locked --no-default-features --features jobs --lib` | 1 transport test passed |
| Python venv: `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` | Passed with default features |
| Venv `python -m pytest core/bindings/python/auki-sdk-py/python_tests --ignore=core/bindings/python/auki-sdk-py/python_tests/test_process_exit.py -q`, then `python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_process_exit.py -q` | 94 + 9 passed |
| `cargo clippy --locked -p auki-sdk-py --no-default-features -- -D warnings` | Passed |
| `PYO3_PYTHON=<venv>/bin/python cargo test --locked -p auki-sdk-py --lib --quiet` | 34 passed |
| Web binding: `npm run check` | WASM build and TypeScript checks passed |
| `cargo clippy --locked -p auki-sdk-web --target wasm32-unknown-unknown --all-features --all-targets -- -D warnings` | Passed, including browser test code |
| `WASM_BINDGEN_TEST_RUNNER=<0.2.121 runner> bash test-support/run-domain-data-browser-tests.sh` | 10 Chromium tests passed, including 2 jobs tests |
| `cargo test --locked -p auki-sdk-swift jobs::tests` | 2 passed |
| `cargo clippy --locked -p auki-sdk-swift --all-targets -- -D warnings` | Passed |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | All Apple slices and generated Swift typecheck passed |
| `bash core/bindings/swift/auki-sdk-swift/run-jobs-bindings-test.sh` | Typed Swift codec exercise against a fake generated object passed |
| Expo binding: `npm run build`, `npm run typecheck`, `npm test`, `npm run test:domain-data`, `npm run test:jobs` | Passed after rebase |
| Expo binding: `bash scripts/sync-ios-xcframework.sh` | All three Apple targets rebuilt, Swift typechecked, and framework synced after rebase |
| Expo `example/ios`: `xcodebuild -quiet -workspace ZitadelHandoffTest.xcworkspace -scheme AukiSdkExpo -configuration Release -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' -derivedDataPath <repo>/target/jobs-expo-ios-native-rebased CODE_SIGNING_ALLOWED=NO build` | Native module build passed after rebase, including `JobsRegistry.swift` and the upstream message API, against ExpoModulesCore and the generated XCFramework |
| `cargo fmt --all -- --check`, `git diff --check`, local Markdown link checks | Passed |

The native jobs contract suite covers User, trusted App and imported ZITADEL
sessions; custom dedicated job graphs; decimal prices; opaque pagination;
progress and result receipts; denied permissions; invalid or expired Domain
grants; one shared renewal after concurrent 401s; failed credential persistence
and retained-snapshot recovery; byte limits; cancellation; awaited client close;
and uncertain submissions that must never be replayed automatically.

The WASM transport check exercises streamed response limits, request credential
policy, redacted errors and abort on drop. Browser and Python binding tests
exercise the actual session-to-jobs path, input conversion, result fields,
structured failures and cancellation. Expo JavaScript tests exercise its native
bridge contract and lifecycle. The Swift codec exercise verifies generated API
compatibility and field conversion; it is not a live DMS round trip.

The full Expo example app harness reached Metro bundling but failed because its
entry path used `/tmp` while the canonical checkout used `/private/tmp`. The
isolated native module build above passed. No new jobs flow was run in an Expo
simulator or on a physical device.

## Remaining live validation

No live job execution has been run for this change. It needs a separately
approved environment/Domain, an existing compatible worker and an agreed
credit budget. Exercise User, trusted App and imported ZITADEL grants, a custom
dedicated task, estimates, progress/results, denied access, and cancellation;
retain job IDs for cleanup and reconciliation. An imported session must have one
refresh owner. Mobile/browser live checks also need the target deployment's
network and CORS behavior verified.

Current DMS pagination can omit an item between pages; the backend correction is
tracked separately in [#396](https://github.com/aukilabs/auki-sdk/issues/396).
This client deliberately passes the provider cursor through unchanged.
