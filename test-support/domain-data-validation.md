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

The API's [ordinary exchange](https://github.com/aukilabs/api/blob/0cdcda5d1404b9be1998d919a2040018d50f9c57/pkg/api/service.go)
projects imported identities into existing User/App token profiles. An owner
receives `user-access` with a null or empty Domain restriction list; a scoped human
User receives `user-access` with explicit allowed IDs. The paged `list(org=own)`
route intersects the token's organization with these restrictions. The legacy
`accessible_domains` picker uses DDS access-control semantics: owned Domains,
public Domains, and Domains explicitly shared with that organization, also
intersected with any token restrictions. These are the existing local User
listing contracts; the two pickers need not return identical totals.

The SDK initially required the separate `purpose=p2p` exchange for every
imported session. This was an SDK route-selection bug: it unnecessarily blocked
owners whose ordinary User grant already authorized DDS listing. Imported
listing now selects the existing User route when the ordinary grant validates
as `user-access`. It validates the organization, optional Domain restrictions,
token profile, issuer, audience, expiry, and returned page.

Imported viewers receive `app-access` with an opaque human subject. That token
must never enter legacy DDS listing: `/accessible-domains` expects a UUID App
subject, while `/domains` can omit App permission filtering for an opaque
subject. For this profile, the SDK uses the separate `purpose=p2p` exchange,
validates its `user-p2p-access` grant and explicit nonempty human Domain
allowlist, then requests DDS `/accessible-domains`. An older API that ignores
the purpose fails closed. Denials are preserved; the SDK does not retry a
denied viewer as an unrestricted User.

These token profiles are coarse role projections, not complete per-operation
permission reports. Domain data and P2P retain their separate authorization
contracts. Listing never establishes write or delete permission.

### Deployed dev system, inspected read-only on 2026-09-15

The user authorized inspection of the dev cluster. Workload image digests were
matched to ECR tags, rather than inferred from `latest` or Argo chart revisions:

| Service | Running source / version | Deployment evidence |
| --- | --- | --- |
| API | `0cdcda5d1404b9be1998d919a2040018d50f9c57` | Digest `a78244796e90c42c314820fc808b61eb950542cebb31db2463bdaaa1d7c62459`; image changed by `kubectl-edit` on September 8; Argo `OutOfSync` against `d388b2f6` |
| DDS | `a49345e0a0d864a78ae57a3e5d484c3d4d4f6461`, `v0.14.6` | Digest `4ea5189961f64ea4d74703130323bf6ade3d0f70bb9c8fd9503b7024c06839cc` |
| Policy service | `c8ae1e2393b282ce06aeda1677a2fc6b9160e69e` | Digest `dda428f62aaa26f928e7738b6f12c7a39232495f0e307955961fe073460d912c` |

API and DDS point to the dev policy service and ZITADEL issuer. The supplied
identity has `api_role=owner` and `cactus_role=organization.admin`. Live policy
queries returned org-wide metadata-read permission and explicitly allowed
metadata read/write and data read on the selected test Domain. This account's
owner failure was not a missing policy grant.

