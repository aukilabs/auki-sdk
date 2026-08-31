# auki-session

Network-free local recording model for one application peer.

`Peer` owns the stable application identity and content-addressed registries.
Each `Peer::start_session()` creates one independent recording timeline with a
fresh Session ID, clocks, and logs. This crate has no credentials, DDS/DMS
client, relay, route, or P2P runtime.

## Peer and registries

```rust,ignore
let peer = Peer::new(peer_id.to_string(), "my-app")
    .with_storage_root(app_root);

let frame = peer.register_frame("base", FrameDef::ros_body())?;
let sensor = peer.register_sensor("camera/front", sensor_body)?;
let session = peer.start_session()?;
```

`Peer` registers Sensor, Frame, Detector, Map, and Device Model metadata under
its storage root. Sessions own Clock metadata. Entries are immutable and
referenced by `RegistryRef { peer_id, id, hash }`.

`Peer::registries()` returns a cheap snapshot handle used by local adapters and
applications. `Peer::owns_session()` verifies that a Session came from the
exact same `Peer` instance, not merely a peer with the same textual ID.

## Session and logs

Starting a Session mints monotonic and UTC clock entries. Applications may add
more clocks and register:

- `SensorLogHandle` from `SensorLogSpec`;
- `PoseLogHandle` from `PoseLogSpec`;
- `TimeTransformLogHandle` from `TimeTransformLogSpec`;
- `DetectionLogHandle` from `DetectionLogSpec`; and
- `MapLogHandle` from `MapLogSpec`.

Every handle carries a stable `resource_id`, exact `LogRef`, canonical
manifest, and local storage path or writer. Duplicate logical resources are
rejected.

`Session::logs()` returns a cheaply cloneable view of the current handles.
Map and Detection handles expose durable replay, live subscription, and an
atomic replay/live boundary used by protocol adapters.

## Put a Session on the network

Networking is composed outside this crate. Enable `session-adapter` in
`auki-protocols` and mount its provider on a running `AukiPeer`:

```rust,ignore
let provider = SessionProtocolProvider::new(&peer, &session)?;
let catalog = CatalogEndpoint::mount(auki_peer.protocols(), provider.clone())?;
let streams = StreamEndpoint::mount(auki_peer.protocols(), provider)?;
```

`SessionProtocolProvider` projects current Session metadata into Catalog v3/v4
and provides Stream v2 sources for Map and Detection Logs. It is mechanical and
does not decide which authenticated peers should receive data. Applications can
wrap or replace it for requester-specific policy.

Registry and blob serving use the native `FsRegistryProvider` and
`FsBlobProvider` from `auki-protocols`; they are not dependencies of this
crate.

## Detectors

`RegisteredCameraDetector` is the bring-your-own camera detector entry point.
The application registers an implementation and accepted sensor contracts,
then starts explicit instances with selected input, cadence, output, and
lifetime.

Two execution styles share the same validation and provenance pipeline:

- recorded/local `DetectorTask` processes accepted samples in order; and
- `StreamingDetectorTask` consumes an async frame stream with bounded
  latest-wins pending work.

`CameraFrameHub` provides bounded `Arc<CameraFrame>` fanout for viewers,
caches, and multiple detectors. Slow subscribers skip overwritten frames
instead of blocking the producer.

Components are not network Resources. Their Detection or Map Logs are the
objects catalogs describe and peers consume.

## Mappers and materialization

`MapLogHandle` is the local sink used by `auki-mappers`. Transport consumers
can convert a validated `StreamSubscription<T>` into mapper inputs without
adding a network dependency here.

Remote-log materialization and static-transform resolution remain explicit
stubs returning `MaterializationError::NotImplemented`.

## Dependency direction

`auki-session` depends on local data crates such as `auki-registry`,
`auki-manifests`, `auki-logs`, `auki-datatypes`, and `auki-jcs`.

It deliberately does not depend on `auki-sdk`, `auki-p2p`, `auki-auth`, or
`auki-protocols`. Networking and protocol adapters depend on the Session model,
never the other way around.
