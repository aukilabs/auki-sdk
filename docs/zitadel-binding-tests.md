# ZITADEL binding runtime proof

Run from the SDK root. These tests use synthetic credentials and a loopback-only
IdP/API/DDS fixture. They verify actual generated Web/Wasm and Swift/UniFFI
adapters, **not** the Z10 real Auki service chain or a live ZITADEL tenant.

## Web

Requires Node/npm, Chrome, wasm-pack, Rust's `wasm32-unknown-unknown` target, and
the wasm-bindgen-test-runner version matching Cargo.lock (currently 0.2.121).
Install the existing package's npm dependencies if necessary. Set the runner's
absolute path; wasm-pack normally caches it with its matching wasm-bindgen CLI.

```sh
WASM_BINDGEN_TEST_RUNNER=/absolute/path/to/wasm-bindgen-test-runner \
  bash test-support/run-zitadel-web-bindings.sh
```

The script runs the existing `npm run check` (wasm-pack plus public TypeScript
contract), loads generated production JS/Wasm in Chrome, and runs five host-flow
cases. It then runs all 41 feature-enabled Web binding Wasm tests in the same
real browser, including exact opaque subjects and fractional timestamps. The
Playwright CLI is pinned to 0.1.19. Snapshots and logs are under
`output/playwright/zitadel-z08-*`. Expected HTTP 400/403/503 negative cases are
browser resource errors, not uncaught JS failures; the Wasm runner's favicon 404
is harmless. Test-result assertions fail the script, including CLI-reported errors.

## Swift

Requires Xcode/Swift, Rust's three Apple targets listed in the build script,
Node, and the existing UniFFI generator. Generated sources/artifacts stay ignored.

```sh
bash bindings/swift/auki-sdk-swift/build-xcframework.sh
cargo test -p auki-sdk-swift --features standard-protocols --locked
bash test-support/run-zitadel-swift-bindings.sh
```

The existing artifact script builds device and both simulator architectures,
generates bindings, assembles the XCFramework, and typechecks Swift against its
exact headers. The host script builds a matching macOS Rust static library and
executes those generated Swift bindings with an actual async Swift actor store.
Eight host cases cover import without I/O, acknowledged single-flight save,
rejected/unexpected host errors and retry, Domain denial, retained startup handle,
restart, terminal errors, and close/cancel/drain before storage clear. It also
checks nanoseconds through the credential object and exact UTF-8 subject bytes
through the generated FFI RustBuffer converter. The Rust Message conversion test
covers the production authenticated-peer-to-record side. The test actor's store
is deliberately in-memory; it is not a Keychain implementation. Results are in
`target/zitadel-swift-host/`. Z09 separately exercises the Expo/iOS simulator host.

Both scripts require port 18111 to be free, so run them sequentially. Web also
uses 18112. They refuse unknown listeners and clean up only their own fixture,
runner, and named browser. No backend containers, other browsers, remote
identities, or shared environments are modified.
