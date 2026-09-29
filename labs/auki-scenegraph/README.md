# QR scenegraph maps

Experimental first slice of a Map Component: one root frame, directly parented
QR anchors, complete snapshots, and an authorized `upsert_qr` / `lookup_qr` / `find_qr` API.
A Mapper is a separate producer of proposed updates; no detector, pose estimator,
or localization service is required to create this map.

`auki-scenegraph` keeps scene data and USD export independent of the Component
runtime. Disable the default `components` feature for data-only use. The optional
`component` module depends on `auki-components`, never on networking. A host mounts
and explicitly exports its interfaces using `auki-component-protocol`.

## Data contract

- `MapDefinition` has a stable map ID, optional name and Domain reference, and an
  explicit root-frame definition. A Domain describes the place; multiple maps
  may refer to it. Membership does not establish alignment or confer authority.
- Default frames are right-handed, Z-up, meters. V1 supports right-handed Y-up
  and other positive linear scales too. A different handedness requires explicit
  conversion before USD export. The caller supplies the origin/heading definition;
  seeing a QR does not silently align it with gravity or establish world heading.
- `QrAnchor` has a physical-marker ID independent of decoded payload. Its centered
  local frame is X printed-right, Y printed-up, Z out of the front. The positive
  side length is in meters, across the encoded square excluding the quiet zone.
- Placement is an active rigid transform into the map root. Translation is in
  map units, rotation is a unit quaternion in w,x,y,z order. All anchors are
  direct children of the root in v1, so lookup needs no ancestor traversal.
- `MapSnapshot` contains the typed scenegraph and a deterministic, generated USDA
  representation of exactly that scene. Typed scene state is authoritative;
  arbitrary USD editing/import/composition is not implemented. After decoding
  untrusted input call `validate()` to reject inconsistent representations.
- Snapshot identity is Product reference plus original observation sequence.
  There is no independent map revision counter. Observation timestamps are
  publication times on the host-supplied clock, not anchor capture times.
- A physical marker ID is unique within a map. Identical QR payloads may appear
  on different physical markers. `lookup_qr` uses anchor ID; `find_qr` searches exact decoded payload.

Bounds: at most 1,024 anchors, a 4 MiB encoded snapshot, 256-byte identities and
4,096-byte QR payloads. Text must be nonempty and contain no control characters.
The capture retains the latest complete snapshot only; slow subscribers may get
a sequence gap and recover using the next complete snapshot. No file assets or
external USD references are emitted. QR nodes are transforms and attributes, not
rendered/textured squares or a registered OpenUSD plugin schema.

## Construct and advertise

```rust,ignore
let map = MapComponent::new(
    &runtime,
    MapComponentConfig {
        component_id: "map".into(),
        publication_id: fresh_publication_id, // new ID on every publisher run
        clock: host_clock_reference,
        map: definition,
    },
    read_host_clock_ns,
    authorize_lookup,
    authorize_update,
)?;
let endpoint = ComponentProtocolEndpoint::mount(peer.protocols(), runtime)?;
endpoint.export_product(&map.product())?;
endpoint.export_operable(map.lookup_qr())?;
endpoint.export_operable(map.find_qr())?;
endpoint.export_operable(map.upsert_qr())?;
```

A fresh publication starts with an empty snapshot. Its Catalog Product carries
`auki.scenegraph.qr-snapshot/v2`; the producer contract's `observes` is the map ID
and `spatial_frame_id` names its root frame. Consumers discover the endpoint via
ordinary peer discovery, select the exact exported Product, and subscribe using
`ObservationStart::LatestExisting`. The host owns discovery registration, routes,
trust, and the choice of map. The test uses an explicit loopback route; it does
not prove DDS discovery or relay behavior.

All subsequent snapshots keep the same map definition. Starting a publisher for
an incompatible frame/schema requires a fresh publication identity and explicit
consumer selection. The constructor requires a fresh publication ID even after
restart; this prevents sequence zero from reusing an old snapshot identity.
Durable map restoration and publisher failover are future work.

## Edits, queries and authority

`upsert_qr` replaces one complete anchor record or inserts it. It requires the
exact `expected_snapshot` reference obtained from a snapshot or lookup. Stale or
foreign references are rejected rather than silently overwriting another mapper's
work. The caller can fetch current state, reconsider, and retry. This is optimistic
concurrency, not observation fusion. Retrying a successful edit with its old base
returns a conflict; invocation IDs do not provide exactly-once mutation semantics.

