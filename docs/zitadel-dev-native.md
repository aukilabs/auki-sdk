# Native ZITADEL smoke test against dev

This runner uses the real dev identity provider, API, DDS, DMS and relay. It does
not run fixtures, change policy, deploy services, or modify Domain data. Once
admission succeeds, it creates two ephemeral peers with five-minute relay
bookings, advertises them through DDS, and requires three successful Info calls
in each direction over TCP relay circuits. It only probes its own two Peer IDs.
Shutdown is awaited for every started peer, including on test failure.

## Private configuration

Use a dedicated public-client grant that no other application is refreshing.
Fill the local, ignored `.env.zitadel-dev` with these keys:

```dotenv
AUKI_API_BASE_URL="https://api.dev.aukiverse.com/"
AUKI_DDS_BASE_URL="https://dds.dev.aukiverse.com/"
AUKI_DMS_BASE_URL="https://dms.dev.aukiverse.com/v1/"
AUKI_DOMAIN_ID=""
ZITADEL_ISSUER="https://auth.dev.aukiverse.com"
ZITADEL_CLIENT_ID=""
ZITADEL_ACCESS_TOKEN=""
ZITADEL_REFRESH_TOKEN=""
ZITADEL_ACCESS_TOKEN_EXPIRES_AT=""
```

The expiry is optional RFC3339; blank means unknown. No ID token, client secret,
browser credentials or legacy password is used. The selected Domain must have
effective `domain_metadata_read` permission for the authenticated user.

Before filling credentials, ensure `/.env.zitadel-dev` and
`/.zitadel-dev-state/` are ignored by Git. The dotenv and credential files must
have mode `0600`; the state directory must have mode `0700`. The launcher checks
private file permissions and rejects symlinks and unexpected service endpoints.

## Run

From the SDK root, with Node supporting `node:util.parseEnv` and the native Rust
toolchain installed:

```sh
node test-support/run-zitadel-dev-native.mjs
node test-support/run-zitadel-dev-native.mjs --exercise-refresh
```

The second command deliberately supplies an expired **local import hint** to
exercise one SDK-owned provider refresh immediately. It does not shorten or
prove survival across the provider's real token lifetime. The SDK saves the
complete replacement before continuing to the API. The short smoke test is not
a long-running authority-renewal soak and does not test browsers or revocation.

Rotated credentials are saved atomically, with file and directory fsync, in
`.zitadel-dev-state/native-session.json`. Subsequent runs acquire an exclusive
lock, then load that saved generation instead of replaying the original dotenv
refresh token. Keep this private state: the dotenv is no longer the current
refresh credential after rotation. Do not copy the saved grant to a second
active refresh owner. The seed hash retained in the private file only associates
the saved generation with its original dotenv grant; it is never logged.

An active, interrupted or uncertain run leaves `native.lock`. Do not blindly
remove it or replay old credentials: first establish that the old process has
stopped and whether refresh completed. An unknown refresh outcome requires a
fresh login. Preserve prior state for review when changing grants. Graceful
signal handling cleans up completed peers, but a booking created during a
cancelled startup may need to expire by its authority TTL.

Only redacted event diagnostics are printed. Credentials pass to Rust on stdin,
not through arguments or exported environment variables. Do not enable HTTP or
credential tracing when running this test.

## Dev observation: 2026-09-08

- Native import succeeded with the supplied five-field session payload.
- The explicit SDK refresh succeeded against real ZITADEL. Both tokens changed,
  the returned access expiry was approximately twelve hours, and the complete
  replacement was saved privately before the subsequent API request.
- A fresh process resumed the saved generation with zero additional refreshes.
- All three native starts stopped before DDS admission with
  `AuthorizationDenied: the principal has no readable Domains`.
- API `POST /service/domains-access-token?purpose=p2p` returned
  `403`, `no readable Domains`. Authenticated userinfo and API organization lookup
  both returned `200`, including with the refreshed credential.
- Policy `POST /orgs/{org_id}/authorization/check` returned `200` with
  `allowed: true` for organization-scoped `domain_metadata_read`, but
  `allowed: false` for the configured Domain. The refreshed token gave the same
  results. This alone does not distinguish a missing Domain/parent mapping from
  an effective Domain-policy denial or a mismatched test Domain.

No peers, advertisements or relay bookings were created. Native relay traffic,
long-running renewal, and Domain-denial/revocation scenarios remain unverified
against dev until a readable test Domain is available. Do not bypass this with
the legacy policy Domain-list shortcut or infer that organization-level access
is sufficient for the Domain-specific check.

Local harness validation: two Rust tests (private atomic replacement/reimport and
unsafe-outcome replay guard), strict Clippy, package formatting, Node syntax,
and `git diff --check` passed. This is evidence for the host's storage behavior,
not a substitute for the blocked end-to-end peer test.
