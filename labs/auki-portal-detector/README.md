# Portal Detector

`PortalDetectorComponent` combines QR scanning with an internal asynchronous
size-resolution helper. There is one catalog Component with one `frames` Product
input, not a separately addressable PortalSizer. The generic `auki-qr-detector`
remains independent of Portal maps and database access.

## Input and output

Bind a retained JPEG/RGB8 camera Product with an explicit optical frame and clock,
local `PortalMaps`, a host-selected Domain, a metadata source, and a monotonic
publication clock. `PortalSizeResolver` is the authenticated DDS implementation;
`PortalMetadataSource` permits fixtures. The host supplies its authenticated
client; binding does not create another login or infer a Domain from a QR.

Two Observables belong to the same Component:

- `detections`: unmodified `QrDetections` schema, including empty batches and exact
  camera provenance. `detection_product()` retains the latest 64 batches and can
  be bound directly to the existing `QrLocalizerComponent`.
- `portals`: `auki.portal-detection/v1`, one record per detected QR, with its
  corners/payload, raw detection Product/sequence/index, original camera frame
  reference and capture clock, plus size status. Retain this output explicitly
  if consumers need replay. Empty images are represented by the raw empty batch;
  they do not produce a Portal record.

Size status is one of:

- `LocalMaps`: a size in meters and all matching local map candidates, each with
  explicit frame/placement and exact snapshot reference.
- `Database`: encoded-square side length in meters, canonical Portal ID, selected
  Domain, and database `updated_at` provenance.
- `Pending`: a database lookup is underway.
- `Conflict`: matching local maps disagree on physical size; candidates are
  preserved, with no arbitrarily selected common size.
- `Unresolved`: invalid payload, lookup failure, inaccessible local map, identity
  mismatch, timeout or capacity rejection. No guessed size is supplied.

The raw QR observation is never mutated. An uncached database request produces a
Pending record and then a second enriched record with the same detection identity.
The `portals` Observable uses the **processing clock** because asynchronous results
can complete out of capture order. Consumers use the embedded camera timestamp
and clock for pose math; publication time is not capture time. Raw detections keep
the camera clock. The supplied processing clock must advance for each publication.

## Lookup and lifecycle

Local maps are queried first on every detection and again after a database lookup
completes. A denied or unavailable local map is an error, not proof of absence.
Known payloads need no database parsing. Unknown payloads must match the existing
Portal UUID/shortcode or HTTPS r8.hr convention; arbitrary QR URLs are never fetched.
Sizes exclude the quiet zone and decoration. A database result must match the
requested Portal identity.

Image scanning runs independently from asynchronous metadata requests. Requests
for the same exact payload in this component share an in-flight lookup; alternate
payload spellings are not coalesced. The helper caches at most 128 payloads, with
60-second successful-result and 5-second failure lifetimes. Local maps always take
precedence over cache entries. It bounds work to 8 concurrent lookups, 64 waiting
detections, and a 64-record input queue. Overload produces an explicit unresolved
record; it does not block scanning on the database. Each lookup has a 10-second
timeout. Future detections can retry after failure expiry; there is no retry loop.

`close()` and Drop cancel outstanding requests, stop/join the input and enrichment
workers, end both outputs, and cancel raw retention. A pending record may terminate
without a resolved update when shutdown ends the stream. Input failures/overruns
are available through `input().stats()`, publication/runtime failures through
`last_error()`. The host owns remote export/authorization and retention of enriched
results. Source bindings are fixed; create a replacement component when changing
camera or Domain.

## Downstream use

A ready size or local candidate makes subsequent work **possible**, not mandatory.
The host chooses whether to map or localize. For localization today, bind the
localizer to `detection_product()`, then use the enriched record's raw Product,
sequence, index and selected candidate snapshot in `localize_once`. The localizer
revalidates that exact map revision and uses its recorded portal size. Conflicting
local sizes therefore require explicit map selection, not a common guessed size.

For a new portal, the record supplies size for calibrated geometry; the host still
computes a placement and requests an authorized map edit. This component does not
run PnP, establish map frames, calculate placements, or modify maps. Existing
`PortalMapper::lookup_helper` remains available; it has not been removed or changed
to consume the new record type. The size helper currently reuses its metadata
interfaces from `auki-qr-mapper` rather than duplicating the DDS contract.

This adapter is native-only, like the current QR Component integration; no browser
or hardware support is claimed. No stable core dependency or backend wire contract
changes. Existing raw QR consumers continue to work; enriched output is a new schema.

```sh
cargo test --locked -p auki-portal-detector -p auki-qr-mapper
cargo clippy --locked -p auki-portal-detector -p auki-qr-mapper --all-targets --no-deps -- -D warnings
```

Tests render actual QR images and use fixture metadata, separate capture and
publication clocks, delayed requests, map changes, conflicting sizes, queue limits,
identity mismatches, arbitrary URLs, and cancellation. No live backend is contacted.
