# Auki Core Explorer · data, Echo and Jobs Playground

A TypeScript/Vite example using the actual generated
[same-module Portable Echo Web wrapper](../portable-echo/web/README.md). Domain
data browsing, explicit single-file uploads and DMS jobs need no running peer; Echo networking is
optional. It serves Space Grotesk and DM Sans locally and makes no service requests on page load.

## Setup and run

Requires Node 22.18+, Rust 1.89+, wasm-pack 0.13.1 and the
`wasm32-unknown-unknown` Rust target. From this directory:

```sh
npm ci
npm run build
# Terminal 1: optional synthetic loopback services
node tests/fixture.mjs
# Terminal 2: serve the built application
npx vite preview --host 127.0.0.1
```

Open the URL printed by Vite (normally `http://127.0.0.1:4173`). Rebuild after
changing app code; preview is not hot reload. This uses the existing preview
workflow without changing Vite's development-server filesystem policy.
The editable defaults target **Dev**: API `https://api.dev.aukiverse.com/`,
DDS `https://dds.dev.aukiverse.com/`, and DMS `https://dms.dev.aukiverse.com/v1/`.
Loading the app or editing **Connection settings** makes no service requests;
only explicit **Sign in** contacts the configured services. The Dev label describes
these exact configured bases, not service availability or deployed compatibility.
Other configurations show **Custom / mixed environment**; all-loopback bases show
**Local fixtures / synthetic test data**. There is no environment fallback.

For the optional local fixture above, open **Connection settings** and replace
**all three URLs** with `http://127.0.0.1:18114` before signing in. The fixture
accepts any synthetic email/password (for example `fixture@example.test` /
`fixture-only`). It never forwards requests and retains only synthetic data and
request observations in memory. Do not use real credentials with fixtures.
Browser harnesses explicitly override all three URLs and block nonlocal requests.

For an approved shared environment, verify the aligned API, DDS and DMS bases in
**Connection settings**, then sign in.
Non-loopback bases require HTTPS; embedded credentials,
query strings and fragments are rejected. Data reads do not use DMS. Optional outbound-only networking starts no browser relay booking;
the remote robot owns its relay booking. Provider deployments
must support the existing SDK contracts; synthetic tests do not establish live
compatibility. No wire formats, Rust SDK code or backend contracts were changed.

Passwords are cleared immediately after submission. Only the installation client
ID is stored in localStorage. Tokens remain owned by the in-memory SDK session.
Errors show fixed messages, never backend bodies. All inspectors use text nodes;
JSON credential fields and bearer/JWT patterns are redacted recursively. Download
is an explicit original-byte `.bin` attachment; downloaded bytes are not redacted.

## Focused screens

The viewport-oriented application shows one current task, rather than appending
details below a long page. **Data**, **Jobs**, **Fleet**, **Poses/Portals** and **Network** are the five primary workspaces. Fleet has All, Dedicated and Public views; Poses/Portals contains Domain information, Portals and Poses. Domain selection opens Data. One compact header carries environment and selected Domain; lists and details replace one another in the dark workspace. Long content scrolls internally and short windows retain reachable controls. Shared rows, forms, metadata and status treatments use restrained orange actions.

Buttons open the Domain picker, connection settings, advanced filters, record,
preview and technical screens. Domain paging and known-UUID selection remain
available. Name/type/ID filters still submit to the server; record selection,
preview and original-byte download remain explicit. Back returns to the originating
task without discarding the Data filter. Settings cannot silently retarget an
already signed-in session.

**Network** shows the current real peer/target step. Manual route entry,
technical context and verified results have dedicated sub-screens. Confirm a manual
target with **Use manual target**; editing does not hide the form. Stop remains
available. Send requires a ready peer, confirmed target and 1–1024 UTF-8 bytes of
nonblank text. Editing the target/message invalidates old and in-flight results.
Technical screens retain redacted full IDs, routes and receipt metadata.
Primary navigation preserves the selected Domain, session, pending reads and
running peer. Domain changes and logout preserve awaited cleanup.