The host's publication clock must advance for each changed snapshot. Invalid
geometry, clock regression, bounds violations and conflicting bases leave state
unchanged. A retention failure closes publication without acknowledging the
unretained change. Edits and lookups share a lock, so queries report an internally
consistent snapshot and the corresponding Product source sequence.

`lookup_qr` returns the map definition, exact snapshot reference and optional
anchor. `anchor: null` explicitly means not found in that snapshot. V1 answers
latest-state queries only; historical lookup is not promised by latest-only
retention. Consumers may keep earlier snapshots themselves.

All operations require explicit authorization callbacks. Over P2P the caller
Peer ID comes from the authenticated stream. Caller Component ID is an asserted
label, not independently authenticated identity. Read permission never implies
write permission, and Domain association never authorizes writes. Exporting the
snapshot Product makes its contents readable to peers admitted by the protocol
endpoint; the lookup callback is **not** a Product-level read ACL. Do not export
sensitive maps to a broader audience than intended.

Close/unexport the network endpoint first, then close the Map Component and
shut down the peer. Closing/dropping the Map Component rejects new operations and
closes retained readers; the last snapshot remains readable. It does not invent
a producer failure. Hosts must still own and await network cleanup.

## Verification

All tests use synthetic geometry and isolated signed loopback credentials; no
camera, shared service, discovery registration, or relay booking is used.

```sh
cargo test --locked -p auki-scenegraph
cargo clippy --locked -p auki-scenegraph --all-targets --no-deps -- -D warnings
cargo check --locked -p auki-scenegraph --no-default-features
cargo check --locked -p auki-scenegraph --target wasm32-unknown-unknown
cargo run --locked -p auki-scenegraph --example qr_map_usda > /tmp/qr-map.usda
```

The two-peer test discovers the exported Product through the Catalog, subscribes,
queries the same snapshot, rejects an unauthorized edit, moves an anchor, observes
the new snapshot and query, and checks unknown-anchor behavior before awaited
cleanup. Local tests cover invalid geometry, stale writes, publication-clock
regression, USD serialization and shutdown.

Optional OpenUSD interoperability check, in a disposable Python environment:

```sh
python -m pip install usd-core==26.8
python labs/auki-scenegraph/tests/validate_usd.py /tmp/qr-map.usda
```

This verifies the stage using OpenUSD itself, including quaternion/translation
composition and the Z-up/meter metadata. OpenUSD is not a Rust runtime dependency.
WASM compilation is not browser acceptance. The separate [QR localizer](../auki-qr-localizer/README.md) consumes these
individual anchor queries to estimate camera poses. No automatic anchor pose
estimation, arbitrary scene hierarchy, semantic detector, voxel fusion, persistence,
historical query or general USD editor is implemented by this slice.

For Auki QRs, the Mapper should resolve physical size through the
[Portal size resolver](../auki-qr-mapper/README.md) before estimating placement.
The Map stores the resulting encoded-square side length in meters.

`find_qr` returns only matching anchors with their map definition and snapshot
reference. Multiple matches in one map are explicit ambiguity. `PortalMaps`
queries registered peer-owned Map Components through this authorized operation;
it is the default resolver for localization and Mapper admission. Snapshot
Products remain useful for publication and replication but are not required as
localizer inputs. Each lookup is atomic within one map; queries across several
maps do not constitute a shared transaction or spatial alignment.

## Catalog Portal membership

A snapshot Product's Catalog entry includes `metadata` with schema
`auki.scenegraph.qr-catalog/v1`, `source_sequence`, and this `value`:

```json
{
  "map": { "map_id": "store-map", "name": "Store map", "domain_reference": "store", "frame": { "handedness": "right", "id": "store-frame", "up_axis": "Z", "meters_per_unit": 1.0, "origin_description": "Chosen origin" } },
  "portals": [
    { "anchor_id": "00000000-0000-0000-0000-000000000001", "payload": "HTTPS://R8.HR/ABC12345678" }
  ]
}
```

`portals` lists **every** QR anchor, ordered by anchor ID; it is never sampled,
truncated or probabilistic. An empty map advertises an explicit empty array.
Auki's Mapper uses the canonical Portal UUID as the anchor ID; generic or
manually named QR anchors retain their own IDs and are included too. The exact
payload lets peers compare a detected QR without first resolving its UUID online.
`MapCatalogData::contains_payload` provides that relevance check. Geometry stays
in the map rather than being duplicated in the Catalog.

