# Collaborative mapping in browsers

A relay-only web experiment with one pannable map and toggleable peer layers.
Each browser owns its original map. Shared simulated Portal identities align
maps, including through another peer, without republishing a derived union.
All SDK and mapping code runs in browser WASM; there is no application server,
Rust host process, or offline app mode.

## Run and use

Requires Rust 1.89+, `wasm32-unknown-unknown`, wasm-pack 0.13.1, Node 20.19+
(on 20.x) or 22.12+, and current Chromium. From the repository root:

```sh
cd labs/examples/collaborative-mapping/web
npm ci
npm run dev
```

Starting Vite does not make backend services local. Before signing in against
shared services, approve the environment, User account, Domain, relay bookings,
DDS discovery advertisement/lookups, P2P map publication/reads, and cleanup using
[the first-peer tutorial](../../../docs/tutorials/first-peer.md). This example
never writes Domain Server data, provisions nodes, or submits DMS tasks.

1. Select shared development services or matching custom API/DDS/DMS endpoints.
   Sign in, choose a Domain, session name, and **My coordinate convention**, then
   **Start / join session**. Each peer chooses independently: +X right / up / left /
   down, with perpendicular +Y and +Z out of the map. These are screen-plane
   directions, not different gravity axes. The choice is fixed for that publication;
   leave and restart to change it without relabelling an existing map.
   You can map alone immediately once your relay peer is ready.
2. Other browsers use the same Domain and session. Each browser continuously
   discovers and subscribes to matching peers, up to 16 session participants
   including itself. There is no host browser or partner-selection step.
3. Drag the map to pan; scroll or use +/− to zoom. **Fit** frames visible portals.
   Double-click to drop a temporary pin, enter a name, and Save. Escape cancels
   without publishing. **Drop portal** and Enter on the focused map drop at its
   center; arrow keys pan. Touch users can pan and use the visible controls.
4. In A, place `bridge` at local `(0, 0)`; in B, place `bridge` at local `(10, 20)`.
   Their maps align. **My map · my coordinates** stays selected by default:
   your axes and origin stay fixed, and aligned remote layers are transformed
   into your coordinate system. Peers joining or leaving do not switch your view.
   The optional **Shared canonical frame** uses the lexicographically smallest
   reachable peer's frame for comparing identical numeric coordinates.
5. Toggle peer layers to compare placements. The coordinate selector also shows
   each map's original frame. Unaligned layers appear as faded, labelled previews:
   their raw source X/Y values are plotted against the current axes without claiming
   an alignment. They may look completely misplaced. Previews retain source-frame
   metadata, stay separate even when names match, and are read only; they never
   enter the aligned union or alignment evidence. Layer toggles also hide/show
   previews. Once alignment is established, full-opacity transformed pins replace
   them. **Unaligned · view** opens the original coordinate space.
   Placement is disabled in an unaligned remote frame. Incoming updates preserve
   pan/zoom; a changed coordinate frame fits the view again.
6. Hold **⌘ Command** and drag your own pin to move it. Release to publish the
   snapped grid position; Escape cancels. Ordinary dragging still pans, and
   another peer's pin cannot be moved.
   Click a portal to inspect it. Your own portal can be moved by editing its
   coordinates or deleted with **Remove mine**. Other peers' portals are read only.
   Reusing a name when dropping a new portal moves your existing placement.
7. Conflicting shared placements appear red. **Resolve alignment** names the
   contradictory Portal pairs and peers. Neither side is automatically judged
   correct. Move/remove your own evidence to restore alignment. Contradictory
   connected components fail closed; unrelated unaligned maps remain inspectable.
8. Peers can leave and join without stopping the others. **Leave session & reset
   map** cancels all receive tasks, closes the endpoint, and awaits peer shutdown,
   removing DDS registration and releasing relay bookings. Sign out also closes
   the User session. Restart creates a fresh peer and empty publication.

Maps are memory-only. Leave before reloading or closing. Abrupt browser closure
cannot promise awaited cleanup. Development HMR is disabled to preserve active
maps; manually reload after leaving to load new code.

