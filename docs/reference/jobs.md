# DMS jobs reference

The `auki-dms` crate's `jobs` feature exposes a portable User/App HTTP client.
`auki-sdk` re-exports it as `AukiDmsJobs`. The existing native worker `client`
feature and `AukiDmsTasks` remain separate; submitting a job does not run a peer or
worker and requires no machine credential.

## Operations

| Client method | DMS API relative to its configured base | Result |
| --- | --- | --- |
| `estimate(spec)` | `POST jobs/estimate` | Decimal-string total and task estimates |
| `submit(spec)` | `POST jobs` | Created job UUID |
| `submit_with_key(spec, key)` | `POST jobs` with `Idempotency-Key` | Created or recovered job UUID; requires a capable deployment |
| `list(query)` | `GET jobs` | Items with task counts and opaque `next_cursor` |
| `get(id)` | `GET jobs/{id}` | Job, task counts, tasks, and result receipts |
| `cancel(id)` | `POST jobs/{id}/cancel` | Job status and update time |
| `close()` | No backend mutation | Aborts and drains this client's HTTP work |

The selected Domain is added to submission, estimation, and list requests. Returned
jobs must match it; detail and cancellation responses must match the requested job.
Job details also check the tasks' and receipts' job IDs. The backend independently
enforces organization and Domain permissions. Jobs are not private to their submitting
App: authorized principals in the same organization/Domain can inspect them.

## Input and defaults

`JobSpec` contains `label`, `tasks`, `edges` (empty by default), `priority` (0),
and `meta` (an empty object). A `JobTaskSpec` contains a label, stage and exact
capability name. Its defaults are public mode, priority 0, three attempts, empty
capability filters/inputs/metadata, and no output prefix. Set dedicated mode for
third-party tasks. `inputs_cids`, `outputs_prefix`, and `meta` retain the provider's
wire semantics; the SDK does not infer or transform worker-specific data formats.

Edges refer to stage names, including stages containing multiple tasks. DMS owns
validation of graphs, admission, worker availability, and pricing. Estimates preserve
decimal strings so host languages do not round credit amounts through floating point.

`JobListQuery` supports limit (1–100, default 50), cursor, job status, capability
filters, and `match_all_capabilities` (false by default). One SDK call returns one
backend page. It neither scans all jobs nor deduplicates or repairs provider cursors.

## Authentication and request policy

User, backend App, and imported ZITADEL sessions reuse their existing `AuthSession`.
Every job operation requires a DDS-issued grant accepted by DMS with write scope.
The audited DMS middleware rejects read-only grants with HTTP 401, including for
listing; the SDK performs its one permitted renewal and then returns that status.
A read grant or successful P2P connection does not authorize jobs. Raw imported
ZITADEL tokens and compute/robot worker credentials are not passed to job endpoints.
App secrets stay on trusted Rust/Python hosts.

DMS URLs require HTTPS, or literal loopback HTTP for local tests. Existing path
prefixes are preserved with or without a trailing slash. Redirects are disabled;
browser requests omit cookies and referrers. Response bodies and bearer tokens are
not included in errors. Network calls have these configurable `JobsLimits` defaults:

| Limit | Default | Allowed |
| --- | --- | --- |
| Entire operation, including auth and one 401 renewal | 30 seconds | Greater than 0, at most 300 seconds |
| Encoded request body or list URL | 1 MiB | 1 byte–16 MiB |
| Streamed response body | 4 MiB | 1 byte–16 MiB |

Native connection establishment also has a five-second timeout. Only an explicit
401 triggers one shared Domain-grant renewal. The client never automatically retries
an ambiguous submission, other HTTP errors, or a failed transport. `JobsError::code()`
and `http_status()` expose recovery categories without leaking response bodies.
Imported-session persistence errors keep their `persistence` category. A timeout,
cancellation, malformed success, or transport/server failure after submission begins
can become `submission_uncertain`. Unkeyed submissions require reconciliation before
submitting again; keyed recovery follows the deployment-gated contract below.

## Recover a keyed submission

