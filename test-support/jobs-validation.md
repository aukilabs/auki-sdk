# Domain jobs validation

This records the SDK implementation for [#379](https://github.com/aukilabs/auki-sdk/issues/379).
Offline checks use local HTTP or intercepted Fetch fixtures and synthetic
credentials. Authorized dev runs exercised Python/shared Rust on 2026-09-16 and
WASM in Chrome plus Swift on macOS and the iOS Simulator on 2026-09-17. Final
job states were independently audited through the SDK. No workers were provisioned
and no backend or deployment changes were made.

## Provider contract

Source and public dev build metadata were checked on 2026-09-16. DMS main
`06bd863a8b8537dabd761b826818844a72593d2c` has the same source tree as dev build
`8088111`; DDS main `b27c0804a7ff6c99b228c3b8f36665fb3ebea10f` has the same tree
as dev `v0.14.6`. Both provider main commits and the DMS dev build were unchanged
when rechecked on 2026-09-17. Cluster image digests were not rechecked. See the
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

## Native Rust/Python live dev results

The run used aligned dev API/DDS/DMS endpoints, the retained test App, and an
existing dedicated compute fixture. Each run registered a unique third-party
capability, with one attempt per task. The worker uppercased small test records;
no robot, P2P, relay, or discovery operation was involved. Job requests used
`session.jobs(domain_id)` in the actual Python extension, including the configured
DMS URL ending in `/v1/`.

| Check | Result |
| --- | --- |
| User two-stage graph | Passed: estimate `0.004`, submission, dependency order, progress/events, two worker receipts, byte-for-byte output reads, and filtered listing |
| Trusted App single task | Passed: estimate `0.002`, submission, execution, progress/receipt, output read, and filtered listing; the compute fixture matched the App's organization while the permitted Domain had a different owner |
| Running-job cancellation | Passed: cancel acknowledgment, handler termination, canceled task, provider audit receipt with no outputs, and reported credit release |
| Read-only App | Passed on 2026-09-17 with a fresh session: Domain data listing succeeded, while job listing and submission both returned HTTP 401. This is the deployed middleware's response for a grant without write scope; no permission changes were needed for this final check |
| Imported ZITADEL | Domain/job listing passed; estimate returned HTTP 400 with no eligible test worker in this account's organization. No successful job submission/execution is claimed |
| Cleanup | All six run-owned Domain records deleted; temporary App permissions and legacy access-control fields matched their original snapshots; worker closed and original capabilities/version/mode/concurrency restored |

Job IDs retained for audit:

| Job | Final state | Purpose |
| --- | --- | --- |
| `39f512dd-2956-4731-b63b-30d6ddd1fa1d` | Completed | User graph, two tasks |
| `a6747d14-51bd-4fa7-8e10-d366c9978063` | Completed | App task |
| `7a73fe19-c089-450e-b637-e99582697bd3` | Canceled | Running cancellation with awaited heartbeat cleanup |
| `21da0ea9-4dde-49c2-a674-0ffbdec89001` | Canceled | Initial cancellation conflict, then cancellation followed by worker shutdown and lease expiry |
| `0687ea9b-3a7c-4d8e-bb55-28528a88e44d` | Canceled | Queued follow-up while the prior lease was still active |

All tasks are terminal. Completed tasks report `0.006` total debited credits;
the three canceled jobs report no task debit. All five jobs' combined estimates
were `0.012`, below the harness's `0.10` ceiling. Terminal job/receipt records remain
as audit evidence. Private harnesses and credential state are ignored and are
not committed.

### Findings and regression validation

- The initial worker poll returned 404 because the native DMS client appended
  `tasks` to `/v1/` as `/v1//tasks`. Removing the empty trailing path segment fixes
  claims, heartbeats, completion, and failure requests. The successful live runs
  used the rebuilt extension with this fix and the original `/v1/` configuration.
- An initial cancellation returned 409, consistent with a concurrent heartbeat
  changing the lease before DMS's guarded task update. A fresh read and explicit
  cancellation retry succeeded. The SDK continues to expose the conflict without
  automatic retries.
- **Backend follow-up:** job `21da0ea9-4dde-49c2-a674-0ffbdec89001` drained through
  lease expiry and has a cancellation receipt, but its `credit_released_at` was
  still null on 2026-09-17 for a `0.002` credit lock. The audited DMS sweeper
  finalizes canceled tasks without calling the credit-release helper, and
  repeated cancellation returns early for an already canceled job. This records
  the DMS state; the underlying credit service's lock was not independently
  inspected or changed. Heartbeat-based and queued cancellation reported release.

Additional checks after the URL fix:

| Command | Result |
| --- | --- |
| `cargo test --locked -p auki-dms` | Passed, including the new HTTP regression for all four worker operations with and without a trailing slash |
| `cargo clippy --locked -p auki-dms --all-targets -- -D warnings` | Passed |
| `cargo test --locked -p auki-tasks -p auki-sdk --quiet` | 151 passed |
| Venv `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` | Rebuilt successfully |
| Venv `python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_jobs.py core/bindings/python/auki-sdk-py/python_tests/test_tasks.py core/bindings/python/auki-sdk-py/python_tests/test_robot_tasks.py -q` | 82 passed |
| `cargo fmt --all -- --check`, `git diff --check` | Passed |

## WASM and Swift live dev results

On 2026-09-17, the existing dedicated compute fixture served unique Web and
Swift capabilities for run `dfb9dad3-0283-4fc3-81cd-30d66187246c`. These clients
used User login; no App secrets or machine credentials entered the browser or
Swift apps. Each runtime submitted one completion job and one running-cancellation
job through its public SDK binding, with no submission retries.

| Runtime | Completed job | Canceled job |
| --- | --- | --- |
| WASM, Headless Chrome 153 | `e3124a51-6e2f-4b48-ab1e-f8027d1d4f14` | `4a5d2996-36ac-44a4-bba0-98feda59dc2d` |
| Native Swift, macOS arm64 | `ba6f709e-0698-4a44-bb2f-af0c7566af51` | `449b07f0-3705-484d-b5e8-90e484457501` |
| Native Swift, iPhone 17 Pro Simulator, iOS 26.5 | `37e2f0ce-3c3e-4733-a2b9-a4355dc2b3d3` | `d61886aa-47de-4b04-948c-30aad28a1bda` |

All three runtimes passed login, decimal-price estimation (`0.002` per job),
submission, completion/progress, worker receipt checks, exact output-content
reads, and capability-filtered listing. Cancellation began only after observing
the task running with its waiting progress; each client then verified canceled
job/task state and reported credit release. The independent audit verified that
each cancellation receipt had no outputs. Swift
also checked the receipt's SHA-256 and byte count. Jobs/data clients and sessions
were closed with awaited cleanup.

The browser loaded the generated WASM module from `http://127.0.0.1:18139` and
called the dev API, DDS, DMS and Domain Server directly. Requests were not mocked
or proxied, and browser security remained enabled. This exercises the actual
Fetch/CORS path, rather than the intercepted Fetch used by offline tests.

Swift executed the generated UniFFI binding and the typed `Jobs.swift` wrapper.
The macOS executable linked the freshly built native Rust static library. The
simulator app linked the branch's generated XCFramework simulator slice
(`ios-arm64_x86_64-simulator/libauki_sdk_swift.a`) and headers. `vtool` verified
the app's `IOSSIMULATOR` platform, minimum iOS 17.0, and SDK 26.5. This is a real
simulator network round trip, not the fake generated object used by the codec test.

Commands and local evidence (private harnesses are intentionally uncommitted):

| Command / artifact | Result |
| --- | --- |
| Web binding: `npm run check` | WASM rebuilt and TypeScript checks passed |
| `npx --yes --package @playwright/cli@0.1.19 playwright-cli --session jobs-379-wasm-20260917 open http://127.0.0.1:18139/ --browser chrome`, then snapshot/click the run button | Real Chrome run passed; sanitized report in `.jobs-dev/wasm-live-report.json` |
| `.jobs-dev/swift/build.sh` | `cargo build -p auki-sdk-swift --release --features standard-protocols --locked` plus `swiftc` against generated bindings and the Rust static library passed |
| `.jobs-dev/swift/live-jobs .jobs-dev/platform-config.json .jobs-dev/swift/run-macos` | macOS live run passed; `run-macos/evidence.json` records assertions and IDs |
| `.jobs-dev/swift/build-ios-simulator.sh` | `xcrun swiftc` targeting `arm64-apple-ios17.0-simulator`, UIKit and the XCFramework simulator slice passed |
| `xcrun simctl install 7A1E86FA-CB95-45B2-883E-F91CE10B7EC8 .jobs-dev/swift/LiveJobs.app`, then `xcrun simctl launch 7A1E86FA-CB95-45B2-883E-F91CE10B7EC8 com.aukilabs.sdk379-livejobs` | iOS live run passed; `.jobs-dev/swift/run-ios-simulator/evidence.json` records assertions and IDs |

An independent final SDK audit confirmed all six jobs/tasks terminal, all six
reported credit locks released, and all six created Domain records deleted.
Completed tasks report `0.006` credits debited in total; canceled tasks report no
debit. The worker was stopped and its original configuration restored. The browser
and loopback server were closed; the temporary simulator app was uninstalled, its
credential copy removed, and the simulator booted for this test was shut down.
No SDK runtime change was needed for these platform checks.

## Remaining live validation

Successful imported-ZITADEL execution needs an approved compatible worker in that
principal's organization. The saved imported session expired after the successful
listing check; on 2026-09-17 its refresh was rejected as OAuth `InvalidRequest`,
so further imported checks need a fresh session with the SDK as sole refresh owner.
No refresh or issuer validation was relaxed. The Web/Swift checks above cover User
login. Expo's live JavaScript-to-native jobs flow and physical-device execution
remain unrun; the Swift simulator run does not claim those additional paths.

The backend cancellation credit-release finding above remains unresolved; backend
changes are outside this SDK PR.

Current DMS pagination can omit an item between pages; the backend correction is
tracked separately in [#396](https://github.com/aukilabs/auki-sdk/issues/396).
This client deliberately passes the provider cursor through unchanged.
