# Fleet inventory and activity

`AukiFleet` provides read-only observations from DDS inventory and DMS jobs.
`auki-fleet` owns aggregation; `auki-auth` owns credentials, renewal and
persistence; `auki-dms` owns jobs transport. The facade reexports the public
types. No peer, discovery registration, relay booking, worker, scheduler or
polling loop is started. See [Inspect a fleet](../how-to/inspect-fleet.md).

## Views and identity

| Operation | Machines included | Association |
| --- | --- | --- |
| `list` | DDS robots assigned to the selected Domain, including offline robots | `assigned` |
| `list` | DDS nodes joined to observed leased/running tasks in readable jobs for this Domain | `active_task` |
| `compute_pool` / `computePool` | Visible public or dedicated compute candidates, selected by required `mode` | `candidate` |

Machines are joined by DDS machine UUID and DMS `reserved_by`, never name or
Peer ID. Active tasks with no matching inventory remain in `unresolved_activity`
for the Domain view. They do not establish the worker's kind. Conflicting robot
and node identities, duplicate provider identities and wrong-Domain responses
fail closed. Jobs with canceled status may still contain draining active tasks.

Capability filters use exact, case-sensitive arbitrary strings. By default,
any listed capability matches; `match_all_capabilities` selects all. Filters
apply to inventory metadata. Unresolved Domain task references remain available
even when a machine capability filter is present. Infrastructure-only entries
whose capabilities are `/legacy-domain-server/v0` or `/p2p/circuit-relay/v1`
are excluded from the candidate pool. Offline and unstaked nodes can appear:
neither presence nor inclusion proves scheduling eligibility or reserves capacity.

## Snapshot and status

`FleetSnapshot` contains `domain_id`, `view`, `observed_at`, `machines`,
`unresolved_activity`, `sources`, and `complete`. Machine records expose IDs,
organization, name, capabilities, provider mode/status, association, presence,
work state and readable task activity. Optional timestamps serialize as null;
JSON results use snake_case in Rust, Python, Web and Expo. Swift exposes typed
camelCase properties. Dates are RFC 3339 strings in host bindings.

- `presence` is `online`, `offline` or `unknown`. Unrecognized DDS status is
  preserved in `provider_status` and maps to unknown.
- `last_seen_at` and `active_lease_expires_at` are provider timestamps when
  available. DDS does not expose compute last-seen: it stays null.
- `presence_observed_at`, `work_observed_at`, activity `observed_at` and source
  `observed_at` record SDK observation times, not provider liveness timestamps.
- `work_state` is `busy` only with a successful busy-feed observation consistent
  with any attached task/lease observations. Expired or missing attached leases,
  differing assignments and conflicting mode/job references leave it unknown.
- `idle` requires an online machine, a successful complete busy response whose
  coverage includes it, no reported busy entry, no attached active task, and no
  unexpired robot lease. Coverage includes public machines and dedicated machines
  in the successful User grant's organization. Other organizations' dedicated
  machines are not covered by absence from the feed.
- Missing or unsupported busy information yields unknown, including App sessions
  that can read Domain tasks. Busy entries from other Domains may establish only
  a work-state observation; their job/task references are never attached without
  a matching authorized Domain job read.

Each source reports `complete`, `partial`, `denied`, `unsupported`, or
`unavailable`, with observation time, stable codes and optional HTTP status.
`complete` at snapshot level requires all requested sources to complete and no
unresolved Domain activity. It describes those bounded provider reads, not an
atomic view, a freshness guarantee, or scheduling authority. Work may remain
unknown even in a complete snapshot when the source's permission scope does not
cover a machine. Empty successful inventory is distinct from a denied source.

## Permissions and provider contracts

