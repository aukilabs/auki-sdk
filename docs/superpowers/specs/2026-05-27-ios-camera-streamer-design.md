# iOS Camera Streamer Design

## Goal

Build an iOS app that uses the generated Swift bindings to participate in an Auki cluster as a peer, capture the device camera, log the camera feed to the SDK on-disk format, and stream the same feed so Overwatch can join the cluster and view it live.

The first successful end-to-end run is:

```text
iPhone app
  -> joins or creates an Auki cluster through generated Swift bindings
  -> advertises one camera sensor stream
  -> logs encoded camera frames locally
  -> serves /auki/stream/0.1.0 camera entries

Overwatch
  -> joins the same cluster as a browser peer
  -> discovers the iOS camera sensor
  -> opens the stream
  -> decodes and renders the live camera feed
```

## Current Repo Context

The repo already has the right SDK layers for this milestone.

- `auki-domain` exposes `DomainClusterManager` through generated Swift bindings. It covers cluster bootstrap, participant info, catalog providers, registry providers, resource catalogs, and byte-oriented camera/detection streams.
- `auki-network` owns libp2p transport and `/auki/stream/0.1.0`; Swift should not reimplement libp2p.
- `auki-registry`, `auki-time`, `auki-logs`, `auki-layout`, and `auki-manifests` already expose Swift-safe functions needed for registry entries, session clocks, log paths, log writes, and manifests.
- `AukiProto` is generated locally from root `proto/auki` and gives Swift the `Auki_Camera_CameraFrame` protobuf type.
- `examples/ios/AukiNetworkTestApp` proves an iOS app can consume generated Swift packages, but it is scoped to `/auki/message/0.0.1`.
- `examples/overwatch` already consumes generated JavaScript/WASM SDK packages and opens SDK streams, but its preview path currently treats stream payload bytes as raw JPEG.

## Design Constraints

Swift must remain a host layer. It owns camera capture, UI, local app state, and protobuf data conversion; Rust SDK crates own cluster lifecycle, libp2p, stream framing, registries, logs, and protocol semantics.

The iOS app must consume generated packages rather than hand-written SDK networking. No `swift-libp2p`, no app-specific HTTP backend, and no custom Overwatch-only signaling path.

The app should use `DomainClusterManager`, not `AukiMessageNode`, for the camera milestone. Message-node interop stays useful as a smaller smoke test, but camera streaming needs cluster membership, catalogs, registries, resources, and typed streams.

The v1 cluster flow starts with iOS creating or join-or-creating the cluster, then Overwatch joins it. Browser Manager/create-Domain semantics are still parked in `auki-network` because a browser Manager has stricter reachability and liveness constraints.

## Approach Options

### Recommended: Native iOS Producer Peer

The iOS app is a native producer peer backed by `DomainClusterManager`. It starts a cluster or joins an existing native-managed cluster, advertises a camera sensor stream, writes camera frames to a Sensor Log, and pushes the same encoded payloads to stream subscribers.

This matches the SDK architecture: sensor bytes become logs and streams, identity metadata lives in registries and manifests, and Overwatch consumes the public cluster/resource/stream protocols instead of an app-specific endpoint.

### Alternative: Extend Message-Node Test App

The existing iOS test app could send camera chunks inside `/auki/message/0.0.1` envelopes. This is useful for validating raw native/browser reachability, but it bypasses stream manifests, resource catalogs, sensor registries, and the Sensor Log model. Overwatch would need a special-case receiver, so this path does not prove the product behavior.

### Alternative: Local HTTP Camera Bridge

The iOS app could host an HTTP endpoint and Overwatch could poll it. That would be fast to prototype, but it misses the goal: Overwatch should discover and subscribe to the camera feed through the Auki cluster.

## Recommended Architecture

Create a new app under `examples/ios/AukiCameraStreamer`. Keep `AukiNetworkTestApp` as the message-node smoke harness.

The app imports these generated Swift packages:

```text
auki_network
auki_domain
auki_registry
auki_time
auki_logs
auki_layout
auki_manifests
AukiProto
```

The app has four focused host components:

- `CameraCaptureService`: owns `AVCaptureSession`, converts frames to JPEG, and emits captured frame records on a serial queue.
- `AukiCameraSession`: owns wallet seed loading, session id, `SessionClock`, registry writes, Sensor Log creation, and `DomainClusterManager`.
- `CameraStreamFanout`: tracks active stream ids returned by `acceptStreamOpen`, pushes each encoded frame to every active stream, and removes streams on finish/error.
- `CameraStreamerViewModel`: binds UI controls to start/stop capture, cluster bootstrap, logging status, advertised addresses, and stream status.