The newer API [metadata-read bridge](https://github.com/aukilabs/api/blob/0cdcda5d1404b9be1998d919a2040018d50f9c57/pkg/api/peer_service_token.go)
still enumerates API's legacy Domain table before checking each candidate.
The selected DDS Domain is absent there: API lookup with its ordinary service
grant returned 404, while DDS listing and data access succeeded. The bridge
returned 403 `no readable Domains`. Direct ZITADEL bearers on both DDS listing
routes returned 401; the ordinary service exchange is required.

Viewer parity therefore still needs backend integration between the bridge's
candidate source and the authoritative DDS catalog, preserving per-Domain
policy checks. This is related to [#384](https://github.com/aukilabs/auki-sdk/issues/384).
The bridge came from [API #415](https://github.com/aukilabs/api/pull/415), which
closed unmerged; API `main` does not contain it despite the pinned dev build.
Its rollout also needs explicit ownership. No backend, policy, secret, or
deployment changes were made during this inspection.

The currently configured DDS admin endpoint on port 18190 has no candidate
enumeration route. A bridge fix needs a bounded, authenticated DDS catalog
query or a policy-service adapter that materializes those IDs; changing an
environment flag or duplicating DDS records into API is not a complete fix.
The external human allowlist token contract can remain unchanged.

Earlier source/configuration checks found staging/production DDS pinned to
`v0.13.3`, API staging `v0.8.2` with policy authentication, and API production
`v0.7.3` without that setting. Those environments were not inspected live.
Validate the target deployment before claiming compatibility.

## Local validation

All new tests use synthetic credentials and local HTTP or Fetch fixtures.
The implementation reuses the core Domain client and session refresh owner
on each platform.

| Check | Result |
| --- | --- |
| `cargo build --locked -p auki-sdk` | Passed |
| `cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk -p auki-tasks -p auki-dms --quiet` | 289 passed after imported User listing correction |
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
| `PYO3_PYTHON=<venv>/bin/python cargo clippy --locked -p auki-sdk-py -p auki-sdk-swift --all-targets -- -D warnings` | Passed |
| `cargo test --locked -p auki-sdk-swift --lib` | 17 passed |
| `bash core/bindings/swift/auki-sdk-swift/build-xcframework.sh` | All three iOS slices built; generated Swift typecheck and framework validation passed |
| `bash test-support/run-zitadel-swift-bindings.sh` | 8 generated Swift imported-session runtime cases passed |
| `bash core/bindings/swift/auki-sdk-swift/run-domain-data-bindings-test.sh` | Generated Swift User/imported Domain data runtime passed; one multipart completion, one abort, zero outstanding uploads, only the seed record retained |
| Expo `npm run build`, `npm run typecheck`, `npm run test:domain-data` | Passed, including the final Web error-code mapping |
| Expo `bash scripts/run-domain-data-expo-web.sh` | 8 actual Metro/Chromium/WASM cases passed |
| Expo `bash scripts/sync-ios-xcframework.sh`, then CocoaPods/Expo/Hermes Release simulator app build | Passed with the framework rebuilt for iOS 15.1; no newer-deployment-target linker warnings |
| Expo `AUKI_DOMAIN_DATA_IOS_APP=<built app> bash scripts/run-domain-data-expo-ios.sh` | 8 actual iOS simulator cases passed |
| `cargo fmt --all -- --check`, `git diff --check`, documentation and script checks | Passed; 67 local Markdown links, 19 heading anchors, and changed shell/Python/JavaScript fixture syntax |

Native imported-session regressions cover distinct data/peer credentials,
wrong issuer/audience/Domain/expiry, denied writes/deletes, one concurrent
refresh, awaited persistence and retained-generation retry, cached-grant
persistence checks, repeated authentication rejection, cancellation, and
logout draining a pending save. Browser tests exercise the actual generated
WASM binding, streaming/abort cleanup, error codes, and credential-store
promises. Browser requests are intercepted with synthetic Fetch responses.

Imported listing regressions cover the actual owner `domains:null` profile,
ordinary scoped User grants, wrong organization/Domain responses, explicit
viewer human allowlists, invalid issuer/audience/expiry/allowlist,
old providers ignoring `purpose`, server pagination and empty pages beyond
the total, 101-Domain accumulation,
out-of-allowlist responses, API/DDS 403, bounded DDS 401 renewal, one concurrent
API refresh/save, retained-save retry, and cancellation before DDS I/O.
Viewer tests prove the ordinary App-shaped bearer never reaches legacy DDS
listing, including when the separate viewer exchange is denied. The
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
The minimal build excludes optional application protocols. An earlier full
minimal run passed 86 tests and skipped three; two protocol-only
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
check an expired login, failed awaited save, identical retained-snapshot
retry with one refresh, successful known-Domain data, paged listing, preserved
403 denials, rejection of legacy token profiles before DDS I/O, unsupported
filters and portal association before I/O, and client/session close.
Runtime checks caught and fixed Swift's ambiguous `Double.init` conversion
for upload chunk limits and an Expo adapter
that prematurely supplied EOF.

Expo commands run from `core/bindings/expo`. The checked-in
`scripts/build-domain-data-expo-ios.sh` reproduces the simulator build. This
run used an equivalent already-prepared Release project under
`target/domain-data-expo-ios-owner15/DerivedData/`. Runtime artifacts are in
`output/playwright/domain-data-expo-web-94652/` and
`target/domain-data-expo-ios-app/run-3053/`. Browser, fixture processes, and
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

Before the owner-route correction, SDK `87efb124` also
passed a read-only live check: paged `domains().list` returned HTTP 403 with
`authorization_denied`, and `accessibleDomains` preserved the same auth code.
Both used `purpose=p2p`; neither proceeded to a DDS catalog or a broader
organization query. The same retained session then read 15 poses, an individual
pose, and filtered data metadata through the ordinary data exchange. Client
and session close completed with zero OAuth attempts or persistence callbacks.
Redacted evidence is in
`output/playwright/domain-data-dev-20260915/listing-result.json`.

### Imported owner picker and data after the SDK correction

The corrected, regenerated Web/WASM binding ran in isolated Chromium with
the same unexpired supplied credentials and the same issuer-blocking policy.
The full flow passed:

| Live operation | Result |
| --- | --- |
| Ordinary service exchange and two owned-Domain pages | HTTP 200; total 26, first two pages each contained 10 unique entries |
| `accessibleDomains()` with normal DDS User access rules | 864 entries accumulated across nine server pages |
| Select approved `dmtbot-test-domain` from the SDK picker | Passed |
| Selected-Domain pose reads and buffered create/read/replace/read | Passed; bytes matched |
| Delete the uniquely named record and reconcile by name | Passed; zero records remained |
| Awaited client/session close | Passed |
| Service responses / OAuth requests / persistence callbacks | All 27 service requests returned 200; zero OAuth requests or callbacks |

No request used `purpose=p2p`: the SDK selected the ordinary User grant before
listing. The owned total changed from 25 in the earlier operator probe to 26
in this later run; the broader accessible total follows separate DDS ACL rules.
No Domain or permission was created or changed by this test. The browser and
private local host were closed. Redacted evidence is in
`output/playwright/domain-data-owner-dev-20260915/result.json`.

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

Owner listing and data now pass live. Successful restricted/viewer listing
still needs the API/DDS bridge integration described above and suitable dev
identities. Preserve viewer denials; never send their App-shaped bearer through
legacy User listing. The outstanding App/restricted-account/renewal checks and
review/merge remain before claiming #374 complete. No backend implementation is
included.