## Components and spatial contract

- `DemoMap` uses `ComponentRuntime`, the existing
  [`MapComponent`](../../auki-scenegraph/README.md), and
  `MapAlignmentChecker`. Local edits call authorized `upsert_qr` / `remove_qr`
  with exact snapshot references.
- The WASM adapter mounts
  [`ComponentProtocolEndpoint`](../../auki-component-protocol/README.md),
  exports the snapshot Product only, validates Catalog identity and schema, and
  subscribes independently to each exact peer Product using `LatestExisting`.
  No remote editing Operables are exported.
- Source layers retain explicit independent frame IDs, right-handed XY, Z-up,
  meters. Each preset declares an exact map definition and Portal orientation.
  Non-default presets have distinct frame IDs and a declared screen-axis
  convention in the origin description; discovery validates the complete known
  definition rather than guessing a convention from a peer or Domain. Every alignment carries source/destination frame IDs. The canvas
  applies validated transforms only; missing alignment never means equal origins.
- Convention presets build explicit `auki-registry::FrameRegistryEntry` values.
  `auki-geometry` handles point convention conversion, Portal orientation, and
  alignment application/inversion. The demo adapter checks transform endpoints
  and rejects projections outside its 2D plane. Browser conversion calls these
  helpers through WASM; TypeScript keeps only display labels and canvas projection.
  Convention descriptors are mathematical declarations, never evidence of shared
  physical origins. The scenegraph checker still requires equal up-axis and units;
  arbitrary 3D or mixed-unit maps are outside this demo's contract.
- Session+name deterministically derives synthetic Portal UUIDv8 identity using
  SHA-256. Names are exact and case-sensitive, 1–64 ASCII letters/digits/spaces/
  underscores/hyphens, without leading/trailing spaces. These are not DDS records.
- Portals have fixed printed-right = screen-right heading and 0.25 m sides.
  Their pose quaternion explicitly converts that Portal frame into the chosen
  local axes. A shared Portal determines a rigid alignment with the appropriate
  quarter-turn rotation and translation. Merely choosing a convention never
  establishes shared origins. This demo does not claim
  arbitrary real-world names establish physical alignment. Integer source
  coordinates are bounded to ±10,000 m, with at most 128 Portals per peer.
- Conflict diagnostics compare shared-Portal offsets pairwise across every map
  pair after explicit conversion to the declared screen-plane basis. Different
  integer offsets exceed the checker's 2 cm tolerance. Diagnostics
  do not choose a supposedly correct outlier or relax frame/geometry validation.
- The canonical aligned subset includes the local map. Maps outside that subset
  remain separate layers. Each view exposes originals, optional transformed
  placements, exact snapshot sequences, and explicit transform endpoints.
- Derived layers/unions are never republished as evidence. Publication timestamps
  use a named, monotonic per-publication logical clock; these are synthetic,
  static placements rather than time-varying physical observations.

Domain discovery and session metadata determine relevance, not read permission.
A session name is not a secret. Published synthetic snapshots are readable by
P2P peers admitted to the selected Domain. Do not publish sensitive maps here.
Transport admission does not grant map edits or Domain Server writes.

## Connection recovery

Discovery continues every five seconds even while streams are connected. One
unreachable peer cannot suspend another peer's updates. Each round inspects at
most 16 candidate peers, four concurrently, trying up to two WSS relay routes.
A larger candidate set reports a discovery error rather than silently taking
an incomplete sample. Active streams remain alive during discovery outages.

A receive task makes three bounded connection attempts with 1 s / 2 s backoff.
On transport failure its map evidence is withdrawn, so stale geometry cannot
remain a live combined layer. Discovery retries failed peers with exponential
backoff capped at 40 seconds, and retries DDS failures capped at 80 seconds,
for the lifetime of the user-started session. Stop cancels all timers and tasks.
Fresh publications/routes are inspected again and replace old subscriptions
only after their receive tasks have been canceled and joined.

