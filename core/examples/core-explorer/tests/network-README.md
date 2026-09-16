# Local M2 network harness

## Local validation and limits

The integrated example was verified with freshly built combined Web/WASM,
Python Echo and relay artifacts, including the SDK shutdown fix from
[PR394](https://github.com/aukilabs/auki-sdk/pull/394). The M1 Chromium data suite
and the real browser-to-Python network suite passed. Three minimal robot runs
and both robot runs in the full network suite exited normally with code 0,
empty advertisements/bookings and zero task requests.

The narrow checks use unit doubles or synthetic loopback services; they do not
establish a browser Echo roundtrip. The separate full suite exercises real
Chromium/WASM, WSS framing, libp2p relay circuits and the compiled Python module.
Desktop/mobile screenshots showed the verified response without horizontal
overflow. This is local Linux/Chromium evidence, not deployed-provider validation
or complete browser/OS coverage. No SDK or binding implementation changes are
part of this example.

## Setup

Requires Rust 1.89+, Node 22.18+, OpenSSL, Python with venv, wasm-pack 0.13.1,
and the `wasm32-unknown-unknown` target. See the
[portable Web](../../portable-echo/web/README.md) and
[Python](../../portable-echo/python/README.md) build prerequisites.
From the repository root, use a fresh environment for this checkout:

```sh
python3 -m venv .venv-network
. .venv-network/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
# Optional: set an absolute CARGO_TARGET_DIR before all builds and harness runs.
# export CARGO_TARGET_DIR='/absolute/path/to/generated-artifact-cache'
maturin develop --locked --manifest-path core/examples/portable-echo/python/Cargo.toml
export ROBOT_PYTHON="$VIRTUAL_ENV/bin/python"
cargo build --locked -p core-explorer-test-relay
cd core/examples/core-explorer
npm ci
npx playwright install chromium
npm run build
```

On resource-constrained hosts, use `CARGO_BUILD_JOBS=1`, run the heavy gates
serially, and apply suitable resource limits outside critical service processes.

Both network harnesses use `python3` from PATH when `ROBOT_PYTHON` is unset.
Both use the repository's `target/debug/core-explorer-test-relay` when
`CARGO_TARGET_DIR` is unset. A relative target directory is resolved from the
repository root; use an absolute path when invoking Cargo from another directory.
Build and test with the same target directory. All Python SDK and Echo objects
come from the single `auki_portable_echo` module. All Web objects come from the
combined Portable Echo WASM module; do not mix separately compiled SDK objects.

## Run the checks

From `core/examples/core-explorer`, with dependencies and artifacts prepared:

```sh
# Narrow checks: no compiled robot, browser, relay, or external DNS required.
npm test
python3 -m unittest discover -s robot -p 'test_*.py'
node --test tests/network-config.test.mjs tests/network-fixture.test.mjs

# Compiled-artifact integration gates, in order:
npm run test:browser
SHUTDOWN_REPEATS=3 node tests/network-minimal.mjs
npm run test:network
```

The harness owns its fixtures and children; do not start `fixture.mjs` separately.
Keep loopback ports 18114 and 18116 free, plus ephemeral loopback ports for the
Domain Server, network fixture, relay and WSS bridge.

The native and browser network gates require **external public DNS** for
`127.0.0.1.sslip.io`. They check that resolved addresses are loopback; Chromium
maps only that exact hostname to 127.0.0.1. No global resolver or hosts changes
are made. Service HTTP listeners and relay sockets bind loopback. Self-signed
TLS acceptance is scoped to the disposable Chromium context. Never point this
harness at shared services, and never supply real credentials to its fixtures.

The minimal gate stops on the first failed exit. Each robot must exit normally
with code 0 after ordered, awaited cleanup, with advertisements/bookings empty,
withdrawal/release observed and zero task requests. A timeout or signal exit
fails. The 270-second observation ceiling allows for native relay/protocol joins;
it is a harness ceiling, not proof that every SDK join has a deadline. Failure
teardown may terminate children, but forced termination never counts as success.

## Contracts and acceptance assertions

- `network-fixture.mjs` composes the M1 `fixture.mjs` through a fixed loopback
  proxy. Password User login uses the ordinary API service-token exchange and
  DDS Domain listing. Imported-ZITADEL listing profiles are outside this example.
- M1 data authorization uses a separate DDS grant with `iss=dds`, the selected
  Domain, unexpired `exp`, and the Domain Server URL in `aud`; portal reads also
  require the DDS audience. These match `auki-auth/src/domain.rs` and
  `portals.rs`. The data fixture is synthetic and does not prove server-side
  signature or permission enforcement. P2P grants are separate.
- Robot registration and binding follow the current Python task fixtures.
  Machine tokens carry the configured robot audience, stable UUID identity and
  assignment, `node_type=robot`, `node_mode=dedicated`, and the bound Peer ID.
  Response expiry derives from the signed expiry claim.
- P2P credentials use ES256, `iss=dds`, exclusive `aud=[auki-p2p]`, transport
  peer and selected Domain binding, `domain-data:r`, and exactly 1800 seconds
  between `iat` and `exp`. The fixture verifies one-time Ed25519 peer proofs.
  SDK signature, issuer, audience, expiry, peer, Domain and scope checks stay on.
- Discovery publication/listing/withdrawal follows SDK discovery; booking
  snapshots follow relay-booking contracts. Empty HTTP 204 responses omit JSON
  content type. Renewal advances authority/provider expiry; discovery excludes
  expired advertisements. Discovery is not proof of liveness or authorization.
- The relay uses real libp2p TCP/Noise/Yamux and relay circuits with a bounded
  synthetic source-admission lease of 20 seconds (below the SDK's 30-second cap).
  Source admission is a test fixture, not provider authorization validation.
- TLS termination and binary WebSocket framing bridge to the relay TCP stream.
  Browser routes contain `/wss/p2p/relay/p2p-circuit/p2p/target`; the bridge is
  not a direct WebSocket proxy to the Python robot.
- The runner requires explicit DDS/DMS endpoints, audience, private identity
  directory and environment-only registration credential. `tasks.start()` starts
  an idle runtime; no task polling occurs and its handler rejects invocation.
  Echo accepts only bounded diagnostics. Cleanup shields and awaits Echo, tasks,
  then credential closure before propagating cancellation.
- The browser suite requires actual `sendExact` verified bytes, both endpoint
  proofs, a bridge connection and zero task requests. It also asserts auth denial,
  browser restart, Domain switch during discovery, logout during startup,
  unavailable robot/relay, persistent-identity restart, advertisement withdrawal
  and booking release. Browser output is checked for credential leakage.

The runner's optional `AUKI_SHUTDOWN_TRACE=1` emits only fixed phase/status labels
and elapsed time. The harness reports sanitized counters, observes exit before
sending the stop signal, and checks maps before teardown clears them. These
observations aid diagnosis; none replaces the required normal process exit.