The containing Product reference plus `source_sequence` identifies the map
snapshot described. Every successful changed snapshot refreshes metadata and
invalidates Catalog revision caches, including changes that leave Portal
membership unchanged. No-op and rejected edits leave the advertisement unchanged.
Changes never alter the Product's immutable identity. A later map update may race
with a fetch, so consumers must compare source sequences if exact agreement is
needed; the Catalog does not promise historical retention.

The complete metadata is bounded to 256 KiB; all 1,024 supported Portal UUIDs
and standard QR payloads fit. Oversized metadata (e.g. unusually long generic QR
payloads) rejects a map edit before publication, rather than silently omitting
members. The network Catalog's aggregate 1 MiB control-frame bound still applies;
very large collections of exported maps need future Catalog pagination. The
protocol fails oversized responses instead of returning a partial membership list.

Metadata follows the Product's export visibility. Exporting the map makes this
membership list visible to admitted Catalog readers; the `lookup_qr` authorizer
does not independently restrict it. Old Catalog entries without metadata mean
membership is unknown, not that the map has zero Portals.

## Default and named maps for a Domain

A `MapDirectoryComponent` manages one Domain's map selection. Its
`DomainMapDirectory` contains registered map definitions and exact Product
references, plus **one `default_map_id` pointer**. Defaults are not independently
asserted by each Map Component. The directory has no spatial frame and does not
change any map's geometry, origin or Portal membership.

`resolve_map` accepts a Domain and an optional selector. Omitting the selector
means default; a name or ID selects a particular registered map:

```json
{ "domain_reference": "store-domain" }
```

```json
{ "domain_reference": "store-domain", "selector": { "kind": "name", "value": "Stockroom" } }
```

The result identifies the selected map, its exact Product reference, and the
directory snapshot used for selection. Consumers can then fetch that Product
or query its Map Component. `DomainMapDirectory::resolve` applies the same rules
locally to a validated directory snapshot or its Catalog metadata.

Names come from `MapDefinition.name`, are exact and case-sensitive, and must be
unique within this Domain directory. The same name can exist in other Domains.
Unnamed maps remain selectable by ID or as the default. No default is selected
implicitly, even when only one map is registered. A missing default, unknown
name/ID, or wrong Domain is an error; none falls back to a different map.

Host setup follows the existing Component pattern:

```rust,ignore
let directory = MapDirectoryComponent::new(
    &runtime,
    MapDirectoryConfig {
        component_id: "store-map-directory".into(),
        publication_id: fresh_directory_publication_id,
        clock: host_clock_reference,
        domain_reference: selected_domain,
    },
    read_host_clock_ns,
    authorize_directory_read,
    authorize_directory_write,
)?;
InMemoryTransport.invoke(directory.update_directory(), admin_context.clone(), UpdateMapDirectory {
    expected_snapshot: directory.snapshot_reference(),
    change: DirectoryChange::Register {
        entry: DirectoryMap::from_product(&local_map.product())?,
    },
})?;
InMemoryTransport.invoke(directory.update_directory(), admin_context, UpdateMapDirectory {
    expected_snapshot: directory.snapshot_reference(),
    change: DirectoryChange::SetDefault { map_id: Some(chosen_map_id) },
})?;
endpoint.export_product(&directory.product())?;
endpoint.export_operable(directory.resolve_map())?;
endpoint.export_operable(directory.update_directory())?;
```

The directory Product advertises complete Catalog metadata under
`auki.scenegraph.domain-map-directory/v1`, including the Domain, default pointer,
map names and Product references. Map Products keep their own complete Portal
lists. Changing the default updates only the directory snapshot/Catalog revision;
it does not alter map snapshots or Product identities. Removing the default map
from the directory clears the pointer and never implicitly promotes another map.
`SetDefault { map_id: None }` also clears it. Re-registering the same stable map ID
can replace its publication/name, subject to uniqueness and Domain validation.

Directory reads and edits have separate authorizers. Edits require the exact
current directory snapshot reference, so concurrent or stale changes conflict.
Names, Domain association and directory membership do not grant map access.
Registration is an authorized assertion; resolution does not establish that a
referenced Product is reachable. Publisher restart/failover requires explicit
re-registration of the new Product reference, not silent fallback.