## Data Flow

Camera frame flow:

```text
CMSampleBuffer
  -> JPEG bytes
  -> Auki_Camera_CameraFrame { frame = JPEG bytes }
  -> SwiftProtobuf serialized bytes
  -> BytesLog.append(timestamp_ns, payload)
  -> DomainClusterManager.pushStreamEntry(stream_id, DomainStreamEntry)
```

Cluster discovery flow:

```text
wallet seed
  -> peerIdFromWalletSeed
  -> DomainClusterManager bootstrap
  -> static sensor catalog provider
  -> static resource catalog provider
  -> static registry entry provider
  -> Overwatch fetches catalogs/resources
  -> Overwatch opens /auki/stream/0.1.0
```

The stream manifest must identify the same sensor, clock, and frame references that the log manifest uses. That keeps the stream and the materialized Sensor Log aligned for downstream `convert_time` and `convert_pose` infrastructure.

## Registry and Manifest Model

At app startup, the iOS app creates an app root under Application Support, for example:

```text
<Application Support>/AukiCameraStreamer/
```

It writes these registry entries:

- Frame Registry: `<peer_id>/<session_id>/camera_optical`, using the ROS optical convention.
- Clock Registry: the `SessionClock` registry entry produced by `auki-time`.
- Sensor Registry: `<peer_id>/<session_id>/camera`, with body type `camera`, JPEG frame dimensions, capture frame rate, `pixel_format = "jpeg"`, `color_space = "srgb"`, `intrinsics_model = "pinhole"`, `distortion_model = "unknown"`, and the camera optical `(frame_id, frame_hash)`.

It opens one Sensor Log under the current session root:

```text
<app_root>/<session_id>/sensorlogs/ios-camera/
```

The log manifest is built with `buildSensorLogManifestJson`, using:

- `app_id = "ios-camera-streamer"`
- `session_id = <uuid>`
- the camera `(sensor_id, sensor_hash)`
- the session clock `(clock_id, clock_hash)`
- the camera optical `(frame_id, frame_hash)`
- `segment_duration_ns = 1_000_000_000`
- `retention_ns = 300_000_000_000` for the first default, giving a five-minute local ring buffer while the stream is live

## Cluster and Stream Behavior

The first milestone uses `ClusterTargetMode.joinOrCreate`. If the Discovery directory has no cluster by that name, the iOS app becomes the native Manager. If the cluster exists and has a reachable Manager, the app joins it.

After bootstrap, the app installs:

- a static sensor catalog containing the camera row;
- a static resource catalog containing one `sensor_stream` resource;
- static registry entries for the frame, clock, and sensor entries.

The app polls `drainStreamOpenRequests`. For a request matching the camera sensor id:

1. Build a stream manifest from the same sensor/clock/frame values as the log.
2. Call `acceptStreamOpen(responder_id, manifest_json)`.
3. Store the returned `stream_id`.
4. Push future encoded `CameraFrame` payloads to that stream id.

Unknown sensor ids are declined with `sensor_not_found`. Capture stopped or camera permission denied is declined with `sensor_unavailable`.

## Overwatch Compatibility

Overwatch must decode native camera stream payloads as `auki.camera.CameraFrame`.

Today, the Overwatch preview path creates a JPEG `Blob` directly from `StreamEntry.payload`. That works for local browser demo streams that publish raw JPEG bytes, but native Rust stream producers encode the typed camera payload first. The browser consumer should:

1. Inspect the accepted stream descriptor or participant sensor metadata.
2. If the sensor kind is `camera`, decode `StreamEntry.payload` as `CameraFrame`.
3. Use `CameraFrame.frame` as the JPEG bytes for the preview image.
4. Preserve the current raw-byte path for demo or unknown payloads when the accepted descriptor does not identify a camera stream.

This keeps the wire shape consistent with Sensor Logs: camera streams carry the same payload record as camera logs.

## Address Advertisement

The iOS app needs to advertise browser-dialable addresses through Discovery. Passing the configured listen address such as `/ip4/0.0.0.0/udp/0/webrtc-direct` is not enough because peers need the actual emitted listen multiaddr.

