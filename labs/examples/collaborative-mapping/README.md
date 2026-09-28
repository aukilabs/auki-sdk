# Collaborative mapping in two browsers

A relay-only web experiment: each peer places named, simulated Portals on its
own grid. A shared name aligns the maps, and both browsers show the same union
in the same explicitly selected frame. All mapping code runs in browser WASM;
there is no application server or native mapping process.

## Run

Requires Rust 1.89+, `wasm32-unknown-unknown`, wasm-pack 0.13.1, Node 20.19+
(on 20.x) or 22.12+, and a current Chromium browser. From the repository root:

```sh
cd labs/examples/collaborative-mapping/web
npm ci
npm run dev
```

Open the printed URL in two tabs or browsers. The app has no offline mode.
Starting the local Vite server does not make the backend local. Before signing
in against shared services, obtain approval for the environment, User account,
Domain, relay bookings, DDS discovery advertisement/lookups, P2P map publication/read operations, and cleanup, following
[the first-peer tutorial](../../../docs/tutorials/first-peer.md). This example
never writes Domain Server records, provisions nodes, or submits DMS tasks.

1. Select **Shared development services**, or choose **Custom environment** and
   enter matching API, DDS, and DMS base URLs. Sign in with a User account.
2. Select the same Domain and demo session in both tabs. Click **Start relay peer**
   in each. Both peers book inbound WSS relay routes and enable DDS
   **DiscoverAndAdvertise** against the session's configured DDS environment.
3. Each browser automatically discovers same-Domain component peers, authenticates
   their exact WSS relay routes, and checks Catalog metadata for the same demo
   session. With two matching peers, both subscribe to the other's map without
   exchanging cards or clicking Connect. Empty maps are discoverable too.
   If several peers share the session, choose your partner in the displayed picker
   (or use a unique session for your pair). The demo still supports two peers.
4. In A, place `apple` at `(1, 2)`. In B, place `banana` at `(4, 5)`.
   Each main grid stays independent; a separate preview shows the partner's map.
5. In A, place `bridge` at `(0, 0)`. In B, place `bridge` at `(10, 20)`.
   Both now show three Portals, including one shared `bridge`. The display frame
   belongs to the lexicographically smaller Peer ID, so its exact coordinates
   depend on which browser owns that ID. Both browsers agree on them.
6. Add more Portals in either view. Clicking the shared grid is converted back
   into that peer's original map frame before publishing. Reusing your own name
   moves the existing Portal. Shared Portal names are case-sensitive.
7. Click **Stop peer & reset map** in both tabs before closing them. This awaits
   subscription cancellation, endpoint closure and peer shutdown to release
   relay bookings and removes the discovery registration. **Sign out** also closes the User session.

Maps are in memory only. Restarting gives a fresh peer and Product, which is
rediscovered and validated against the same Domain/session. Closing a browser abruptly cannot promise awaited cleanup. There is no
persistence, remote edit permission, or single-Portal delete in this example.

## Components and spatial contract

- `DemoMap` owns a `ComponentRuntime` and an existing
  [`MapComponent`](../../auki-scenegraph/README.md). Local edits invoke its
  authorized `upsert_qr` operation with the exact current snapshot reference.
- The WASM adapter mounts [`ComponentProtocolEndpoint`](../../auki-component-protocol/README.md),
  exports only the snapshot Product, checks the remote Catalog and subscribes
  with `LatestExisting`. It exports no remote editing Operables.
- `MapAlignmentChecker` consumes validated, exact snapshot references. The
  host derives a union for rendering, preserving source placements and
  contributors. Derived views are never republished as new evidence.
- Every source map declares an independent XY frame: right-handed, Z-up, meters.
  Every anchor and map alignment carries explicit source/destination frame IDs.
- These are **simulated** Portal identities, not DDS Portal records. An exact
  name and demo session deterministically derive a UUIDv8 from SHA-256. Portal
  frames are fixed-heading, +X right, +Y up, +Z out of the grid, with 0.25 m side
  length. Therefore one shared Portal determines translation; there is no
  rotation estimation or claim that arbitrary real-world names establish alignment.
- Multiple shared Portals must agree. Contradictory placements withdraw alignment
  and restore separate views, retaining the source maps for correction.
- At most 128 Portals per peer; names/session names use 1–64 ASCII letters,
  digits, spaces, underscores or hyphens without leading/trailing spaces.
  Positions are integer cells within ±10,000 m in each source frame.