The application must select the directory publisher it trusts for this Domain.
Multiple publishers can advertise different directories; the SDK does not infer
global authority from peer identity or Domain membership. This implementation
adds no DDS backend fields and does not persist a server-wide default. Hosts own
persistence, authority selection and restart restoration. Directory bounds are
64 maps and 256 KiB of Catalog metadata; oversized edits fail atomically. Export
visibility and aggregate Catalog frame limits are the same as for Map Products.

Tests cover default/named/ID lookup, Catalog invalidation, name collisions, wrong
Domains, authorization, concurrent updates, removal, clock/retention failures,
and an authenticated two-peer selection and default-switch exchange.

## Explicit transform endpoints

Every `RigidTransform` requires `from_frame_id` and `to_frame_id`; there is no
unlabelled default or deserialization fallback. An anchor's source identifies
its centered QR frame (+X printed-right, +Y printed-up, +Z outward). Its
destination must equal `MapDefinition.frame.id`. Translation is in map units;
QR geometry is converted from meters to those units for the USD stage.
`RigidTransform::identity(from, to)` is an explicit declaration of coincident
origins and axes, not an inferred alignment. USD preserves both endpoint IDs.

The map snapshot, upsert request and anchor-bearing lookup responses now use
v2 contracts. Old unlabelled snapshots are rejected; migration requires the
producer to supply the actual frame identities. No Domain or anchor ID is used
as an implicit frame identity. The original retained-snapshot reference and
catalog metadata contracts remain unchanged.

## Two-peer Portal exchange acceptance test

`tests/portal_exchange.rs` starts two authenticated loopback peers, publishes
A–B in A's frame and B–C in B's frame, discovers Portal membership through the
remote catalog, and fetches each original partial map over P2P. Host-side test
orchestration aligns through B using `auki-geometry`, imports the missing anchor
through an authorized local edit, then fetches both updated maps remotely.
Independent analytic fixtures verify all three positions, orientations and
explicit frame endpoints in each map; original anchors remain unchanged.

```sh
cargo test --locked -p auki-scenegraph --test portal_exchange -- --nocapture
```

This exercises map exchange and exact rigid alignment with equal, explicitly
specified conventions and meter units. It does not implement an automatic map
merger, infer cross-convention conversions, test noisy PnP observations, or
resolve a moved shared Portal. The import orchestration is test-local. Local
TCP access is required; credentials are isolated fixtures and no shared backend
is contacted.

## Optional alignment discovery

`alignment::MapAlignmentChecker` is a bounded host-owned index of up to 64 known
map publications. Feed it authorized incoming Catalog metadata or validated map
snapshots; it never fetches remote data or mutates a map itself.

```rust,ignore
let mut checker = MapAlignmentChecker::new(AlignmentOptions {
    automatic: true, // default is false: explicit checks only
    ..Default::default()
})?;
let events = checker.receive_catalog(map_key, snapshot_reference, catalog_data)?;
// Forward events to the application. A potential path identifies Portal IDs
// and exact source/target snapshots whose anchors are needed.
let events = checker.receive_snapshot(map_key, snapshot_reference, snapshot)?;
let result = checker.check("purchased-map", "local-map");
```

`receive_*` and `remove` return changed-pair `AlignmentEvent`s in automatic mode.
In manual mode call `check(source, target)` or `check_all()`; the latter returns
changed results only. The host connects these ingestion calls to its Catalog
and Product subscriptions, and forwards events through its own UI or Observable.
There is no implicit network subscription or background task.

Results distinguish `NoConnection`, `PotentialConnection`, `Available`, and
`Conflict`. Catalog overlap establishes only potential connectivity. Paths
include map keys, supporting canonical Portal UUIDs, and exact snapshot
references. Validated snapshots permit geometric alignment; available results
carry explicit source/destination frame IDs. Adding a bridge map discovers
transitive paths regardless of arrival order. Domain membership creates no edge.
Generic map-local anchor IDs are not treated as global Portal identities.

The initial solver supports maps with equal, explicitly declared conventions
and units. Differing conventions/units remain potential connections requiring
explicit conversion with `MapConventionConversion` before alignment. A shared Portal uses the centered QR convention declared
by `QrAnchor`; its physical sizes must agree. Multiple shared Portals and
alternative paths are checked for consistency. Default residual tolerances are
2 cm and 0.02 radians, configurable at construction. Conflicting evidence fails
closed for the connected candidate component. A single shared Portal can supply
an alignment but cannot prove that it has not moved; the supporting path makes
that limitation visible to the application. These are estimates, not merge approval.

