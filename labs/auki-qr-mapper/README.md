# QR Mapper building blocks

`PortalMapper::lookup_helper` implements local-map-first QR admission:

1. Query all registered local maps. A known QR returns `Known` without HTTP.
2. For an unknown QR, resolve metadata using the existing authenticated
   `AukiDomains::portal` API and validate the physical size.
3. Without a host-supplied placement, return `NeedsPlacement`; never invent one.
4. With an explicit placement into a registered local map, submit an authorized
   compare-and-set edit. Return `Added` only after the Map accepts it.

The default `PortalSizeResolver` performs the database lookup; the
`PortalMetadataSource` interface also permits isolated fixture testing.

```rust,ignore
let mapper = PortalMapper::new(local_maps.clone(), PortalSizeResolver::new(domains));
let result = mapper.lookup_helper(
    decoded_payload, selected_domain,
    Some(AnchorPlacement {
        map: local_map.clone(),
        expected_snapshot: local_map.snapshot_reference(),
        pose_in_map: approved_qr_placement,
    }),
    &cancellation,
).await?;
```

Use an explicitly chosen first-anchor placement to establish a new map frame.
For an existing map, derive the placement from a known camera pose and calibrated
QR geometry. This helper does not estimate that placement, create a map, run a
background detector subscription, or align independent map frames. The host
routes unknown detections to it and manages asynchronous work/cancellation.

```rust,ignore
let resolver = PortalSizeResolver::new(domains);
let portal = PortalId::parse(portal_identifier)?;
let resolved = resolver.resolve(selected_domain, &portal, &cancellation).await?;
let side_length_m = resolved.side_length_m();
// Use this size for QR pose estimation and QrAnchor.side_length_m.
// resolved.metadata() preserves the canonical Portal ID and updated_at.
```

The host selects the Domain and authenticated client. The Mapper accepts a raw
Portal UUID/short ID or the Console's `HTTPS://R8.HR/<id>` payload convention.
It never fetches the QR URL. Other URL hosts, queries and extra paths are rejected
for unknown QRs. A known exact payload can still resolve locally without parsing.
No Domain inference or second login loop is introduced.
The existing DDS client verifies that returned metadata matches the requested
Domain and Portal and propagates authorization, lookup and cancellation errors.

`Portal.size` is centimeters; `side_length_m = size / 100`. The measurement covers
the encoded QR square, not the quiet zone or AQR decoration. This is supported by
both the SDK's documented `Portal` field and the Console printing path:
`src/routes/OrganizationPortalsTab.tsx` converts centimeters to millimeters;
`src/util/qrCodes.ts::drawQrCodeToPdf` divides that width by the encoded matrix's
module count and draws the surrounding decoration outside it. The DDS provider
model is `aukilabs/domain-service/pkg/models/lighthouse.go`; its wire name remains
`lighthouses`. No backend contract changes are required.

Invalid, missing or inaccessible size must leave a new QR unplaced rather than
use a guessed default. This resolver has no cache or automatic retry loop beyond
the existing client's credential refresh behavior. A future Mapper can retain
resolved metadata and define its own explicit refresh policy. A changed size
requires reconsidering the estimated placement, not merely changing the number
on an existing anchor. Consumers localize using the size recorded in their map
snapshot, without a database lookup per camera frame.

Lookup failures, invalid sizes, denied writes and stale placements leave the map
unchanged. The helper rechecks local maps after an awaited metadata request to
avoid needlessly admitting a QR another Mapper has already added. A concurrent
edit still causes a compare-and-set conflict; the caller must reconsider its
placement before retrying. Existing canonical Portal IDs with different payloads
are not silently overwritten. Localizer lookup uses exact stored payloads.

```sh
cargo test --locked -p auki-qr-mapper
cargo clippy --locked -p auki-qr-mapper --all-targets --no-deps -- -D warnings
cargo check --locked -p auki-qr-mapper --target wasm32-unknown-unknown
```

Unit tests use local Portal fixtures; they do not access a live database. The
underlying client's HTTP/authentication fixture tests live in
`core/auki-domain-client/tests` and `core/auki-auth/src/tests`.

Placements must explicitly provide both `from_frame_id` (the observed Portal's
local frame) and `to_frame_id` (the target map frame). The mapper preserves these
labels; the map rejects a destination mismatch. Frame IDs are never generated
from a shortcode or inferred from Domain membership.
