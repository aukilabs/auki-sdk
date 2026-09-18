# Discover Domains by effective permissions

DDS is the source of truth for Domain existence, ownership and visibility.
`discover` lists the DDS catalog and previews effective data/pose permissions.
It neither issues Domain tokens nor bills access for picker entries. Imported
ZITADEL sessions go directly to DDS; API's Domain store is not consulted.

## Rust

```rust
use auki_sdk::{AukiDomains, DomainDiscoveryQuery, DomainPermission};
use tokio_util::sync::CancellationToken;

let domains = AukiDomains::new(credential.clone());
let cancel = CancellationToken::new();
let mut query = DomainDiscoveryQuery {
    allows: vec![DomainPermission::DataRead],
    ..Default::default()
};
loop {
    let page = domains.discover(&query, &cancel).await?;
    for entry in page.domains {
        show_choice(entry.domain, entry.permissions);
    }
    let Some(cursor) = page.next_cursor else { break };
    query.cursor = Some(cursor);
}
```

`AukiDomains::new` requires no running peer. The application's picker owns the
choice; listing never selects the first Domain automatically.

## Python, Web, Swift and Expo

Python:

```python
page = await session.domains().discover(allows=["domain-data:r"], limit=50)
while page["next_cursor"] is not None:
    show_choices(page["domains"])
    page = await session.domains().discover(
        allows=["domain-data:r"], limit=50, cursor=page["next_cursor"]
    )
show_choices(page["domains"])
```

Web:

```ts
const domains = session.domains();
const page = await domains.discover({ allows: ["domain-data:r"], limit: 50 }, signal);
// Continue with { ...query, cursor: page.next_cursor } when non-null.
domains.free();
```

Swift:

```swift
let page = try await session.domains().discover(
    query: AukiDomainDiscoveryQuery(limit: 50, allows: ["domain-data:r"])
)
// Entries contain .domain and .permissions; continuation is .nextCursor.
```

Expo Web/iOS:

```ts
const page = await domains(session).discover({ allows: ["domain-data:r"] }, signal);
```

All bindings retain shared refresh ownership, awaited credential persistence,
cancellation and session shutdown. No refresh token is sent to DDS. Android
remains unsupported.

## Page and permission semantics

- `organization` defaults to `own`; imported users can request only their own
  organization. Local User/App routes retain DDS access-control rules.
- `limit` is 1–100 and bounds the number of DDS candidates examined. A filtered
  page may be empty and still carry `next_cursor`; continue until it is absent.
- `allows` is an intersection of `domain-data:r/w/d` and `pose:r/w/d`, expressed
  as individual strings, for example `["domain-data:r", "domain-data:w"]`.
  Unknown permissions fail. Metadata visibility is checked separately.
- A cursor is opaque and bound to the principal and filters. It is not authority
  or a snapshot. Denial, transfer or revocation can change later pages.
- Response entries include effective atomic permissions after staking policy.
  Actual authorization is rechecked on use. No total count is invented.

DDS signs imported human data grants as `zitadel-user-access`, preserving the
trusted issuer and opaque subject. These grants are specific to one Domain and
contain atomic metadata/data/pose scopes. They expire within five minutes and
no later than the upstream access token. They confer no machine-management,
P2P admission or DMS task authority. Policy failures fail the whole page; 403
and 503 never cause a broader credential retry.

Legacy `list`/`accessible_domains` APIs remain for earlier provider profiles.
They lack these permission semantics. New identity-only API viewers must use
`discover`; the SDK returns a clear configuration error on those legacy methods.

## Portal pages

Rust exposes `for_portal_page` and `portals_page`; Python uses the same names,
Web/Expo use `forPortalPage` and `portalsPage`, and Swift uses generated camelCase
methods. Pages contain `items`, `next_cursor` and `paginated`. An older provider
may return a bounded complete first response with `paginated=false`; continuing
requires versioned pagination acknowledgement. Existing complete-list methods
remain available. Imported portal-to-Domain association lookup is unsupported;
selected-Domain portal reads require pose read authority.

## Provider rollout

Source changes are coordinated through
[DDS #569](https://github.com/aukilabs/domain-service/pull/569) and
[SDK #404](https://github.com/aukilabs/auki-sdk/pull/404), with API and policy
companions linked there. Source support is not deployed support.

1. Release DDS's additive catalog reader (`2429670`) and the policy App reader.
   Configure a dedicated server-side App and DDS's exact catalog App allowlist.
2. Verify complete policy reconciliation, including a DDS-only Domain absent
   from API and more than one catalog page. This replaces the legacy machine
   PAT → viewer-shaped App listing dependency.
3. Release API's `zitadel-identity` viewer profile and DDS human discovery/auth
   and App/operator hardening. Keep issuer, audience, policy URL, model and
   permission configuration aligned in the same environment.
4. Update SDK consumers only after every serving DDS replica supports
   `/api/v1/domain-discovery/zitadel` and `/api/v1/domains/{id}/auth/zitadel`.
   Imported data now requires these routes and does not fall back to broad old
   role grants. Discovery also requires its provider; it never substitutes a
   legacy visibility-only response.
5. Run approved live restricted-user discovery/read/denied-write/foreign-Domain,
   renewal, persistence and cleanup checks, recording exact provider/SDK SHAs.

No deployment or live account/machine changes are part of this source work.
Domain Server data-metadata/pose pagination remains a separate portion of #385.

Imported job submission remains a separate legacy operator contract. The SDK
uses `domain_job_access` and a separate grant cache, preserving the API/DDS User
grant required by DMS. Restricted human data grants cannot satisfy that contract,
even if they permit a data write. DMS still verifies its required job scopes;
this does not introduce policy-derived task authority. The shared session remains
the only refresh/persistence owner. Neither path enumerates API's Domain table.
