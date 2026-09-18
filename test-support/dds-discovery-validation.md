# DDS discovery and human identity validation — 2026-09-18

This extends [SDK #404](https://github.com/aukilabs/auki-sdk/pull/404), based on
`develop` at `8403091a` (including the Python/CI fix in #406). It covers the DDS
portion of #385, permission-aware discovery in #383, restricted-human source
support in #384, and provider contract planning for #381.

DDS is the authoritative Domain catalog. Imported discovery and data grants use
the original ZITADEL access token directly with DDS. API's Domain table is not
a candidate source. Data grants, machine operators, DMS jobs and P2P admission
remain separate authorization contracts. See the
[consumer guide and rollout gates](../docs/how-to/discover-domains.md).

## Provider changes

| Provider | Source reviewed and validated | Result |
| --- | --- | --- |
| [DDS #569](https://github.com/aukilabs/domain-service/pull/569) | `862b26b9`, additive catalog foundation `2429670` | Full local `make test`, `make go-build`, Swagger generation/check passed. GitHub CI passed both builds, unit coverage, vet, docs and integration scenarios. |
| [API #417](https://github.com/aukilabs/api/pull/417) | `d991d14` | Full isolated `go test -p 1 ./...`, normalization, build and Swagger checks passed; build/test CI passed. The corrected deterministic credential-cache test passed 20 repetitions. |
| [Policy #40](https://github.com/aukilabs/auth/pull/40) | `6a087ba` | `cargo check --locked -p policy-system`, 207 library tests with isolated OpenFGA, Clippy and formatting passed; image/test CI passed. |

Local backend fixtures used isolated Docker projects and synthetic credentials.
DDS used PostGIS/Postgres, Redis and Silo without host ports. The policy fixture
used a loopback-only ephemeral OpenFGA endpoint. No shared services, deployment,
App provisioning, machine enrollment or live human accounts were modified.

## SDK checks

Commands run from the repository root unless another directory is stated.

| Check | Result |
| --- | --- |
| `cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-fleet -p auki-tasks -p auki-dms -p auki-sdk-swift` | Passed, including 101 Auth tests, 23 DMS job contract tests, 26 Fleet tests, 131 SDK unit tests, 19 Swift adapter tests and the affected integration/doc suites. |
| `cargo build --locked -p auki-sdk` | Passed. |
| `cargo clippy --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-fleet -p auki-dms -p auki-sdk-py -p auki-sdk-swift --all-targets -- -D warnings` | Passed. |
| `cargo clippy --locked --target wasm32-unknown-unknown -p auki-auth -p auki-domain-client -p auki-sdk -p auki-sdk-web --features auki-sdk-web/finite-protocols,auki-sdk-web/message,auki-sdk-web/stream --lib -- -D warnings` | Passed. Native-only HTTP mock dev dependencies prevent using `--all-targets` as a WASM check; the browser suite below exercises the actual WASM tests. |
| `cargo fmt --all -- --check`, `git diff --check`, changed Markdown local-link validation | Passed. |
| `WASM_BINDGEN_TEST_RUNNER=<matching 0.2.121 runner> bash test-support/run-domain-data-browser-tests.sh` | 15 Chromium tests passed. The new explicit job-auth method retains boxed futures; without that allocation boundary, the combined Fleet test overflowed the debug WASM stack. |
| In a virtual environment: `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml`, then `.venv/bin/python -m pytest core/bindings/python/auki-sdk-py/python_tests -q` | Default-feature extension built; all 108 tests passed, including discovery and direct human data. The earlier continuous-runner expectation failure is fixed by merged #406. |
| In `core/examples/portable-echo/web`: `npm ci --ignore-scripts`, `npm run check` | WASM and TypeScript checks passed. |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | Device ARM64 and simulator ARM64/x86_64, generated Swift/header typecheck and XCFramework validation passed. |
| `bash core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh` and `bash core/bindings/swift/auki-sdk-swift/run-fleet-bindings-test.sh` | Generated Swift/native hosts passed against loopback fixtures. |
| In `core/bindings/expo`: `npm run build`, `npm test`, `npm run test:domain-data`, `npm run test:fleet` | WASM/TypeScript build and bridge tests passed. |
| In `core/bindings/expo`: `bash scripts/run-domain-data-expo-web.sh` | All eight real Expo Web case groups passed, including direct discovery, no API exchange for that path, portal pages, persistence and cleanup. |
| In `core/bindings/expo`: `bash scripts/build-domain-data-expo-ios.sh` | Release simulator app built successfully with the generated Swift binding. |

The final DMS allocation adjustment changes no FFI or wire signatures. Native
suites and the actual Chromium WASM suite were rerun after it; the generated
Swift/iOS app and Python host checks above precede that allocation-only change.

Regression coverage includes DDS-only catalog records, sparse filtered pages,
cursor binding, permission revocation and outages, malformed identity profiles,
wrong issuer/audience/Domain, expiry and scope checks, denied access without
broader retries, awaited refresh persistence, cancellation and cleanup. Separate
human data and DMS job caches prevent a data grant from becoming task authority.

## Limits and remaining acceptance

The Expo iOS runtime attempt stopped while `simctl create` waited for Xcode's
first-launch setup: installed CoreSimulator `1051.55.0` is older than the Xcode
build's expected `1171.7.0`. Only the task's own blocked processes were stopped;
no global simulator/toolchain repair was attempted. The app build passed, but
its runtime cases are unverified in this pass. Android remains unsupported.

No live provider acceptance was run. Release the additive DDS catalog and policy
App reader first, configure the dedicated App allowlist, and verify complete
multi-page reconciliation including DDS-only Domains. Then release API's viewer
correction and full DDS hardening. Every DDS replica must support the new human
routes before SDK consumers update. Record exact deployed versions during an
approved restricted-user read/denied-write/foreign-Domain/renewal cleanup run.

#385 still includes Domain Server data-metadata/pose pagination. #381 still
includes the public SDK provisioning facade and secure credential handoff;
this pass supplies the provider contract, pseudocode and permission fixes.