Add a small native binding helper in `auki-domain` so Swift can bootstrap a `DomainClusterManager` with auto-advertised WebRTC Direct addresses:

```text
listen_addrs: ["/ip4/0.0.0.0/udp/0/webrtc-direct"]
advertise: collect emitted listen addresses from the runtime
```

The implementation should reuse the existing Rust swarm address-resolution logic instead of duplicating address parsing in Swift. Operator-provided advertise overrides remain valid for local-only or hand-routed test networks.

## UI

The first UI is a dense test/operator surface, not a landing page.

It should show:

- Discovery URL
- cluster name
- cluster mode: join-or-create
- camera permission/capture status
- local peer id
- advertised multiaddrs
- current sensor id/hash
- log root path
- active stream count
- last frame timestamp, sequence, bytes, and fps
- recent event log

Controls:

- Start cluster
- Start camera
- Stop camera
- Stop cluster
- Copy peer id
- Copy advertised addresses
- Clear event log

## Error Handling

Camera permission denial blocks capture and stream acceptance with `sensor_unavailable`, while the cluster can remain running.

Cluster bootstrap errors surface the Discovery URL, cluster name, and target mode. If join-or-create loses a create race and returns an already-exists error, the app retries once with join.

Registry write failures stop bootstrap because catalogs without resolvable registry entries would make Overwatch display an ambiguous stream.

Log append failure stops logging and stream push together for the first milestone. The app should not present a stream as live if it cannot materialize the same bytes locally.

Stream push failure removes that stream id from fanout and records the peer/request context in the event log.

## Test Plan

Rust binding tests:

- Add or extend an `auki-domain` Swift smoke path proving a generated Swift host can bootstrap with auto-advertised WebRTC Direct addresses.
- Add a focused test for the new auto-advertise helper so emitted listen addresses are used for Discovery registration.

Swift unit tests:

- Verify camera registry JSON hashes are stable for a fixture sensor/frame pair.
- Verify `Auki_Camera_CameraFrame(frame: jpegBytes).serializedData()` decodes back to the same frame bytes.
- Verify `CameraStreamFanout` accepts a stream id, pushes entries in sequence, and drops finished streams.

Overwatch tests:

- Add a fixture where `StreamEntry.payload` is an encoded `CameraFrame`, then assert `preview.ts` extracts the inner JPEG bytes.
- Keep the raw JPEG fixture for browser demo streams.

Manual end-to-end:

1. Generate Swift and JavaScript bindings plus Swift/JavaScript proto packages.
2. Build and run `AukiCameraStreamer` on an iPhone or simulator with camera substitution.
3. Start a cluster through Discovery.
4. Start camera capture.
5. Open Overwatch against the same Discovery URL and cluster.
6. Confirm the iOS peer appears with one camera sensor.
7. Open the live video tile.
8. Confirm live frames render and sequence/fps update.
9. Stop camera capture and confirm Overwatch reports the stream as ended or unavailable.
10. Inspect the app root and confirm Sensor Log manifest and segments exist.

## Milestones

### Milestone 1: Local Generated-Binding Build

Generate all required Swift packages and build the new app target. The app can create a session clock, write registry entries, and encode a fixture `CameraFrame`, without joining a cluster.

### Milestone 2: Camera Logging

The app captures camera frames, encodes `CameraFrame` protobuf payloads, appends them to `BytesLog`, and shows frame/log stats in UI.

### Milestone 3: Native Cluster Producer

The app bootstraps a cluster with `DomainClusterManager`, advertises the camera catalog/resource/registry entries, and accepts stream open requests from another native test runtime.

### Milestone 4: Overwatch Interop

Overwatch joins the same cluster, discovers the iOS camera stream, decodes `CameraFrame`, and renders the live feed.

### Milestone 5: Operational Polish

Add permission recovery, background/foreground handling, retention controls, stream subscriber visibility, and a concise runbook.

## Non-Goals

- No Swift libp2p implementation.
- No iOS HTTP preview endpoint.
- No Overwatch backend server.
- No full `convert_time` or `convert_pose` implementation.
- No browser Manager/create-Domain semantics in this milestone.
- No ARKit pose publishing in the first camera-feed slice.
- No audio, depth, or multi-camera support in the first slice.

## Review Notes

This design intentionally keeps the iOS app as a normal SDK producer. It does not introduce a special camera transport. The live stream and the local log carry the same typed payload bytes, and the identity metadata needed to interpret those bytes is served through the existing registry and resource protocols.
