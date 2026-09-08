# ZITADEL browser supervision proof

Run from the SDK root, with Node/npm, Chrome, the Wasm Rust target, and
`wasm-bindgen-test-runner` **0.2.121** (matching `Cargo.lock`). `wasm-pack` 0.13.1
can provide the cached runner, or install that pinned `wasm-bindgen-cli` version.
The harness uses Playwright CLI 0.1.19 and refuses occupied test ports.

```sh
WASM_BINDGEN_TEST_RUNNER=/absolute/path/to/wasm-bindgen-test-runner \
  bash test-support/run-zitadel-browser-tests.sh
```

The script builds the Wasm tests, starts only its loopback fixture/runner on
ports 18109/18107, launches its own named Chrome session, asserts the test
results, then closes its browser and stops its own processes. Logs and snapshots
remain under ignored `output/playwright/`. It never touches deployed services,
real credentials, existing browser profiles, or another listener.

The main browser suite exercises actual Wasm `BrowserNode` and supervisor code:
terminal classifications and lifecycle delivery, transient/persistence backoff,
the ten-second renewal bound, isolated Domain denial, stale pending installation,
and stable Peer IDs. The session test uses real Fetch to a synthetic loopback
IdP/DDS fixture, including cancellation after the server consumes the refresh
grant and a rejected acknowledged save. Both cases assert one refresh and no
DDS admission before storage acknowledges, then successful signed peer proof.
API exchange and Domain listing must remain unused. Rust implements this stage's store;
JavaScript/Swift host callback adapters are tested separately in the binding stage.

The separately filtered suspension test installs a normally signed 30-minute
credential issued 1785 seconds earlier. After installation, Playwright freezes
the actual page with Chrome's `Page.setWebLifecycleState` for 16 seconds, then
resumes it. No production lifetime or verification rule is changed. The test
checks expired-authority fencing during persistence failure, then successful
renewal on the same browser Peer ID. Cold Wasm crypto initialization has explicit
headroom; the browser is frozen only after the test signals readiness.

These are SDK runtime tests, not cross-service acceptance. The local HTTP
fixture substitutes IdP/API/DDS for deterministic request counts. Z10 separately
runs the actual API, DDS, DMS and relay chain. The suite makes no claim about
immediate remote revocation or already-open stream closing deadlines.