Enable keyed submission only after verifying that the target environment has
[NCS #208](https://github.com/aukilabs/network-credit-service/pull/208) and
[DMS #43](https://github.com/aukilabs/domain-manager-service/pull/43), including the
DMS migration, deployed to **all** serving replicas, or equivalent later versions.
The source baselines below predate that contract. Older DMS silently ignores the
header, so a successful keyed call does not prove support; mixed old/new replicas
cannot guarantee deduplication. This SDK does not detect support or enable retries.

Generate a random operation key once and persist it **with the immutable job
specification before the first send**. Keys contain 1–128 visible ASCII characters,
without spaces; invalid keys fail locally without authentication or HTTP work.
The key is a header, not a job field, label, transport peer ID, or credential.

On that verified deployment, `submit_with_key` can recover a lost result by sending
the same key and specification. DMS scopes the key to the verified DDS issuer,
token type, subject, organization and selected Domain. Token renewal preserves
recovery only when that principal scope is unchanged. Refreshing or changing
credentials does not bypass authentication or write permission checks.

| Outcome | SDK result | Caller action |
| --- | --- | --- |
| New job or completed matching replay | Original job UUID | Persist the returned ID |
| Matching submission still processing | `submission_in_progress`, HTTP 409, `retry_after_seconds()` | Wait at least the supplied delay, then retry the same key/spec with a bound |
| Same key, changed request | `http_status`, HTTP 409, no retry hint | Stop; recover the original request rather than replacing its key |
| Transport loss, timeout, cancellation after send, or server failure | `submission_uncertain` with redacted cause/status | Retain key/spec; explicitly retry the same operation when appropriate |
| Insufficient credits | HTTP 402 | The reserved key remains bound to the original request |

Only a keyed submission's HTTP 409 with a valid numeric `Retry-After` becomes
`submission_in_progress`. Missing, malformed, or inaccessible hints remain plain
conflicts. Browsers require CORS permission for `Idempotency-Key` and exposure of
`Retry-After`; DMS #43's permissive CORS layer provides both. Proxies must preserve
them. Other retry hints do not change the SDK's conservative error classification.

DMS compares parsed requests with defaults applied: object key order is irrelevant;
array order and recognized fields matter. Keep the original payload rather than
reconstructing it after an SDK/default change. Keys are retained indefinitely,
including after job cancellation/deletion; replay never starts a replacement job.
New work requires a new key. Canceling the local HTTP operation does not cancel
an accepted job or erase its reservation. Abandoned pending credit reservations
need caller recovery or operator reconciliation; no background sweeper is provided.
See the [provider contract](https://github.com/aukilabs/domain-manager-service/blob/feature/387-job-idempotency/docs/job-submission-idempotency.md).

## Provider compatibility and known constraints

Source audited 2026-09-16: DMS `06bd863` (the same tree advertised by dev `8088111`)
and DDS `b27c080` (the same tree as dev `v0.14.6`). Public version metadata was checked;
cluster image digests were not reverified. Live dev checks covered User, trusted
App and imported ZITADEL in native Rust/Python. Chrome/WASM, macOS Swift and the
iOS Simulator covered User and imported ZITADEL. These checks included execution,
results and running-job cancellation. Expo iOS also exercised the imported session
through its actual JavaScript-to-native bridge. A dedicated dev compute node was
provisioned through the existing API/DDS contracts for the imported session's
organization; see the
[validation record](../../test-support/jobs-validation.md) for the exact boundaries.

- Third-party dedicated capabilities use a configured default price when no explicit
  price exists. Public and reserved Auki names do not use that fallback.
- Submission and estimation require an online compatible worker. Dedicated workers
  match the organization; robots additionally match the Domain. Estimation requires
  enabled credit locking.
- Cancellation is for whole jobs. DMS may mark a job canceled while a task remains
  running until a heartbeat or lease cleanup. There is no user-facing task-cancel API.
- A concurrent heartbeat can cause cancellation to return HTTP 409. Read the current
  job state before explicitly retrying cancellation with a small attempt limit.
  The audited backend's lease-expiry cleanup can leave `credit_released_at` unset
  after cancellation; this requires a backend correction, not an SDK retry loop.
- The audited baseline predates keyed submission. Follow the deployment gate above;
  application labels are not uniqueness keys.
- Older DMS pagination encoded the discarded overflow row as its next cursor and
  then excluded that row, skipping one job between pages. A complete listing
  requires [DMS #42](https://github.com/aukilabs/domain-manager-service/pull/42)
  (tracked in [SDK #396](https://github.com/aukilabs/auki-sdk/issues/396)). The SDK
  passes opaque cursors unchanged and does not detect whether that fix is deployed.
- Current DMS persists a task's stage as its label. Both returned fields are exposed
  as supplied by DMS; the SDK does not pretend to repair persisted labels.

These are provider behaviors, not additional SDK scheduling or pricing rules.
Backend changes and live-test approval remain separate from this SDK implementation.

## Bindings

Each binding exposes jobs from its existing session with explicit Domain selection:
[Python](../../core/bindings/python/auki-sdk-py/README.md),
[Web](../../core/bindings/web/auki-sdk-web/README.md),
[Swift](../../core/bindings/swift/auki-sdk-swift/README.md), and
[Expo Web/iOS](../../core/bindings/expo/README.md). Android is not implemented by the
existing Expo binding. See [Submit and monitor Domain jobs](../how-to/submit-jobs.md)
for Rust examples.

The keyed methods are `submit_with_key(spec, idempotency_key)` in Rust/Python,
`submitWithKey(spec, idempotencyKey, signal?)` in Web/Expo, and
`submitWithKey(spec, idempotencyKey: key, cancellation: token)` in Swift. Rust also
has `submit_with_key_and_cancellation`. Python exposes `retry_after_seconds`;
Web/Expo expose `retryAfterSeconds`; Swift's `Jobs` error includes
`retryAfterSeconds`. These methods never generate keys or retry automatically.
Existing `submit` signatures and unkeyed behavior are unchanged.

**Source compatibility:** Rust `JobsError` adds `SubmissionInProgress`; update
exhaustive matches. Swift `AukiJobsFailureKind` adds `submissionInProgress`, and
`AukiSdkError.Jobs` adds an optional `retryAfterSeconds` associated value before
`message`. Rebuild generated bindings and native libraries together and update
exhaustive Swift error switches/catches. Expo adds a failure-kind union member and
native bridge method; ship its matching native module with the JavaScript update.
Existing endpoint, JSON and token formats are unchanged. See the
[keyed-submission validation](../../test-support/job-idempotency-validation.md) for
this opt-in API and the original
[validation record](../../test-support/jobs-validation.md) for checks and live
validation boundaries.
