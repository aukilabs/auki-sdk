# Local ZITADEL peer acceptance

This is the Z13 direct-DDS evolution of the cross-service Z10 test, not a
production login or deployment script. Existing `Z10_*` runner variable/artifact
names are retained for compatibility with the local tooling.
It exercises two Rust peers sharing one imported session and two browser peers
sharing a second session. Never share the same rotating refresh token between
those processes. Application integration starts with the
[Rust session contract](../crates/auki-auth/README.md#zitadel-sessions),
[Web/Swift bindings](zitadel-binding-tests.md), and
[Expo Web/iOS handoff example](../bindings/expo/example/README.md).

## What is real

The new chain is SDK ZITADEL access token → DDS identity/policy/peer proof →
DMS booking → standalone relay admission → native/browser authenticated Info
requests in both directions. The selected Domains exist only in DDS, never in
API's catalog. API is exercised separately for password/app and migration-only
`purpose=p2p` compatibility; the new SDK path does not call it or list Domains.
Postgres, Redis, signatures, libp2p possession proofs, TCP, WSS and relay circuits
are real. No Auki authorizer is disabled and no pre-minted peer token bypasses
the chain.

API and DDS run opt-in `zitadel_acceptance` Go test entrypoints, mounting their
production handlers, authentication and database queries on loopback listeners.
DMS runs its production router and database-clock relay sweeper through a
loopback-only example entrypoint. These are not the complete production mains:
unrelated data-storage, reward, notification and compute jobs are not started.
The relay **is** the actual `relay-node/cmd` binary, including real DDS SIWE,
peer-key proof, provider-session renewal, booking claims and reservation handling.

Only external ZITADEL/OIDC and policy-system are replaced by
[`zitadel-local-fixture.mjs`](../test-support/zitadel-local-fixture.mjs).
It simulates discovery, rotating public-client grants, introspection and the
contract-tested Domain-specific `domain_metadata_read` check. DDS's introspection
client assertion is verified against a generated test RSA key, expected client,
key ID, algorithm, audience and expiry. The API-only compatibility fixture still
does not verify the old test client's signature. It deliberately exposes no legacy
policy `/domains` endpoint. The actual policy HTTP/SQL/OpenFGA contract has its
own test in the `auth` repository; it is not running in this harness.

## Prerequisites

Use the Z11–Z13 revisions plus the existing opaque-subject consumers recorded in the workspace's
`ZITADEL_TODO.md`. Required sibling paths are:

```text
workspace/
  auki-sdk/                 # run from this checkout
  api/
  domain-service/
  domain-manager-service/
```

Install the repositories' normal Go and Rust dependencies, the Rust
`wasm32-unknown-unknown` target, `wasm-pack`, Node.js/npm, Docker, OpenSSL with
`req -addext`, and Google Chrome. The native test host and process shutdown use
Unix APIs (verified on macOS). Private Go module access may be needed during
the first build; no live Auki/ZITADEL credentials are needed. Run
`npm ci` in `bindings/web/auki-sdk-web` before the first run. The runner builds
the Wasm package and uses pinned `@playwright/cli@0.1.19` via `npx`.

Ports 18120–18123 and 18125–18131 must be free. Postgres/PostGIS uses 5432 and
Redis 6379, with all listeners bound to `127.0.0.1`. Run from a clean development
shell without service configuration overrides; the runner does not load `.env`
files. It disables HTTP proxies only in its child processes.

The relay uses `127.0.0.1.nip.io`, checked to resolve exclusively to loopback,
because relay addresses require a DNS hostname. A local TLS proxy terminates
WSS for the actual relay WebSocket listener. A generated one-day certificate is
accepted by its exact SPKI pin only in the named test Chrome process. The runner
does not edit DNS/hosts, system trust stores, clocks or global browser settings.

## Run the short E2E check (default)

From `auki-sdk/`:

```sh
node test-support/run-zitadel-local.mjs
```

With free 5432/6379 ports this creates fresh named task PostGIS and Redis
containers. If those ports are already occupied, it refuses to kill their
owners. To reuse **only previously created disposable ZITADEL test containers**:

```sh
Z10_POSTGRES_CONTAINER=<task-postgres-container> \
Z10_REDIS_CONTAINER=<task-redis-container> \
node test-support/run-zitadel-local.mjs
```

Both names must start with `zitadel-`, be running, and have the expected single
loopback port binding. These containers must contain only disposable test data.
The runner creates uniquely named `zitadel_z10_api_*`, `zitadel_z10_dds_*` and
`zitadel_z10_dms_*` databases and applies actual migrations. It never reuses an
existing database or runs migrations against a configured shared database.
Redis uses DB 10 with a unique nonce prefix; it never flushes Redis. The first
image/build downloads may take time (`postgis/postgis:latest`, `redis:alpine`,
`aukilabs/pgmigrator:latest`); exact tested image IDs are in the execution ledger.

Target **2–5 minutes after builds** for a healthy run: startup/preflight, two
minutes of relay traffic once all four peers are ready, revocation checks and
clean shutdown. Each runtime must report at least three successful cross-runtime
probe cycles; elapsed time alone cannot pass the test. The traffic phase has a
five-minute failure deadline, with separate bounded startup/shutdown waits.

All processes use real wall-clock time. API service tokens and synthetic IdP
access tokens last one hour, DDS P2P tokens thirty minutes, and P2P renewal uses
the existing 75% threshold. No production lifetime, clock or validation rule is
changed. Initial SDK-owned refresh and persistence are exercised immediately
using the fixture's expired import hints. The short check does **not** wait for
P2P renewal or literal expiry of those newly issued credentials.

### Focused lifecycle checks

Run these alongside the smoke check, not an hour-long soak on each iteration:

```sh
cargo test -p auki-auth --locked
cargo test -p auki-sdk --lib authority::tests --locked
WASM_BINDGEN_TEST_RUNNER=<wasm-bindgen-test-runner-0.2.121> \
  bash test-support/run-zitadel-browser-tests.sh
```

The existing native/browser tests use signed near-expiry fixtures, controlled
renewal schedules and persistence barriers. The native timeout test advances
Tokio's deadline clock; browser suspension uses an actual 16-second Chrome
freeze/resume. They cover renewal, expiry fencing, stable Peer IDs, one refresh
owner, cancellation and terminal failures without changing production TTLs.
These are component/runtime proofs, not a claim that the entire service chain
survived an hour of wall-clock operation.

### Optional normal-lifetime soak

Only when explicitly requested, add `--soak` to the same command (including the
container variables above when reusing task dependencies):

```sh
node test-support/run-zitadel-local.mjs --soak
```

Allow **70–80 minutes after builds** and keep the machine/browser awake. This
retains the original four P2P generations per peer, multiple ZITADEL refreshes
and literal old-bearer rejection checks. It is not the routine Z13 gate.

## Assertions and evidence

The runner fails on a host/service exit, terminal peer failure, unexpected
identity/TTL, or two minutes without a successful cross-runtime relay probe.
It checks:

- Exact numeric non-UUID subject across direct DDS admission, discovery, DMS
  storage and authenticated remote requests. The human can read DDS-only Domain
  A but not Domain B; the org-scoped user cannot enter a foreign organization's
  Domain. DDS HTTP mutation/data-token endpoints reject both raw ZITADEL and the
  migration-only read-scoped API bearer.
- No-read denial, two different Domains sharing an org-inherited session, and
  legacy UUID password/app issuance and DDS proof. The standalone relay exercises
  the unchanged UUID machine path throughout the soak.
- Concurrent starts and renewals with one refresh owner per runtime, durable
  whole-payload save acknowledgement, rejected-save retry without another
  rotation, retained credentials after DDS/policy startup failure, cancellation after
  the IdP consumes the refresh token, restart and latched `invalid_grant`.
- Four stable Peer IDs with repeated authenticated cross-runtime relay requests.
  Every live issuance must use the direct route and selected DDS-only Domain.
  API exchange count stays exactly three:
  one compatibility wire test and two legacy password/app flows, none from ZITADEL SDK.
- **Soak only:** at least three direct DDS P2P renewals and access-token expiry,
  with another ZITADEL refresh in each runtime and continuous discovery/traffic.
  The original service and P2P bearers are submitted after literal expiry and
  rejected. These checks are explicitly marked not run in a smoke result.
- New direct admission is denied after the fixture revokes read access, while peers
  with existing valid cached authority remain usable. This demonstrates the
  accepted revocation delay; it does not promise immediate revocation or a hard
  closing deadline for existing remote streams.
- Ordered peer shutdown, session close and host credential deletion.

Artifacts are retained under `target/zitadel-z10-*/`: separate build/service logs,
safe event records, database/container names and, on success, `result.json` with
`passed: true`, `mode`, `sustainedRenewalChecked` and `literalExpiryChecked`.
A passing `mode: "smoke"` is not a passing soak. Browser snapshots and CLI evidence live under
`output/playwright/zitadel-z10-*/`. The target directory also contains synthetic
private keys and credentials, is mode 0700, and must not be published. Generated
artifacts are ignored by Git. Expected negative auth HTTP responses can appear
as browser console resource errors; a host failure is a separate failing event.

On exit or Ctrl-C, only owned hosts/listeners and the named Chrome session are
closed, the relay is drained before DMS stops, and containers created by this
invocation are stopped. Reused containers are left running. Databases and
container data are retained for inspection, not dropped. Inspect the recorded
container/database names before any later cleanup; never stop a whole unrelated
Compose project or kill processes by port.

## Known unresolved soak failure

On 2026-09-08, run `target/zitadel-z10-UOC9Zx` passed direct admission and relay
traffic, then failed approximately ten minutes after readiness, before P2P
renewal. The browser reported `RelayReservationFailed`: the selected direct
relay connection closed. Native peers continued until controller cleanup.
DMS booking renewals were still active; no identity/policy denial was observed.
This does not establish the cause or prove an auth-independent defect. The
original host's shutdown error could mask its first failure; the harness now
records the primary run error before cleanup. No production relay fix was made.

Track this as a separate relay-lifecycle investigation. A short smoke pass does
not resolve it, and sustained direct-DDS operation remains unverified. Do not
rerun the long soak automatically or present its failed run as passing coverage.

## Regression and rollout boundary

Z13 supplements, rather than replaces, SDK unit/transport tests, backend serial
and race suites, DDS migration 42, DMS migration 11 and legacy lock vectors, and
the [binding](zitadel-binding-tests.md), [browser](zitadel-browser-tests.md) and
[actual Expo Web/iOS](../bindings/expo/example/README.md) host gates. The execution
ledger records exact commands, failures and reruns. The legacy DDS shell
integration runner is not used: it changes Git revisions and kills port owners.

The original opaque-subject upgrade used coordinated stop → migration → restart
with updated DDS, DMS, relay validators and receiving peers. Z11–Z13 add no new
schema migration or final P2P token format: configure/deploy DDS direct admission,
then use the new SDK. API's legacy exchange and migration-only bridge remain.
The original migration is not mixed-version database compatibility. See sibling repositories'
`docs/p2p-subjects.md`, `docs/peer-read-admission.md` and `docs/relay-subjects.md`.
DMS opaque booking history prevents lossless downgrade even after expiry; DDS
opaque advertisements must be removed by their ordinary cleanup before its
guarded down migration can succeed. Never rewrite identities to force rollback.

Still separate from local acceptance: an actual ZITADEL tenant PKCE login,
audience/organization metadata, public-client `none`, `offline_access`, provider
CORS and token-idle/absolute-lifetime configuration; production TLS/provisioning;
and a reviewed deployment/backup/rollback procedure. No push, PR publication,
deployment, tenant provisioning or release is performed by this harness.
