# Domain data platform validation

This records the SDK-only completion work for [#388](https://github.com/aukilabs/auki-sdk/pull/388)
and [#374](https://github.com/aukilabs/auki-sdk/issues/374). Backend source and
configuration were inspected read-only on 2026-09-15. No provider contracts or
deployment configuration were changed. The approved live check below created
temporary dev data and verified its deletion.

## Provider compatibility

Known-Domain imported-session data uses the existing contract:

1. `POST /service/domains-access-token`, with the imported bearer and no query.
2. `POST /api/v1/domains/{id}/auth`, with the returned service bearer.
3. The selected Domain Server, with its DDS-issued Domain grant.

API `main` was `d388b2f6`; release `v0.8.2` was `da6dbbc`. Its
[service-token implementation](https://github.com/aukilabs/api/blob/d388b2f6/pkg/api/service.go)
projects imported identities into existing User/App token profiles. A viewer
receives an App-shaped token with an opaque human subject; other human roles
receive a User-shaped token with the provider's allowed Domain set. This is a
coarse role projection, not a complete per-operation policy contract. DDS and
the Domain Server retain their own permission checks.

Imported listing uses a separate `?purpose=p2p` exchange, which issues
`user-p2p-access` with an explicit human Domain allowlist, then DDS
`GET /api/v1/accessible-domains?limit=...&offset=...`. The implementation is in
API remote branch `feature/zitadel-read-admission` at `6b45246b`, associated with
[API #415](https://github.com/aukilabs/api/pull/415). That PR closed unmerged on
2026-09-08 and inspected remote `main` does not contain the exchange. This
source history alone does not establish what dev is running.

The 2026-09-15 live probe confirmed that dev recognizes the exchange:
`purpose=p2p` returned HTTP 403 with `no readable Domains`, while an invalid
purpose returned HTTP 400. The same bearer received HTTP 200 from the ordinary
exchange. This differs from inspected API `main`, which ignores the query and
returns the ordinary token. DDS's public Swagger reports `v0.14.6`; the exact
deployed API commit is not publicly exposed. Successful issuance and DDS
listing remain unvalidated for this account because API denied issuance.

The ordinary exchange cannot safely replace listing authorization for every imported role.
DDS's [Domain handlers](https://github.com/aukilabs/domain-service/blob/c2230120/dds/http/domain.go)
require a UUID App subject on `/accessible-domains`. On the older `/domains`
route, the opaque viewer subject can skip App permission filtering and expose
the organization's broader catalog. Imported listing therefore validates the
exact human token profile, issuer, audience, expiry, explicit Domain allowlist,
and the returned page. An older API that ignores `purpose` fails closed; HTTP
403 is preserved. The SDK never substitutes organization listing.
Portal-to-Domain association queries remain unsupported for imported sessions.

The bridge first enumerates the API's own Domain registry, then checks
`domain_metadata_read` per candidate. Its acceptance test explicitly separates
an API-catalog Domain from DDS-only Domains. The live account could discover
25 owned DDS Domains and use selected-Domain data, yet the listing exchange
returned `no readable Domains`. API catalog or policy coverage can therefore
block the picker independently of data access. No backend or policy change is
included here.

DDS `/accessible-domains` was introduced at `45ded2e6` / `v0.14.3`; inspected
source was `c2230120` / `v0.14.4`. Inspected deployment configuration pins
staging/production DDS to `v0.13.3`. API staging uses `v0.8.2` with policy
authentication; production uses `v0.7.3` without that setting. Dev uses `latest`.
Configuration and source support are not proof of current rollout. Validate the
target environment before claiming imported data or listing compatibility.

## Local validation

All new tests use synthetic credentials and local HTTP or Fetch fixtures.
The implementation reuses the core Domain client and session refresh owner
on each platform.

| Check | Result |
| --- | --- |
| `cargo build --locked -p auki-sdk` | Passed |
| `cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-tasks -p auki-dms --quiet` | 286 passed after imported listing support |
| `cargo clippy --locked -p auki-auth -p auki-domain-client -p auki-sdk --all-targets -- -D warnings` | Passed |
| `cargo clippy --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-sdk-web --target wasm32-unknown-unknown --features auki-sdk-web/finite-protocols,auki-sdk-web/message,auki-sdk-web/stream -- -D warnings` | Passed |
| `cargo clippy --locked -p auki-sdk-web --target wasm32-unknown-unknown --features finite-protocols,message,stream --all-targets -- -D warnings` | Passed |
| Web binding `npm ci`, `npm run check`; `npx tsc --noEmit` after the new error-code example | Passed |
| `WASM_BINDGEN_TEST_RUNNER=<0.2.121 runner> bash test-support/run-domain-data-browser-tests.sh` | 8 Chromium tests passed |
| `WASM_BINDGEN_TEST_RUNNER=<0.2.121 runner> bash test-support/run-zitadel-web-bindings.sh` | 5 generated Web host cases and the full 47-test WASM suite passed |
| Portable Echo Web `npm ci`, `npm run check` | Passed |
| Python `maturin develop --locked --manifest-path core/bindings/python/auki-sdk-py/Cargo.toml` | Passed in the binding's virtual environment |
| Same Python build with `--no-default-features`, then `test_domain_data.py` and `test_zitadel_session.py` | 9 passed |
| `python -m pytest core/bindings/python/auki-sdk-py/python_tests -q` | 91 passed; App denial regression also rerun after making its operations sequential |
| `PYO3_PYTHON=<venv>/bin/python cargo test --locked -p auki-sdk-py --lib --quiet` | 34 passed |
| `PYO3_PYTHON=<venv>/bin/python cargo clippy --locked -p auki-sdk-py --all-targets -- -D warnings` | Passed |
| `cargo test --locked -p auki-sdk-swift --lib` | 17 passed |
| `cargo clippy --locked -p auki-sdk-swift --all-targets --features standard-protocols -- -D warnings` | Passed |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | All three iOS slices built; generated Swift typecheck and framework validation passed |
| `bash test-support/run-zitadel-swift-bindings.sh` | 8 generated Swift imported-session runtime cases passed |
| `bash core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh` | Generated Swift User/imported Domain data runtime passed; one multipart completion, one abort, zero outstanding uploads, only the seed record retained |
| Expo `npm run build`, `npm run typecheck`, `npm run test:domain-data` | Passed, including the final Web error-code mapping |
| Expo `bash scripts/run-domain-data-expo-web.sh` | 8 actual Metro/Chromium/WASM cases passed |
| CocoaPods `AukiSdkExpo` Release compile and full Expo/Hermes Release simulator app build | Passed against the regenerated XCFramework |
| Expo `AUKI_DOMAIN_DATA_IOS_APP=<built app> bash scripts/run-domain-data-expo-ios.sh` | 8 actual iOS simulator cases passed |
| `cargo fmt --all -- --check`, `git diff --check`, documentation and script checks | Passed; 67 local Markdown links, 19 heading anchors, all 4 changed shell scripts, and Python/JavaScript fixture syntax |

Native imported-session regressions cover distinct data/peer credentials,
wrong issuer/audience/Domain/expiry, denied writes/deletes, one concurrent
refresh, awaited persistence and retained-generation retry, cached-grant
persistence checks, repeated authentication rejection, cancellation, and
logout draining a pending save. Browser tests exercise the actual generated
WASM binding, streaming/abort cleanup, error codes, and credential-store
promises. Browser requests are intercepted with synthetic Fetch responses.

Imported listing regressions cover the explicit human token profile, invalid
issuer/audience/expiry/allowlist, old providers ignoring `purpose`, server
pagination and empty pages beyond the total, 101-Domain accumulation,
out-of-allowlist responses, API/DDS 403, bounded DDS 401 renewal, one concurrent
API refresh/save, retained-save retry, and cancellation before DDS I/O. The
same session lock serializes listing, data, and peer refresh decisions.

The final native build and native/WASM lint commands above passed. An initial
WASM `--all-targets` invocation across the native facade selected native-only
test dependencies (`mio`) and failed. Production WASM targets and the Web
binding's WASM tests were then checked separately with the documented commands.

Python covers imported-session save failure and recovery, cancellation while
saving, denied/wrong-Domain data, paged and accumulated listing, recoverable
listing denial, and unsupported-filter rejection before I/O. App coverage verifies
explicit service endpoints, gateway MAC validation and propagation, a single
service-token renewal after DDS 401, and preserved write/delete 403 responses.
The minimal build excludes optional application protocols. The full minimal
run passed 86 tests and skipped three; two protocol-only
surface assertions failed because those features were omitted. Both pass in
the restored default-feature suite. The documented minimal selection above
exercises both data/authentication suites:

~~~sh
python -m pytest \
  core/bindings/python/auki-sdk-py/python_tests/test_domain_data.py \
  core/bindings/python/auki-sdk-py/python_tests/test_zitadel_session.py -q
~~~

The [shared loopback fixture](domain-data-local-fixture.mjs) serves separate
DDS and Domain Server listeners for the generated Swift and Expo runtime
checks. It never forwards requests, and its diagnostic counters exclude
credentials and request bodies.

Expo runtime cases cover User login without a peer, DDS 401 renewal, real
Domain pages, portal/pose lookup, metadata/CRUD, streamed upload/download,
explicit EOF, HTTP 403, cancellation during a pending source callback,
multipart abort/drain, and deletion of all test records. The imported cases
checks an expired login, failed awaited save, identical retained-snapshot
retry with one refresh, successful known-Domain data, paged listing, preserved
403 denials, rejection of legacy token profiles before DDS I/O, unsupported
filters and portal association before I/O, and client/session close.
Runtime checks caught and fixed Swift's
ambiguous `Double.init` conversion for upload chunk limits and an Expo adapter
that prematurely supplied EOF.

Expo commands run from `core/bindings/expo`. The checked-in
`scripts/build-domain-data-expo-ios.sh` reproduces the simulator build. This
run used an equivalent already-prepared Release project under
`target/domain-data-expo-ios-final/DerivedData/`. Runtime artifacts are in
`output/playwright/domain-data-expo-web-84031/` and
`target/domain-data-expo-ios-app/run-88152/`. Browser, fixture processes, and
the isolated simulator were cleaned up. Physical iOS hardware and Android
were not tested; Android remains outside the binding's supported scope.

<details>
<summary>Expo/Hermes iOS simulator results</summary>

<img src="screenshots/domain-data-expo-ios.png" width="320" alt="Expo iOS test app showing all eight local Domain data cases passed">

</details>

The Rust data API and provider wire formats are unchanged. Binding APIs are
additive, including a structured Swift Domain data error case. Regenerate the
Swift bindings and XCFramework together; consumers with exhaustive error
switches must handle the new case. Existing Swift login calls retain default
arguments. Python's imported-session constructor runs inside the asyncio loop
that owns its async store callback.

## Shared environment validation

The earlier #388 validation records dev User data round trips on filesystem
`v0.14.6` and S3 `v0.14.2`: Python transferred 17 MiB, Chromium transferred
16 MiB with byte checks, and native Rust shared the credential with a peer.
Portal/pose reads and temporary-record cleanup passed. These are inherited
results; they were not rerun during this SDK completion work.

[#391](https://github.com/aukilabs/auki-sdk/pull/391) and the closed
[#375](https://github.com/aukilabs/auki-sdk/issues/375) already record approved dev
compute/robot validation at SDK `95ef3b1c`: data exchange, direct/relay peers,
renewal across expiry and idle boundaries, denied idle writes/deletes and
wrong-Domain access, cancellation, and cleanup. That evidence covers the
machine data adapters, not imported ZITADEL or App credentials.

### Imported ZITADEL dev data, 2026-09-15

The user supplied a dev OAuth credential snapshot and authorized finding a
suitable Domain in their organization. An operator-only ordinary API exchange
returned `user-access`; DDS `GET /api/v1/domains?org=own&issue_token=false&limit=100&offset=0`
returned 25 owned Domains. This identified `dmtbot-test-domain`
(`055a3c82-22a4-413b-968f-73b5c2e2e0db`) on
`https://domain-s3.dev.aukiverse.com`. The server reported `v0.14.2`, S3
storage, and 8 MiB multipart parts. This particular owner's catalog lookup is
an operator setup step, not support for listing every imported human role.

The generated Web/WASM binding at SDK `051050dd` ran in isolated Chromium with
the real, unexpired credential snapshot. API, DDS, DMS configuration used the
dev presets. Only the local host, dev API/DDS, and selected Domain Server were
allowed by the host's Fetch guard and Content Security Policy. The issuer was
excluded so this check could not rotate the refresh token while ownership by
the originating app was unknown. The public client ID came from the supplied
token; this run does not validate a host's trusted OAuth configuration.

| Live operation | Result |
| --- | --- |
| SDK ordinary API exchange, then selected-Domain DDS authorization | HTTP 200 |
| Portal/pose lists and individual reads | 15 portals and 15 poses; individual reads passed |
| Buffered create, metadata, read, replace, and read again | Passed; byte checks matched |
| Multipart upload and streamed download | 17,825,809 bytes, three parts; SHA-256 matched |
| Unique-name reconciliation and deletion of both temporary records | Passed; zero matching records remained |
| Awaited data client/session close | Passed |
| OAuth requests or persistence callbacks | Zero |

The checksum was
`593ad47fff488379430c7358aac06a0e64f2a1b403644c68c97f38ac654a62f4`.
All 27 recorded service requests returned HTTP 200. An initial harness run
used a chunk cap below the server's 8 MiB part size; the SDK rejected it before
starting a multipart upload and still deleted its buffered record. The final
run used the advertised part size and passed. Both the browser and local host
were closed. Redacted local evidence is in
`output/playwright/domain-data-dev-20260915/result.json`; credentials and the
one-off host remain in an ignored private directory.

This confirms the actual imported-session SDK exchange and known-Domain data
path for the supplied owner on dev, including browser CORS. Renewal,
persistence recovery, restricted users, and imported-session live execution
on the other bindings remain covered by local fixtures rather than this run.

After implementing imported listing, the regenerated Web/WASM binding also
passed a read-only live check: paged `domains().list` returned HTTP 403 with
`authorization_denied`, and `accessibleDomains` preserved the same auth code.
Both used `purpose=p2p`; neither proceeded to a DDS catalog or a broader
organization query. The same retained session then read 15 poses, an individual
pose, and filtered data metadata through the ordinary data exchange. Client
and session close completed with zero OAuth attempts or persistence callbacks.
Redacted evidence is in
`output/playwright/domain-data-dev-20260915/listing-result.json`.

### Remaining live checks

App and restricted imported accounts have not been supplied. Their allowed/
denied operations and a designated inaccessible Domain remain unvalidated on
dev. Live OAuth renewal also remains unrun; it requires an established sole
refresh owner and trusted client configuration.

For each remaining account, run a unique-record round trip
(write, read, replace, multipart upload/download, delete), then denied-operation
checks with restricted credentials and a designated inaccessible Domain. Only
delete records created by that run. Preserve returned HTTP status and redact
credentials. Do not provision roles, Apps, Domains, robots, or compute nodes as
part of SDK validation.

Successful imported listing still needs approved dev identities whose readable
Domains exist in the API bridge's catalog, including restricted/viewer cases.
Preserve the current account's denial; do not broaden visibility to make a
picker pass. The outstanding live listing/App checks and review/merge must be
resolved before claiming #374 complete. No backend implementation is included.
