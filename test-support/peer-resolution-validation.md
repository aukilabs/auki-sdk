# Exact identity resolution validation

Validated locally on 2026-09-17, based on SDK `develop` commit `eba0a576`.
Provider contract: [domain-service #568](https://github.com/aukilabs/domain-service/pull/568),
with the exact-filter implementation introduced in `64166e6c` and migration 45.
All credentials and peer connections in these checks were synthetic and local.

## Behavior exercised

- Exact Peer, Robot and Compute filter encoding, including every continuation page.
- Several peers per machine, deterministic result order, retained requested identity,
  Domain and protocol, and explicit failure instead of incomplete lookup success.
- Missing/wrong provider acknowledgements, including empty pages and a later page
  from an older provider; no unfiltered fallback.
- Missing/wrong advertised subject/type, query cancellation, expired candidates,
  cross-Domain result rejection, and platform route selection.
- Real local TCP peers: both robot and compute credentials, exact Peer ID lookup,
  wrong signed subject and wrong signed type. Rejected streams close successfully
  and the receiver observes zero application bytes. Valid streams exchange bytes.
- Chrome/WASM Fetch resolution for all three identity types and multi-page responses,
  old-provider rejection, the shared signed-identity predicate, and WSS route policy.

## Checks

| Command | Result |
| --- | --- |
| `cargo build --locked -p auki-sdk` | Passed. |
| `cargo test --locked -p auki-sdk` | 131 unit + 2 public-facade tests passed. |
| `cargo clippy --locked -p auki-sdk --all-targets -- -D warnings` | Passed on native. |
| `cargo check --locked -p auki-sdk --target wasm32-unknown-unknown` | Passed. |
| `cargo clippy --locked -p auki-sdk --target wasm32-unknown-unknown --lib --tests -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. |
| `RUSTDOCFLAGS='-D warnings' cargo doc --locked -p auki-sdk --no-deps` | Passed; 29 local Markdown links also checked. |
| `WASM_BINDGEN_TEST_RUNNER=<0.2.121 runner> bash test-support/run-zitadel-browser-tests.sh` | 21 browser tests + 1 suspension/resume test passed in Chrome. |
| `PYO3_PYTHON=/opt/homebrew/bin/python3.12 cargo test --locked -p auki-sdk-py --no-default-features` | 5 Rust tests passed. |
| `cargo test --locked -p auki-sdk-swift` | 19 Rust tests passed. |
| `maturin develop --locked --no-default-features --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` in a venv | Built successfully. |
| Same Maturin build with default features, then `python -m pytest core/bindings/python/auki-sdk-py/python_tests -q` | All 105 tests passed. |
| Web binding: `npm ci`, `npm run check` | WASM build and TypeScript contract checks passed. |
| Expo: `npm ci --ignore-scripts`, `npm run build`, `npm test`, `npm run test:domain-data`, `npm run test:jobs`, `npm run test:fleet` | Passed. |
| `IPHONEOS_DEPLOYMENT_TARGET=17.0 bash core/bindings/expo/scripts/sync-ios-xcframework.sh` | Device ARM64 + simulator ARM64/x86-64 builds, UniFFI generation, Swift typechecking, XCFramework validation and Expo synchronization passed. |
| `bash core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh` | Generated Swift FFI loopback tests passed, including renewal, imported owner/viewer listing, persistence, and cleanup counters. |

The broader `cargo test --locked -p auki-sdk -p auki-p2p -p auki-auth` run passed
96 authentication, 105 P2P unit, and 25 authenticated-transport tests, then hit the
existing timing-sensitive relay test
`cancellation_barrier_closes_every_direct_relay_connection_before_recreate`.
Its assertion expected replacement admission to remain blocked after receiving
the unpublication event. The test passed alone and the complete
`cargo test --locked -p auki-p2p --test relay_transport` rerun passed all 12 tests.
No P2P source or expectations were changed to accommodate it.

An initial Python run using `--no-default-features` passed 100 tests, skipped 3
optional Info cases, and failed the 2 tests that require standard-protocol exports.
Rebuilding with the default features produced the all-105-passing result above.
WASM `--all-targets` also selects existing native-only examples (`compute_task`,
`robot_task`, `domain_data`); those do not compile for browsers. The supported
WASM library/tests lint command above passed.

## Limits

The new API is native/WASM Rust; no new raw stream methods were added to language
bindings. Binding checks establish compatibility of their existing surfaces.
The connection identity regression uses real native TCP peers; browser coverage
uses local Fetch fixtures and shared verification/route-policy tests. A new
browser-to-native WSS relay end-to-end run and Expo iOS app build were not performed.
No shared-environment validation, deployment, or rollout was performed. Actual
provider support must be established before enabling exact lookups.