| Read | Authorization and scope |
| --- | --- |
| DDS `GET /api/v1/domains/{id}/robots` | API-issued User or App service token; DDS applies Domain and organization/public-User or App/MAC policy |
| DDS `GET /api/v1/nodes?org=all&staking_status=all` | API-issued User or App service token; public nodes and permitted organization nodes; client selects public/dedicated mode |
| DMS `GET /v1/nodes/busy?mode=all` | DDS-issued User-type Domain write grant; public tasks plus dedicated tasks in the grant organization; no Domain ID in the response |
| DMS jobs list and details | Existing [jobs authorization](jobs.md); Domain write permission is required even for reads |

App credentials are available only on trusted Rust/Python hosts. Apps can read
permitted inventory and Domain jobs, but the current busy route does not support
their grant: its report is `unsupported`. Robot and compute credentials do not
provide fleet browsing in this release.

Imported ZITADEL sessions retain their shared refresh owner and awaited
credential persistence. Ordinary User grants can use inventory reads; a Domain
allowlist disables broad node inventory and permits robot reads only within that
allowlist. App-shaped human viewer grants never enter the broad legacy inventory
routes: these sources report unsupported. Known-Domain activity uses the existing
Domain exchange and remains subject to its provider permissions. No raw OAuth,
viewer-App or peer-purpose credential fallback is attempted for inventory.

Contract source assessment: DDS
[`b27c080`](https://github.com/aukilabs/domain-service/commit/b27c0804a7ff6c99b228c3b8f36665fb3ebea10f)
and DMS
[`06bd863`](https://github.com/aukilabs/domain-manager-service/commit/06bd863a8b8537dabd761b826818844a72593d2c),
checked on 2026-09-17. Source compatibility does not establish deployment support.
Before rollout, verify provider builds, route availability, grants, robot schema,
and browser CORS in the intended approved environment. Local validation is
recorded separately in [fleet validation](../../test-support/fleet-validation.md).

## Bounds, errors and lifecycle

Defaults are a 60-second overall snapshot deadline, at most 4 job pages of 50
items, 100 job-detail requests, 4 concurrent detail requests and 1,024 activity
records. Rust can configure these with `FleetLimits`. DDS reads inherit the
session's `AuthLimits` (15 seconds and 512 KiB by default); DMS reads use the
jobs defaults (30 seconds and 4 MiB per response). Responses are bounded before
JSON decoding. Inventory and busy responses are never silently truncated.

DDS inventory and DMS busy feeds are currently unpaginated. DDS pagination is
tracked in [#385](https://github.com/aukilabs/auki-sdk/issues/385). The DMS job
cursor can skip a row ([#396](https://github.com/aukilabs/auki-sdk/issues/396));
any multi-page traversal is marked `provider_pagination_unreliable`. Cursors
pass through unchanged. Repeated cursors, changing pages, detail/activity limits
and failed optional reads are explicit source codes. Do not treat partial jobs
as proof of no work; only the separately complete busy feed can support idle.

Optional source denials, timeouts, transport errors and oversized responses
retain other usable results. Invalid responses, authentication configuration or
sign-in failures, persistence failure, cancellation and closed clients return
`FleetError`. Host errors preserve `kind`, `code` and available `status`; a
`persistence` code means the host must recover/save the retained credential
snapshot through the existing session flow. No extra OAuth refresh owner exists.

Every call is cancellable: Rust takes a `CancellationToken`, Python uses asyncio
cancellation, Swift accepts `AukiCancellation`, and Web/Expo accept `AbortSignal`.
Await `close()` to cancel and drain the client's requests. Clones share close
state; separately created clients and the shared session remain usable. Close
fleet clients before closing the session. Observations are not cached or polled.

## Binding compatibility

The APIs are additive in Rust, Python and JavaScript. Swift adds an
`AukiSdkError.Fleet(kind:status:code:message:)` case; consumers with exhaustive
error switches must handle it. Regenerate UniFFI output and ship the matching
Swift/Expo XCFramework with its wrappers. Never mix the previous generated FFI
with the new native library. This change adds no wire format, endpoint, protocol
ID or backend mutation contract. Expo supports Web and iOS; Android remains
unimplemented.