## Jobs Playground

Jobs use the real `session.jobs(domainId)` Web SDK, not P2P task dispatch or a
browser control server. The paired [demo workers](workers/README.md) reuse the
SDK's existing compute/robot handlers:

- **Compute:** read a small UTF-8 record and create its uppercase version.
- **Robot inspection:** read a small record and create a JSON byte-count/SHA-256
  report. This is simulated inspection, with no hardware or shell actions.

After selecting a Domain, open **Jobs**. The dashboard automatically performs one read-only worker discovery and loads one recent-jobs page for an unambiguous installation. Use **Refresh workers** to repeat discovery or **Workers · details** to inspect sources and choose among multiple installations.
The real `session.fleet(domainId)` client reads assigned robots with `list({})`
and organization-dedicated compute candidates with `computePool({mode: "dedicated"})`.
Only exact lowercase canonical `/examples/compute-robot/{UUID}/{compute|robot}/v1`
capabilities form installations. A single installation with an unambiguous role
enables **New job** without UUID setup. Multiple installations require an
explicit readable choice; roles are never paired across installations. Missing
or duplicate executors disable that role, including exact-capability collisions
across machine kinds. Both robot and compute inventories must be complete to
establish uniqueness; activity denials alone do not block selection. Partial inventory cannot establish a
unique executor; denied busy/activity sources retain useful inventory with unknown
work observations. Online/offline/busy/unknown are observations, not reservations.
Source reports, public IDs and unresolved activity are available under details.

Discovery runs once on entry, is bounded by the SDK and is never polled. Domain switches and
logout abort and drain Fleet before session closure, then free the WASM handle.
Cleanup errors remain visible. Configuration stays in memory for the current
Domain/session. One unresolved submission survives Domain changes for reconciliation
on returning; new submissions remain blocked across that session. Discovery and
navigation do not erase recovery. Rediscovery invalidates old reviews and new-job
eligibility until an unambiguous installation is selected again; Back cannot
restore stale executors. Historical executor IDs remain available for history and
receipt verification. Logout or page reload erases in-memory recovery.
Never paste machine credentials into the browser. An operator must activate workers.

Choose **New job**, then **Uppercase text** or **Inspect file** (simulated robot). Search existing Domain records by exact name, select a record and inspect its bounded preview. The picker displays at most 100 matches; narrow the search when needed. Record UUID entry is under Advanced. You can also open Jobs from the selected Data record. Upload a small input through the existing explicit upload flow if needed.
The demo input limit is **64 KiB**, independent of the upload limit. Review the
configured environment, Domain, input, capability, expected executor, output naming
and DMS decimal-string credit estimate before confirming. Compute inputs must be
valid UTF-8. The existing worker also caps output at 64 KiB: Unicode uppercase expansion can exceed this cap even when input is valid and estimation succeeds (for example 11,000 U+0390 characters expand from 22,000 to 66,000 UTF-8 bytes in the Python handler). Such a job can deterministically fail; billing consequences are unknown. This pre-existing controller/worker limit is deferred, not changed by the dashboard repair. Each job has one dedicated task with one maximum attempt. Capability
names are `/examples/compute-robot/{installationId}/{role}/v1`; this is capability
routing, not an arbitrary worker-ID selector.

The detail screen leads with status, input and verified output, including a bounded preview and **Open in Data**. Technical details contain worker progress/events and raw receipts. Recent jobs are the dashboard, with no separate history destination. Action/input/result names observed in this session enrich rows; provider history does not contain those fields, so unseen jobs show “Demo job” and “Inspect details”. Cached enrichment is discarded when the job or executor configuration changes. Human-readable times use provider timestamps. No per-row metadata requests are made. Executor checks use returned task/receipt information and
the configured expected ID; configuration alone never proves execution. Accepted
result references open through the existing selected-Domain Data client. To chain
operations, open the compute result and explicitly start a robot inspection of it.
There is no automatic multi-stage pipeline.