Idle streams probe the Catalog every 15 seconds using protocol deadlines.
The probe checks both exact Product identity and published `source_sequence`.
If Catalog metadata proves the observation stream has fallen behind, the adapter
reopens it with `LatestExisting`. A successful Catalog request alone no longer
counts as proof of fresh map data. Layer rows show per-peer status and received
snapshot sequence. Disconnection detection is bounded by the idle interval plus
protocol deadlines; abrupt disconnects are not reported instantly.

A missing DDS advertisement alone does not invalidate a healthy authenticated
stream. Stream/catalog failure removes its evidence; a later matching peer may
join without resetting the local map. Retry state for absent, inactive peers is
pruned. Shutdown closes the mapper first, shuts down the SDK peer to cancel DDS
work, then awaits discovery before freeing WASM handles.

## Compatibility

Existing Catalog v1, observation-stream v1, and scenegraph snapshot v2 contracts
are unchanged. No API/DDS/DMS deployment or auth-profile changes are required.
All browsers should run this coordinate-convention build when testing mixed
presets. Default +X-right maps retain their existing map/Portal contract. Older
builds cannot discover or consume the new rotated map definitions; they must be
upgraded together before using non-default presets. No shared backend contract
changes are required.
The example's WASM view JSON now contains `layers`, `display_frame`, and pairwise
`conflicts`; each layer now declares `convention`, and transforms include
rotation as well as translation. `mount(peer, session, convention)` requires a
validated preset string. The additive `convertConventionPoint` and `transformPoint` exports use SDK geometry;
`transformPoint` requires explicit matching frame endpoints. UI and WASM must be rebuilt together. `follow` allows independent
subscriptions and `unfollow(peer)` cancels/joins one. These are demo binding
changes, not stable SDK binding changes.

The adapter follows the
[Portable Echo pattern](../../../core/examples/portable-echo/web/README.md);
stable core crates acquire no dependency on labs. The earlier WASM buffer fix
uses `web_time::Instant` for WASM only; native behavior is unchanged.

## Validation

From the repository root:

```sh
cargo test --locked -p auki-collaborative-mapping
cargo check --locked -p auki-collaborative-mapping-web --target wasm32-unknown-unknown
cargo clippy --locked -p auki-collaborative-mapping -p auki-collaborative-mapping-web --all-targets --no-deps -- -D warnings
cargo clippy --locked -p auki-collaborative-mapping -p auki-collaborative-mapping-web --target wasm32-unknown-unknown --lib --no-deps -- -D warnings
cargo fmt --all -- --check
```

From `labs/examples/collaborative-mapping/web`:

```sh
npm ci
npm run build
npm run test:ui
WASM_BINDGEN_TEST_RUNNER=/path/to/wasm-bindgen-test-runner npm run test:wasm
```

Use wasm-bindgen-test-runner 0.2.121 matching Cargo.lock. Seventeen mapping tests
run natively and in Chromium WASM. Isolated authenticated loopback tests cover
two- and three-peer streams (including mixed axis presets), edits, conflict
removal, resubscription and departure. Preset tests cover all 16 pairings, inverse
placement transforms, rotated conflicts and recovery, and rejected unknown map
definitions or mismatched Portal orientations. Browser tests cover rotated
axis labels, double-click/drop coordinates, ⌘-drag, and remote-frame rendering.
Browser controller tests cover solo startup, full-mesh joins, discovery during
active subscriptions, failure recovery and cancellation. UI tests first load
real generated WASM, then use a test-only intercepted WASM port fixture to drive
session controls, layer/frame selection, pan/zoom, named drops, cancellation,
inspection/removal, viewport preservation, mobile layout and Stop. The fixture
is never imported by production and creates no offline application mode.
Screenshots are saved under ignored `web/test-results/`.

These checks do not prove deployed DDS discovery, WSS relay liveness/recovery,
authentication or shared-service cleanup. Live multi-browser relay validation is
still required in an approved environment. Python, Swift and Expo are unchanged.
