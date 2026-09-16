# Auki Core Explorer · Milestones 1–2

A TypeScript/Vite example using the actual generated
[same-module Portable Echo Web wrapper](../portable-echo/web/README.md). Domain
data exploration is read-only and needs no running peer; Echo networking is
optional. It serves Space Grotesk and DM Sans locally and makes no service requests on page load.

## Setup and run

Requires Node 22.18+, Rust 1.89+, wasm-pack 0.13.1 and the
`wasm32-unknown-unknown` Rust target. From this directory:

```sh
npm ci
npm run build
# Terminal 1: optional synthetic loopback services
node tests/fixture.mjs
# Terminal 2: loopback application
npm run dev
```

Open the URL printed by Vite. Defaults explicitly target local fixtures at
`http://127.0.0.1:18114` for API, DDS and DMS. The fixture accepts any synthetic
email/password (for example `fixture@example.test` / `fixture-only`). It never
forwards requests and retains only method/path counters. Do not use real
credentials with fixtures. The UI always labels loopback as
“Local fixtures / synthetic test data”. There is no fallback to shared services.

To use an approved shared environment, open **Connection settings** and explicitly enter aligned API, DDS and DMS
base URLs, then sign in. Non-loopback bases require HTTPS; embedded credentials,
query strings and fragments are rejected. Data reads do not use DMS. Optional outbound-only networking starts no browser relay booking;
the remote robot owns its relay booking. Provider deployments
must support the existing SDK contracts; synthetic tests do not establish live
compatibility. No wire formats, Rust SDK code or backend contracts were changed.

Passwords are cleared immediately after submission. Only the installation client
ID is stored in localStorage. Tokens remain owned by the in-memory SDK session.
Errors show fixed messages, never backend bodies. All inspectors use text nodes;
JSON credential fields and bearer/JWT patterns are redacted recursively. Download
is an explicit original-byte `.bin` attachment; downloaded bytes are not redacted.

## Guided chapters

Choose a Domain in **Choose or change Domain**, or expand the known-UUID option.
**Overview** shows actual Domain metadata and independently loaded portals and
poses. **Data** shows stored records; **Apply filters** submits name, type and ID
filters to the server. Technical disclosures retain complete redacted responses.
Preview and original-byte download remain explicit actions. On mobile,
**Jump to selected record** moves focus to its details.

**Networking** shows only the current step body from the real peer and confirmed
target state.
Choose a discovered candidate route or expand **Advanced · enter a manual route**.
Confirm a manual target with **Use manual target**; editing its fields keeps the
form open.
**Choose another target** returns to selection, and Stop remains available.
Send is enabled only for a ready peer, a confirmed target and 1–1024 UTF-8 bytes of nonblank
text. Changing the target or message clears the old response and invalidates any
in-flight result for that selection. Stop and reselection remain available.
Short Peer IDs identify candidates and the selected target; Technical details
retain full IDs, routes and receipt metadata. Verified payload text and byte count
appear only after a successful receipt.
Chapter navigation keeps the selected Domain, session and running peer. Domain
changes and logout keep the existing awaited cleanup behavior.

The UI uses `/brand/tokens.css`, the official `/brand/auki-logo.svg`, and local
OFL fonts; it does not load fonts from a CDN. Final Guided verification on
`develop` (`be769a5`) passed 30 JavaScript unit tests, 6 Python runner tests,
3 fixture/config tests, both real Chromium suites and five normal robot exits.
Fresh WASM/Python/relay builds, strict relay lint, formatting and the npm audit
passed. Independent final review found no consequential blockers; it also
verified the delayed/concurrent logout regression and retained M1/M2 assertions.
Desktop/mobile connected, empty, denied, failed and verified-response states
were visually inspected with fonts loaded locally. These are local synthetic
fixtures exercising real SDK paths, not deployed-provider compatibility checks.

Actual app screenshots: [Overview](screenshots/overview.png),
[Data](screenshots/domain-explorer.png), [verified Echo](screenshots/networking.png)
and [mobile Echo](screenshots/networking-mobile.png).

## Verification

The local suite exercises the generated SDK/WASM against synthetic loopback
services. The current integrated example passed the M1 Chromium regressions and
the real browser-to-Python network suite with freshly built artifacts, including
normal robot process exits. This branch includes the SDK shutdown fix from
[PR394](https://github.com/aukilabs/auki-sdk/pull/394).
See [network setup and validation limits](tests/network-README.md).

```sh
npm run typecheck
npm test
npm run build
# Install once if Chromium is not already available:
npx playwright install chromium
npm run test:browser
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

- Writes, delete, streaming transfers above 8 MiB and spatial visualization.
- Imported ZITADEL/PKCE login and browser machine credentials.
- Jobs and fleet tooling: this example makes no claim that corresponding Web
  SDK APIs exist.
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
