# Data products and Resource Catalogs

A data product is one addressable thing a peer can describe or serve: a Sensor
Log, Pose Log, TimeTransform Log, Detection Log, Map Log, or live message
channel.

The current portable APIs are:

- Catalog v3 for log rows and message channels;
- Catalog v4 for Map Logs;
- Registry v3 for exact metadata entries and Device Model listing;
- Blob v1 for immutable binary content; and
- Stream v2 for typed live/replay subscriptions.

Catalog v2 remains a wire-only codec because v3 embeds its locked log-row
shape. `CatalogEndpoint` does not serve v2. Registry support begins at v3 and
has no older fallback.

## Catalogs are snapshots

A catalog response is sampled when the request arrives. It is not a membership
record or a permanent promise. An empty catalog is valid.

Applications should reconcile rows that appear, change, or disappear. Product
policy decides which rows an authenticated requester may see. The catalog
provider receives `AuthenticatedPeer` so this filtering can happen before data
is advertised.

Catalog metadata and Stream admission are separate explicit surfaces. A host
should advertise only the Resources it intends to make useful, then mount a
matching Stream provider for streamable rows.

## Catalog v3

Catalog v3 contains four log variants plus receiver-owned message channels:

| Variant | Identity and interpretation |
| --- | --- |
| `sensor_log` | Sensor, clock, optional frame, source/writer peer |
| `pose_log` | From/to frames, clock, source, writer mode |
| `time_transform_log` | From/to clocks and transform source |
| `detection_log` | Detector, input log, input sensor, clock |
| `message_channel` | Receiver owner, resource ID, clock |

The four log rows retain the existing v2 JSON shape inside the v3 contract.
Message channels are additive. This preserves locked bytes without making v2
an actively hosted endpoint.

Common log-row fields include:

- `resource_id` — stable logical identity scoped to the source peer;
- `source_peer_id` — physical origin of the data;
- `writer_peer_id` — peer holding the manifest or bytes;
- `state` — `live` or `sealed`;
- `head` or `extent` — open or closed time coverage; and
- `available` — current byte, entry, and duration coverage.

`source_peer_id` and `writer_peer_id` are equal for origin data. A cache or
materializer preserves the source and changes the writer.

Sensor rows keep three separate axes:

| Axis | Example |
| --- | --- |
| Resource variant | `sensor_log` |
| Closed sensor family | `camera`, `rangefinder`, `rf`, `audio`, `joint_encoders`, `scalar` |
| Open family-specific type | `rgb`, `point_cloud`, `wifi`, `pcm`, `battery_charge` |

The `(sensor_id, sensor_hash)` reference resolves the exact Sensor Registry
entry containing byte-level metadata.

## Catalog v4

Catalog v4 advertises Map Logs. Each row binds:

- source and writer Peer IDs;
- stable `resource_id`;
- exact Map Registry reference; and
- exact clock reference.

The Map Registry entry describes the immutable contract. Stream v2 carries the
sequence of `MapUpdate` values.

## Content-addressed interpretation

Catalog rows contain `RegistryRef { peer_id, id, hash }` values. Consumers use
`RegistryClient` to fetch the exact entry from the authenticated owner and
verify all identity fields plus the content hash.

Device Model discovery is different from resource discovery. Registry v3
`List(DeviceModel)` returns current `(id, hash)` tips; `Get` retrieves an exact
immutable entry. Referenced URDF and mesh bytes are fetched through Blob v1 and
verified by SHA-256.

Native applications can mount:

```rust,ignore
let registries = RegistryEndpoint::mount(
    peer.protocols(),
    FsRegistryProvider::new(app_root.clone(), peer.peer_id()),
)?;
let blobs = BlobEndpoint::mount(
    peer.protocols(),
    FsBlobProvider::new(app_root),
)?;
```

Both filesystem providers are fixed-root and read-only.

## Projecting a local Session

`SessionProtocolProvider::new(&recording_peer, &session)` is the native,
mechanical bridge from `auki-session` to portable protocols. It requires the
exact `Peer` instance that created the Session.

The provider currently supplies:

- Catalog v3 rows for Sensor, Pose, TimeTransform, and Detection Logs;
- Catalog v4 rows for Map Logs; and
- Stream v2 replay/live sources for Map and Detection Logs.

It does not decide authorization. An application may wrap or replace the
provider when different authenticated peers should see different resources.

```rust,ignore
let provider = SessionProtocolProvider::new(&recording_peer, &session)?;
let catalog = CatalogEndpoint::mount(peer.protocols(), provider.clone())?;
let streams = StreamEndpoint::mount(peer.protocols(), provider)?;
```

Other Sensor, Pose, or TimeTransform stream sources remain application-owned
until equivalent mechanical adapters exist.

## Consumer flow

A typical consumer:

1. receives an expected Peer ID and exact route from application discovery;
2. fetches Catalog v3 or v4 with `CatalogClient`;
3. selects a row using product rules;
4. resolves its exact metadata with `RegistryClient`;
5. fetches referenced binary content with `BlobClient` when needed; and
6. opens the typed `StreamClient` subscription through the authenticated peer.

Each Client validates the expected peer and response shape. A route alone
never grants access.

## Data lifecycle

- A rolling `head` describes a live retention window.
- A fixed `head` describes an open log with a stable start.
- A sealed `extent` describes a closed time range.
- `available` reports current coverage, not health or authority.
- A stable logical Resource may temporarily disappear and later return with
  the same `resource_id`.

## Out of scope

The catalog does not define peer discovery, route publication, Domain policy,
payment, automatic Mapper/Detector execution, or graph-level pose/time
composition. Those concerns remain explicit at their own layers.