- Publication timestamps use a named, monotonic per-publication logical clock.
  These synthetic placements do not represent time-varying physical observations.

Domain discovery plus session metadata selects the partner this app consumes.
A session name is not a secret or a Product read ACL: exported synthetic snapshots are readable by authenticated peers
admitted to the selected Domain. Do not use this demo to publish sensitive maps.
P2P admission never grants map mutation or Domain Server write authority.

## Recovery and compatibility

Each browser searches every five seconds while waiting for a partner. Normal
empty discovery results keep waiting until Stop, so the other person may join
later. Expired advertisements and self entries are ignored. Candidate inspection
uses up to four concurrent Catalog requests, two WSS routes per peer, and at most
16 peers per round; exceeding that limit fails explicitly instead of silently
selecting from an incomplete list.

A subscription makes at most three connection attempts with 1 s / 2 s backoff.
On exhaustion the host refreshes discovery and can select the matching peer's new
publication or relay route. Three failed discovery/connection rounds pause the
host until **Retry discovery**. Transport loss marks remote evidence stale.
An idle subscription probes the Catalog every 15 seconds using bounded protocol
deadlines. Stop cancels inspection/subscription work and retry waits, awaits DDS
lookup cancellation through peer shutdown, and joins the discovery task before
freeing WASM handles. Explicit selection among multiple matches stays pinned to
that Peer ID; restart with a unique session to resume automatic pair selection.

Live reload is disabled in the development server because it would discard
in-memory maps and skip awaited relay cleanup. Stop the peer before manually
reloading after a code update.

Both browsers must run the same demo contract `auki.collaborative-grid/v1`.
Both browsers need this discovery-enabled example build for automatic two-way
connection. Existing Catalog v1, observation-stream v1, and scenegraph snapshot v2 contracts
are reused. There are no API/DDS/DMS contract or deployment changes. The SDK and
experimental adapter are compiled into one WASM module, using the
[Portable Echo pattern](../../../core/examples/portable-echo/web/README.md);
stable core crates acquire no dependency on labs.

One shared-crate fix is required: WASM buffers use `web_time::Instant` instead of
panicking on `std::time::Instant::now`. Native behavior is unchanged. WASM Rust
callers supplying `Buffer::append_shared_at` timestamps must use
`web_time::Instant`. No serialized or backend contract changes result.

## Validation

From the repository root:

```sh
cargo test --locked -p auki-components -p auki-scenegraph -p auki-component-protocol -p auki-collaborative-mapping -p auki-collaborative-mapping-web
cargo check --locked -p auki-collaborative-mapping-web --target wasm32-unknown-unknown
cargo clippy --locked -p auki-components -p auki-collaborative-mapping -p auki-collaborative-mapping-web --all-targets --no-deps -- -D warnings
cargo clippy --locked -p auki-components -p auki-collaborative-mapping -p auki-collaborative-mapping-web --target wasm32-unknown-unknown --lib --no-deps -- -D warnings
cargo fmt --all -- --check
```

From `labs/examples/collaborative-mapping/web`:

```sh
npm ci
npm run build
npm run test:ui
# Set this to a wasm-bindgen-test-runner executable matching Cargo.lock (0.2.121).
WASM_BINDGEN_TEST_RUNNER=/path/to/wasm-bindgen-test-runner npm run test:wasm
```

Tests use local fixtures only; they do not create an offline app mode.
The native integration test uses isolated signed loopback peers and verifies
Catalog/subscription exchange, convergence, resubscription and idle shutdown.
The nine mapping/discovery regression tests also run in actual Chromium WASM, including
session/Domain/peer filtering, bidirectional empty-map discovery, invalid
geometry/identity, stale publication and conflict cases. UI tests load
the real generated WASM module, verify the relay-only setup, grid rendering,
click conversion and mobile layout, without signing in. Injected test discovery
ports cover staggered starts, automatic subscriptions in both directions, expired
candidates, route fallback, multiple matches, cancellation, restart, and bounded
failures. These test fixtures are not an app mode. Chrome must be installed;
UI artifacts are written under ignored `web/test-results/`.

These checks do **not** prove deployed authentication, DDS advertisement/discovery, WSS relay booking/renewal,
or real two-browser relay cleanup. Run the approved live sequence above before
claiming shared-environment compatibility. Python, Swift and Expo bindings are
unchanged and are not included in this example's validation.