A newer catalog sequence discards cached geometry until its snapshot arrives.
Older sequences, inconsistent data for the same sequence, invalid metadata and
unexpected publication replacement are rejected without replacing accepted
state. Remove an old publication explicitly before registering its replacement.
Removal/update emits changed results, including withdrawn alignments. The caller
must use the evidence references to revalidate a merge plan before committing it.

`tests/alignment.rs` covers transitive discovery, catalog-to-snapshot promotion,
manual/automatic modes, invalidation, stale data, rotated alignment, conflicts,
and disconnected maps. `tests/portal_exchange.rs` also applies the checker to
snapshots exchanged by real authenticated loopback peers.

The same Portal exchange suite also tests a single publisher with two maps:
Peer2 advertises A–B and B–C as separate Products. Peer1 discovers both through
the catalog, initially reports potential alignment, downloads both snapshots,
and obtains an available alignment through B from `MapAlignmentChecker`.
Host-side test orchestration creates the distinct `merged-ABC` map, copies A–B,
transforms C into A's frame, and verifies shared B before deduplicating it.
Peer2 then reads Peer1's catalog and fetches the complete A–B–C Product. The test
checks all poses and that the source publications and catalog remain unchanged.

## Explicit convention conversion

`MapConventionConversion` converts a `Scenegraph` or complete `MapSnapshot` into a
new map artifact, without mutating or publishing the source. It supports every
convention currently representable by `MapFrame`: right-handed Y-up/Z-up and
positive finite meters-per-unit scales. It is available without `components`.

Use `from_registry_frames(source_frame, target_frame, source_registry,
target_registry)` when complete axis conventions are available. Frame IDs,
handedness, up axes and units must match the map declarations. The rotation is
derived using `auki-geometry`, without assuming horizontal heading from up-axis
metadata. The registry constructor supports the registry's meter/cm/mm units.
For other positive scales, `new(source_frame, target_frame, axis_rotation)` takes
an explicit zero-translation, labelled unit-quaternion rotation. Both APIs check
that source up maps to target up. New frame and map IDs are required.

```rust,ignore
let conversion = MapConventionConversion::from_registry_frames(
    source.scenegraph.map.frame.clone(),
    target_frame,
    &source_registry,
    &target_registry,
)?;
let converted = conversion.convert_snapshot(&source, "converted-map")?;
let point_transform = conversion.coordinate_transform(); // labelled 4x4, includes scale
```

Portal positions and orientations are transformed on the map side; Portal-local
printed-right/up/out frames, identities, payloads and physical `side_length_m`
remain unchanged. Translation and exported mesh coordinates use the destination
units. Canonical USDA is regenerated with the new up axis, scale and labels;
inconsistent source snapshot JSON/USDA is rejected. Conversion is atomic and
rejects invalid/nonfinite rotations, mismatched declarations and numeric overflow.
Names and Domain associations are preserved; the host can assign a distinct name
before publication. Publication clock, Product identity and revision are supplied
by the host, not fabricated by this pure conversion operation.

Conversion preserves the physical origin. It does **not** align independent maps:
normalize a map into the desired convention using a fresh frame ID, then use shared
Portals with `MapAlignmentChecker` to establish the remaining alignment, and
explicitly merge/admit the anchors. Never reuse another map's frame ID merely
because the conventions match. `tests/conversion.rs` demonstrates a Y-up/cm B–C map
converted to Z-up/meters, aligned against A–B, and merged into A–B–C.

The coordinate conversion itself may include scale, so `coordinate_transform()`
returns a labelled matrix rather than incorrectly representing it as a rigid
quaternion pose. Left-handed scenegraphs are not supported by the current Portal
pose representation; registry declarations requiring reflections are rejected.
This API converts the current root-and-Portal scenegraph, not arbitrary USD files,
nested meshes or voxel grids. No snapshot wire-schema change is needed.

Runnable offline example (40cm square, center becomes `[100, 300, -200]` cm):

```sh
cargo run --locked -p auki-scenegraph --no-default-features --example convert_qr_map > /tmp/converted-portal.usda
```