All job endpoints, including history and detail reads, require **Domain write
authority**. Estimates and submission require an eligible online worker and enabled
credit locking. A read-only data session or successful Echo is not sufficient.

Submission is explicit and is never automatically repeated by the application.
A unique label helps reconciliation but is **not** an idempotency key. A lost,
malformed or ambiguous response can mean a job was already created and charged;
inspect/reconcile it rather than resubmitting blindly. History reads one bounded
provider page at a time and passes cursors unchanged. Known provider pagination
issue [#396](https://github.com/aukilabs/auki-sdk/issues/396) means a missing history
entry does not prove the job was never submitted.

Cancellation requests cancellation of the whole job, not an individual task.
A canceled job may still have a running task, and an unset `credit_released_at`
does not prove funds were released. Closing the client, changing Domains or logging
out aborts/drains local requests; it does **not** cancel remote jobs or undo writes.

Worker configuration examples and supervised-service templates are inert files.
This PR does not provision identities, install/start services, spend live credits
or validate deployed-provider execution. Activation requires separate explicit
environment/Domain/identity approval and a finite test-credit budget. See the
[worker runbook](workers/README.md) for validation, start/stop and rollback boundaries.

## Explicit single-file upload

From **Data**, choose **Upload**, select a file and an SDK data type, then review
the destination Domain, configured environment, generated unique name and size.
Only the explicit confirmation starts a real `AukiDomainData.write` request.
Data types are application labels, not MIME types: `/` and other server-disallowed
punctuation are rejected. The default `core-explorer.file.v1` is valid without editing. This tranche uses buffered named creation, capped at
**8 MiB**, not multipart streaming or replacement by ID. Existing-name collisions
are errors, not permission to overwrite.

The server authorizes writes independently of reads. Upload verification fetches
the returned record metadata with `get`; a completed write whose verification
cannot be read is not labelled verified. The UI shows operation stages, not an
invented percentage. There is no application-level automatic retry, overwrite or delete action. Browser transport can retransmit a single JavaScript fetch after a dropped connection; the generated target and bytes remain identical. Server no-overwrite uniqueness is not claimed to be atomic.
After cancellation, timeout or a lost response, a server record may already exist.
Keep the generated target and inspect the Domain before explicitly starting another
upload. Cancellation does not promise rollback. Domain changes/logout abort and
invalidate old work, suppressing late results. Within the same session an uncertain target remains available after changing Domains. Logout completely clears file, type, destination, target and returned metadata before another account signs in. Review starts at the top with destination and file visible; returned metadata opens a separate redacted technical screen with Back.

The UI uses `/brand/tokens.css`, the official `/brand/auki-logo.svg`, and local
OFL fonts; it does not load fonts from a CDN. Local checks include 166 TypeScript
unit tests, 17 fixture/config tests, 25 Jobs worker tests and 6 idle-Echo robot tests.
Real generated SDK/WASM browsing, binary upload/download and Jobs use synthetic
loopback services; the separate Echo suite uses a real local Python robot. Jobs
browser tests do not pretend their synthetic DMS provider executed native workers.
Separate native-SDK probes exercised both actual Python handlers' bounded data
reads/writes and exact bytes/hashes against loopback fixtures, using synthetic task
contexts and User grants rather than real machine leases. Native credential/runtime
construction and awaited closure also passed under the service template's host
hardening and resource caps, without starting registration or task polling.
Fixture checks alone do not establish deployed-provider compatibility or live
availability. During the preceding Fleet tranche, an explicitly authorized Chromium run against dev verified
Fleet discovery of the hosted demo pair, matching installation/executor IDs, both
action choices without manual worker UUID entry, and normal logout. Its network
guard allowed read-only service requests and authentication exchanges only: no live
jobs, estimates, uploads or task outputs were created. This proves live discovery,
not leased-job execution or billing behavior.

[Complete control-panel screenshot gallery](screenshots/control-panel/README.md):
148 real-app captures covering all 15 routes, Fleet filters/details, and every Jobs, Upload and Network
step on desktop and mobile, plus narrow/short viewport captures. Includes sign-in,
settings, Domain selection, Data/details/preview/filter, Poses/Portals overview, portals and poses,
upload confirmation/progress/result, workers, input selection, cost confirmation,
job submission/result/recovery, and connected Echo/technical views.
[Coverage manifest](screenshots/control-panel/manifest.json) records each route,
flow step, viewport and image hash. Pre-login captures show Dev defaults without
contacting services; signed-in captures use local synthetic fixtures.
Jobs screenshots are not deployed worker or live billing evidence. Network result
captures use the real local Chromium/WASM-to-Python Echo fixture.
Older screenshots outside this gallery describe earlier UI milestones.

## Verification

The local suite exercises the generated SDK/WASM against synthetic loopback
services. Unit and fixture checks are separate from browser/runtime acceptance.
The browser upload suite counts JavaScript write submissions separately from
server-observed transport attempts and checks identical targets/bytes, no fixture
replacement, uncertainty, and account A → logout → account B state clearing.
See [network setup and validation limits](tests/network-README.md).

```sh
npm run typecheck
npm test
npm run build
# Install once if Chromium is not already available:
npx playwright install chromium
npm run test:browser
npm run test:upload
npm run test:jobs-fixture
npm run test:workers
npm run test:jobs
```

Browser tests start the production Vite preview on `127.0.0.1:18116` and HTTP
fixtures on `127.0.0.1:18114` plus one ephemeral loopback Domain Server port.
They launch real Chromium and the built generated SDK/WASM, block and fail on
nonlocal browser requests, and stop both servers and Chromium on completion.
The host launch uses `--no-sandbox` and `--disable-dev-shm-usage`, suitable for
this isolated local test harness; do not point this harness at untrusted sites.
Screenshots are saved under ignored `test-artifacts/` (`signed-out.png`,
`domain-data.png`, `mobile.png`). Unit doubles only test closure ordering;
browser integration does not stub or replace the SDK. Cancellation tests hold
HTTP responses until the server observes an abort, with bounded event-based
synchronization; completing a delayed response cannot satisfy these assertions.

## Coverage and public API mapping

| Feature | Public SDK call | Proof |
| --- | --- | --- |
| Explicit User login | `AukiUserSession.loginWithEnvironment` | Chromium failure, success, recovery after logout and cancelled late login |
| Server Domain pagination | `session.domains().list({ limit, offset }, signal)` | Chromium offset 10, previous/empty pages, cleared label on failure |
| Domain metadata | `DomainPage.domains` | Chromium list/selected summary; known UUID works independently |
| Portals | `session.domains().portals(id, signal)` | Chromium metadata and independently denied read |
| Poses | `session.data(id).poses(signal)` | Chromium pose fields remain available when portals denied |
| Data name/type/IDs filters | `data.list(query, signal)` | Chromium matching and empty results |
| Record metadata | `data.get(id, signal)` | Chromium inspector |
| Preview and download | `data.read(id, signal)` | Chromium inert/redacted JSON, 64 KiB display cap, >8 MiB failure, original download bytes |
| Explicit named upload | `data.write(target, bytes, signal)` then `data.get(id, signal)` | Local real-WASM upload browser suite, separate write denials and completion/verification states |
| Jobs Playground | `session.jobs(domainId).estimate/submit/get/list/cancel` | Real-WASM synthetic HTTP suite, exact review, executor/result validation, uncertainty, cancellation and session/Domain isolation |
| Cleanup | `data.close()`, then `session.close()` | Unit awaited order; Chromium server-observed aborts during pending A→B→A switches and logout |
| Validation/redaction/timeout | App helpers around public APIs | Deterministic unit tests |

Copy controls expose Domain, record and portal IDs. Each read has independent
loading/empty/denied/error presentation; refresh and retry controls reissue reads.
Domain pagination uses SDK totals, which can change between requests. Data,
portal and pose lists are bounded SDK responses without invented pagination.
Known UUID metadata is limited to the current list page: the public binding has
no standalone Domain detail lookup. The inspector labels this explicitly.

## Limits and lifecycle

Buffered download is capped at the SDK default **8 MiB**. Preview parses the full
SDK-bounded buffer and recursively redacts JSON before limiting displayed UTF-8
text (including the truncation label) to **64 KiB**. Malformed or incomplete
JSON-looking content is withheld. Other text uses credential-label and
escape-aware quoted-value redaction. Binary content is shown as decoded text only;
there is no media or executable HTML rendering. Read operations receive an
AbortSignal with a 15-second app deadline (the SDK also bounds its requests).
Domain/filter/record changes invalidate old results and abort pending reads.
Domain switches await previous data-client closure before creating a new client.
Logout cancels reads, awaits data clients and networking cleanup, then closes the shared session, including when data cleanup fails.

Login has no public AbortSignal. Cancel or the 30-second app deadline invalidates
the pending login; any eventual session is immediately closed. Browser pagehide
attempts cleanup, but a page/process exit cannot guarantee awaited completion;
use the explicit logout button for awaited cleanup. Networking starts only on explicit request.
The browser neither advertises nor books a relay; it uses the remote peer’s relay route.

The production Vite build currently includes development-profile WASM to match
existing example builds and keep iteration bounded; the generated module is about
30 MB before compression. Release-profile size optimization is future work.

## Future work and explicit exclusions

- Overwrite/delete controls, streaming transfers above 8 MiB and spatial visualization.
- Imported ZITADEL/PKCE login and browser machine credentials.
- One-click multi-stage job graphs.
- Live provider/deployment validation, permission matrices across real roles,
  and additional browsers/platforms need separate approved environments.
- No existing CI workflow is present in this checkout. CI integration is left
  for parent review rather than introducing a new repository-wide CI policy.

See [Domain data](../../../docs/how-to/domain-data.md) and
[limits and permissions](../../../docs/reference/domain-data.md).

## Optional Networking (M2)

After selecting a Domain, choose **Start networking**. This uses the same
in-memory User session and generated WASM module as all M1 data methods.
`startPeerWithDiscovery(domain, DiscoverOnly, OutboundOnly)` uses the explicitly
configured login DDS endpoint. No separate discovery endpoint override is exposed
by this Web contract. Each start creates an ephemeral transport identity.

States are `stopped`, `starting`, `ready`, `stopping`, and `failed`. Ready means
only that the local peer started; the binding exposes no detailed service or
transport status subscription. `waitStopped()` detects terminal peer failure.
Use explicit stop/start to retry. Startup has a 30-second application deadline;
discovery and diagnostics have 15-second deadlines. Nonabortable calls remain
owned through settlement: stopping waits for late startup cleanup, peer shutdown,
and pending calls before freeing handles or closing the shared session. A deadline
invalidates results immediately but does not promise cleanup completes within it.
Switching Domains stops the previous peer and never starts another automatically.

**Discover Echo candidates** calls `discoverProtocol()` and displays advertised
peer IDs, routes, source and expiry. These are candidates, not online/authorized
robots or Fleet inventory. Select a WSS relay route or enter a target manually:
`/dns4/FQDN/tcp/port/wss/p2p/relay/p2p-circuit/p2p/target`.
**Send verified Echo** calls `AukiEchoClient.sendExact()` once with 1–1024 UTF-8
bytes. Only a successful exact response appears in the verified-results panel;
received text uses the existing credential-redaction helper and inert text nodes.
There is no task dispatch, robot control, inbound handler or automatic retry.
The outbound client has no endpoint `close()` API; pending calls settle after
peer shutdown, then client and peer handles are freed.

Prepare the Python extension, local relay and combined WASM using the
[network harness setup](tests/network-README.md), then run:

```sh
npm run test:robot
node --test tests/network-config.test.mjs tests/network-fixture.test.mjs
SHUTDOWN_REPEATS=3 node tests/network-minimal.mjs
npm run test:network
```

`ws` is a development-only harness dependency; no production dependency was added.
The compiled robot, relay and browser gates run separately from
the narrow unit/fixture checks. The M1 browser suite remains loopback-only and exercises the
combined generated WASM plus networking-off data reads; it does not prove a
browser-to-robot roundtrip. Provider deployment compatibility remains unverified.

Fleet validation uses provider-shaped DDS robot/node and DMS busy/jobs routes,
with the rebuilt Web WASM binding (no frontend SDK substitute):

```sh
npm run typecheck
npm test
node --test tests/jobs-fixture.test.mjs tests/fleet-fixture.test.mjs
npm run test:fleet
```

`test:fleet` starts a loopback Vite server without rebuilding WASM; the parent must
supply the matching generated artifact first. The existing `test:jobs` suite
retains all submission, receipt, uncertainty and lifecycle assertions and now
discovers its workers through the same provider fixture. Its preview assets must
be rebuilt by the parent. Screenshots and approved live read-only discovery are
separate parent checks; local fixture success does not establish deployed support.

## Focused control panel rewrite

The presentation uses local fonts and a dark, terminal-inspired GUI. Back restores
Data filters, focus and scroll; Escape follows the visible local Back action or
returns from a secondary screen. Poses/Portals retains the overview, portal and pose
capabilities. Network keeps the same peer across navigation. Jobs rendering remains
request-free; visible entry and explicit actions own reads. Retained uncertain
submissions keep Recent jobs reachable, and discovery settling while hidden defers
history activation until Jobs is visible. Inspect file produces the real JSON
byte-count/SHA-256 report, never simulated line counts. Estimate review shows the
input preview, destination, environment and decimal-string credits before submission.

The focused rewrite and Fleet addition passed the production build, unit/type/config
checks and real-WASM loopback browser acceptance. The current control-panel gallery
above records the verified layout; screenshots outside that gallery are historical.

## Fleet screen

Primary navigation is Data · Jobs · Fleet · Poses/Portals · Network. Fleet is a
read-only view of Domain robots and visible compute candidates. Choose **Refresh**
to make one bounded Domain inventory read and dedicated/public compute-pool reads.
All is the default; Dedicated and Public filter the provider mode. Unknown modes
appear only in All. Unknown-mode compute candidates excluded by the SDK's required
pool-mode filter cannot be discovered through that API; unknown modes returned by
Domain inventory remain visible. This view does not imply candidate assignment,
capacity, eligibility or permission to dispatch work.

UUID joins preserve Domain task observations; conflicting presence, mode or work
observations become Unknown. Machine details show capabilities and authorized work;
Details retains original observations, timestamps, source diagnostics and unresolved
activity. Partial/denied reads remain distinguishable from a successful empty view.
Back restores the filter, list scroll and selected machine focus. View in Fleet
from Jobs retains the current job flow, with a return link.

Fleet owns an independent client per refresh, a 15-second abort deadline, generation
fences and awaited close/free. Refresh replacement, hidden navigation, Domain
changes and logout abort/drain pending reads. There is no polling or automatic read
on entry. The real-WASM loopback browser suite passed all filters, details, source
failures, recovery and five-item navigation, including 320×568 and 640×360 viewports.
It also verifies Fleet-owned reads abort on Domain change/logout, no stale Domain
machines return, and uncertain Jobs survive Fleet visits without resubmission.
The resource-capped full suite and independent review passed; the current gallery
includes Fleet screenshots. These fixture results do not prove live availability.
