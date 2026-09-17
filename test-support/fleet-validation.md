# Fleet validation

Implementation for [#380](https://github.com/aukilabs/auki-sdk/issues/380),
built on [#397](https://github.com/aukilabs/auki-sdk/pull/397) at `83fc4dd7`
and rebased onto its `develop` merge `3ab593fe`. The additional registry-Python
changes on `develop` do not alter the fleet dependency or binding sources.
Checks below were run on 2026-09-17 in the isolated `feat/380-auki-fleet`
worktree. Local fixtures and the subsequent authorized dev E2E run are recorded
separately. Live validation exercised the existing compute fixture and a newly
provisioned DDS robot running a software-only handler on this Mac. Every existing
binding passed its live checks. No SDK runtime fix was needed.

## Provider assessment

The [fleet reference](../docs/reference/fleet.md) records DDS `b27c0804` and DMS
`06bd863a` source contracts, grant differences and the bounded-read behavior.
The live run used aligned `api.dev.aukiverse.com`, `dds.dev.aukiverse.com`,
`dms.dev.aukiverse.com/v1/`, and `auth.dev.aukiverse.com` endpoints. DDS public
Swagger metadata reported `v0.14.6`; DMS `/version` reported `8088111`. Provider
`main` still pointed to the assessed commits. These match the identifiers in
[jobs validation](jobs-validation.md#provider-contract); cluster image digests
were not queried. Authenticated robot provisioning, registration, task claims,
inventory and busy reads verified the required dev routes and rollout gates.

DDS inventory is unpaginated (#385); multi-page jobs remain explicitly partial
because of #396. The live User check observed `provider_pagination_unreliable`.
DMS busy is also unpaginated and byte-bounded. This validates dev only, not a
staging or production deployment.

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
| `cargo test --locked -p auki-sdk-swift --lib -q` | 19 passed |
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

## Live dev E2E results

Run `9dda9df9-f618-4689-93f7-d679231c2636` used Domain
`055a3c82-22a4-413b-968f-73b5c2e2e0db` in organization
`512693fb-63ec-4344-913b-67e3cc592cd6`. The retained dedicated compute node was
`aa57d4c7-19c2-4825-bcf4-89439625a46c`. The authorized DDS API provisioned robot
`1cba59cd-76de-4542-9d0e-2414e74933e3` and assigned it to that Domain before SDK
registration. Both workers ran locally through `AukiDmsTasks`, advertising only
this run's separate compute/robot capabilities. Robot registration required no
wallet, physical device or Posemesh deployment.

| Runtime | Live result |
| --- | --- |
| Native shared Rust through the Python 3.12 extension | Passed inventory and pool reads, robot lifecycle, controlled compute/robot jobs, cancellation, filters, permission differences and awaited close |
| Web WASM, Chrome 153 | Passed direct Fetch/CORS, active machine/task joins, filters, partial results, cancellation, close/session reuse, and a real imported-session refresh with awaited persistence |
| Swift, macOS arm64 | Passed generated UniFFI and typed `Fleet.swift` APIs against dev |
| Swift, iPhone 17 Pro Simulator, iOS 26.5 | Passed the same live checks through the XCFramework simulator slice; `vtool` confirmed `IOSSIMULATOR`, minimum iOS 17.0, SDK 26.5 |
| Expo Web, Chrome 153 | Passed the public JavaScript API, actual Expo Web adapter and generated WASM against dev |
| Expo iOS Release app, iPhone 17 Pro Simulator, iOS 26.5 | Passed public JavaScript through the actual native module, generated Swift binding and Rust XCFramework |

Each Web, Swift and Expo runtime ran both a read-only User case and an imported
owner case. The owner case verified both active workers' exact DDS IDs, DMS job
and task IDs, `busy` state and association (`assigned` robot, `active_task`
compute). It checked the separate dedicated candidate pool, public-pool and
capability filters, provider robot timestamps and null compute last-seen.
Cancellation and a call after close preserved structured errors; another fleet
client could still use the shared session. Browsers called dev services directly
with normal CORS enforcement, without intercepted Fetch or a backend proxy.

The Python lifecycle check additionally established:

- An unassigned robot was absent from the Domain view. Assignment made the
  unregistered robot visible with unknown presence/work state.
- Registration and DMS polling established online/idle presence. An idle compute
  candidate did not become a Domain member until it executed a Domain task.
- Completion removed compute activity while retaining the assigned robot.
  Running cancellation ended the handler, produced the cancellation receipt and
  returned the robot to idle after the observed activity drained.
- After the longer platform task, DMS reported completion while DDS retained a
  future `active_lease_expires_at`. Fleet correctly reported `unknown` rather
  than false idle during this authority-expiry window.

Native permission checks also used the existing User fixture's writable Domain
`de66fdf4-a830-4017-95dd-5741c30a6d0f`. Inventory and busy reads succeeded; jobs
reported partial coverage with `provider_pagination_unreliable`. The read-only
User fixture preserved inventory alongside HTTP 401 activity denials. Trusted
App checks stayed on Python: permitted inventory remained available, jobs were
denied with the retained permissions, and busy reported `unsupported`; candidate
work state stayed unknown. No User/App permissions were widened.

Only one runtime owned the imported session's refresh at a time. A loopback-only
handoff passed the latest credentials to each client; replacement snapshots were
atomically persisted to the canonical store and recovery copy before refresh
completed. Clients and sessions closed before ownership passed to the next
runtime. App secrets and machine registration credentials stayed on the host.

### Commands and evidence

Private harnesses and credential material are ignored and intentionally not
committed. Reports contain source states, assertions and run-owned audit IDs.

| Command / artifact | Result |
| --- | --- |
| Venv `python .fleet-dev/preflight.py`, `provision.py`, `workers.py`, `native.py native`, `lifecycle.py`, `native.py platform`, `permissions.py`, `finish.py` | Live Python/shared-core checks and final audit; reports under `.fleet-dev/` |
| `.fleet-dev/build-swift.sh`, then `.fleet-dev/live-fleet` | macOS executable linked the current generated bindings and `target/release/libauki_sdk_swift.a`; User/imported checks passed |
| `.fleet-dev/build-swift-ios.sh`; `xcrun simctl install` / `launch` on simulator `7A1E86FA-CB95-45B2-883E-F91CE10B7EC8` | Swift iOS app passed with credentials handed off in memory |
| `npx --yes --package @playwright/cli@0.1.19 playwright-cli --session fleet-380-web open http://127.0.0.1:18140/ --browser chrome`, then snapshot/click the run button | Web passed; `report-web-{user,zitadel}.json` |
| Expo example: `CI=1 EXPO_NO_TELEMETRY=1 npx expo start --web --port 18141`, then Playwright open/snapshot/click | Expo Web passed; `report-expo-web-{user,zitadel}.json` |
| Expo `example/ios`: `CI=1 EXPO_NO_TELEMETRY=1 xcodebuild -quiet -workspace ZitadelHandoffTest.xcworkspace -scheme ZitadelHandoffTest -configuration Release -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' -derivedDataPath <repo>/target/fleet-expo-ios-native CODE_SIGNING_ALLOWED=NO ENTRY_FILE=<repo>/core/bindings/expo/example/index.js build`; simulator install/launch | Full Release app passed; `report-expo-ios-{user,zitadel}.json` |
| `output/playwright/fleet-web-live.png`, `fleet-expo-web-live.png`, `fleet-expo-ios-live.png` | Local screenshots of successful runtime checks |

The native script completed its task assertions before its first lifecycle
check hit a harness error: `asyncio.create_task` requires a coroutine, while the
binding returns a Future. The corrected `ensure_future` lifecycle check and the
subsequent platform run passed without repeating the jobs. The cleanup harness
also needed to accept DDS's retained lease deadline after DMS completion; its
initial immediate-idle assertion was too strong. No uncertain job submission
was retried, and neither correction required an SDK or provider behavior change.

### Jobs and cleanup

| Job ID | Final state | Purpose |
| --- | --- | --- |
| `9b4f4573-c83c-40b6-b1fe-c9800067f88e` | Completed | Native compute lifecycle |
| `8b7baa79-c435-424e-b63c-de0a6e3f32dc` | Completed | Native robot lifecycle |
| `4a2913cc-7e0e-48eb-a229-d34797ea80b3` | Canceled | Running robot cancellation |
| `4bd00eb4-3277-4fa0-a592-834c0d7a8c49` | Completed | Compute observed across all platform clients |
| `a8acbe63-8d51-41b7-881d-a919a301a60c` | Completed | Robot observed across all platform clients |

The independent SDK audit found all five jobs/tasks terminal with receipts and
all five reported credit locks released. Combined estimates were `0.010` credits;
completed tasks reported `0.008` debited. Terminal jobs remain as audit evidence.
The handlers returned small metadata-only results, so no Domain data records
were created. No relay, discovery, P2P, infrastructure or deployment operation
was involved.

Both task runtimes and credentials closed with awaited cleanup. The compute
node's original capabilities, version, mode and concurrency were restored and
read back; its record and private credentials remain available for reuse. The
final pool read reported it offline with no activity.

The test robot's credentials were revoked after shutdown. DDS subsequently
reported offline presence, which Fleet represented with unknown work state.
Deletion initially returned HTTP 409 while authority was still valid; it
succeeded after the provider's drain window. A subsequent robot lookup returned
404 and the filtered Domain inventory was empty. No expiry or authorization
check was bypassed.

Both Playwright sessions and loopback servers were closed. Temporary Swift and
Expo apps were uninstalled, the simulator booted for this run was shut down,
and Expo example sources were restored. The latest imported credential snapshot
matches its private recovery copy, and no refresh-owner lock remains. No
credentials, private harnesses or generated build artifacts are committed.

## Remaining boundaries

Physical iPhone/Android hardware and real robot motion were not tested. Expo
Android remains unimplemented. Public compute scheduling was not exercised;
public inventory reads were. Imported viewer/scoped grants, persistence failure,
malformed providers and response limits remain covered by local fixtures rather
than new live principals or forced provider failures. Live App checks used the
retained read-only permissions; writable App activity is covered by fixtures.
Staging/production need their own provider build, grant and rollout verification.
