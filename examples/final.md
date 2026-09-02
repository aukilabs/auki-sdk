# Auki Camera Mesh demo

## Purpose

Build one meaningful, inspectable application on the portable `AukiPeer`
protocol stack: authenticated peers advertise cameras, publish camera frames,
consume remote feeds, exchange controls, and fetch verified snapshots.

The demo should work across the SDK runtimes without reimplementing protocol or
transport behavior in each host:

- a Web application can publish a webcam and view remote feeds;
- an iOS application can view feeds and, in a later phase, publish its camera;
- Python and native Rust applications can publish deterministic mock or
  prerecorded camera data; and
- any supported client can inspect how the six standard protocol families
  participate in the same application flow.

This is an SDK teaching and interoperability demo, not a replacement for an
adaptive production video stack.

## User experience

The default interface stays focused on the camera experience:

- local preview and publish controls;
- discovered camera publishers, with manual peer-card entry as a fallback;
- pending viewer requests with explicit allow/deny controls;
- remote camera tiles;
- publisher name and connection state;
- camera and quality selection where the platform supports them; and
- start, pause, resume, snapshot, and stop actions.

Start and stop are local publisher lifecycle controls: they mount or close the
camera endpoints and update discovery publication. Pause, resume, quality,
camera selection, and snapshot are remote Message controls available only after
the publisher approves the requester.

A separate **Protocol Inspector** reveals the technical detail. On desktop it
can be a side drawer; on mobile it can be a bottom sheet. The camera remains
usable while the inspector is closed, and raw protocol detail appears only on
demand.

## Protocol composition

### Info v1: identify the participant

Info describes the application participant behind an authenticated peer, for
example `Matt's iPhone`, `Chrome webcam`, or `Python mock camera`. The inspector
shows the app and version, participant name, session identifiers, session
clock, Peer ID, and app-instance label.

Info is diagnostic metadata. It is not an authorization roster and does not
publish credentials, bearer authority, routes, or Domain membership.
The demo may expose a minimal Info document to authenticated same-Domain peers,
but receiving it does not place the requester on the camera access list.

### Catalog v3: advertise available resources

Catalog advertises currently usable camera resources and live Message channels.
A camera row identifies its source, writer, stable resource ID, availability,
and Registry references. A control channel identifies its receiver-owned
resource and clock.

The inspector renders both readable resource cards and the exact decoded
Catalog document. Catalog answers “what does this known peer currently offer?”;
it does not discover remote peers.

The Catalog provider returns camera and control resources only to requesters on
the publisher's session-scoped access list. An unapproved request returns an
empty camera catalog and appears as a pending approval in the local publisher UI;
the viewer can retry after approval.

### Registry v3: explain camera data

Registry resolves the immutable definitions needed to interpret a camera feed:

- the Camera Sensor definition, including resolution, image encoding, pixel
  format, and calibration where available;
- the Clock definition used by frame timestamps;
- the Frame definition and coordinate convention; and
- an optional Device Model definition.

The inspector groups entries by Registry kind and shows each ID, expected hash,
canonical JSON, recomputed hash, verification result, owner, and the Catalog or
Stream field that referenced it.

The Registry provider applies the same access list before returning camera
definitions.

### Stream v2: carry the live feed

Stream carries timestamped `CameraFrame` values. A subscriber selects the
advertised camera resource and initially requests `Latest`, avoiding historical
replay. The producer accepts with a manifest that pins the camera, clock, frame,
and payload contract.

The first implementation uses independently decodable JPEG frames inside
`CameraFrame.frame`. It deliberately limits resolution, frame rate, and JPEG
quality. Capture feeds a bounded latest-frame buffer: when transport or a
consumer is slow, stale frames are dropped rather than building an increasingly
late video queue.

The inspector shows the manifest, payload kind, sequence, source timestamp,
observed frame rate, bytes per second, frame size, received/displayed/dropped
counts, backpressure state, selected route, and terminal end reason. It may
decode the `CameraFrame` envelope but must not dump image bytes into the UI.

The Stream provider declines camera subscriptions from requesters that have not
been approved locally for the current publisher session.

### Message v1: control the publisher

Message provides receiver-owned live control channels. Initial message types can
include:

- `camera.pause` and `camera.resume`;
- `camera.set_quality`;
- `camera.select` for front/back or multiple webcams; and
- `camera.request_snapshot`.

The inspector shows the channel identity, authenticated sender, message type,
timestamp, sequence, queue state, and ACK. It must explain that an ACK means the
message entered the bounded receiver queue, not that it was durably stored or
fully processed.

Message v1 has no application admission callback before queue acceptance. The
control consumer therefore checks `MessageEvent.sender` against the same access
list before performing any camera action. Unauthorized events are ignored even
if their transport ACK was returned.

### Blob v1: transfer verified snapshots

A snapshot request produces a content-addressed full-resolution image. The
publisher announces the resulting SHA-256 through an application Message, and
the requester fetches it through Blob.

The inspector shows the serving peer, hash, total size, chunk progress, route,
transfer duration, and final SHA-256 verification. Blob is also suitable for
camera calibration or other immutable assets that do not belong in the live
frame stream.

The Blob provider checks the authenticated requester against the access list;
knowing a snapshot hash alone does not grant access to its bytes.

## Protocol Inspector

The collapsed inspector begins with compact summaries such as:

```text
INFO ready · CATALOG 2 resources · REGISTRY 3 verified
BLOB idle · MESSAGE 1 channel · STREAM camera/front 8 fps
```

Selecting a family opens progressively deeper views:

1. a human-readable summary;
2. decoded typed fields;
3. raw JSON or protobuf-envelope information; and
4. recent operations and errors.

A Timeline view explains the complete connection in order:

```text
Discovered a peer advertising the exact Stream protocol
Authenticated expected Peer ID in the selected Domain
Fetched participant Info
Catalog returned camera/front and camera/control
Resolved and verified Sensor, Clock, and Frame Registry entries
Requested Stream camera/front from Latest
Stream accepted camera_frame
Received frame sequence 0
Requested snapshot through Message
Fetched and verified snapshot through Blob
```

Useful inspector controls include pause/freeze, search, errors-only filtering,
clear, copy a sanitized peer card, copy a decoded object, and export a sanitized
diagnostic snapshot. Credentials, bearer tokens, private keys, and raw authority
material must never appear.

The inspector also shows whether the remote Peer ID is pending, allowed, or
denied for the current publisher session and which provider enforced each
decision.

An optional **Explain this connection** mode walks through the same timeline as
a guided SDK lesson.

## Runtime roles

### Web

The Web application is the first complete publisher and viewer. It uses
`getUserMedia()` for capture, browser image APIs for JPEG compression, generated
protobuf types from the canonical Auki schemas, an ephemeral `AukiPeer`, and an
exact WSS relay route.

The publisher opts into DDS `DiscoverAndAdvertise` so its short-lived Peer ID,
routes, and exact mounted protocols remain discoverable. A viewer can use
`DiscoverOnly`, filter candidates by the exact Stream protocol ID, and then dial
an advertised WSS route. The app must still authenticate the expected Peer ID and
selected Domain before trusting protocol data. Manual peer cards remain a useful
fallback when DDS discovery is disabled.

### Native Rust

The Rust host uses the native `AukiPeer` and standard protocol endpoints. It can
publish a deterministic generated image, a checked-in image sequence, or a local
prerecorded file. Native prost types encode `CameraFrame` directly. It should
also be able to consume a Web or iOS feed and report or save decoded frames.

### Python

The Python host mirrors the Rust mock producer and consumer through the Rust-backed
SDK extension. It uses the generated `auki_datatypes.camera.CameraFrame` type for
protobuf encoding and can use Pillow or OpenCV at the application edge for image
generation, JPEG encoding, display, or file output.

### Swift/iOS

The first iOS milestone is a consumer: select a remote peer card, resolve its
camera metadata, subscribe, decode JPEG frames, and display them.

Camera publishing follows as a separate milestone using AVFoundation. It owns
permission UX, capture session configuration, JPEG compression, canonical
`CameraFrame` protobuf encoding, orientation, camera switching, interruption
handling, and a bounded bridge from capture callbacks to the async Stream
producer.

The initial iOS publisher is foreground-only. Background camera operation and
platform-specific background execution policy are non-goals.

## Delivery phases

### Phase 1: browser-to-browser camera

- Two browser peers authenticate into the same selected Domain.
- The publisher advertises through the opt-in DDS tracker, and the viewer
  discovers it by the exact Stream protocol ID.
- The publisher explicitly approves the viewer's authenticated Peer ID for the
  current session before exposing camera resources or bytes.
- One publishes a webcam through an exact WSS relay route.
- The other resolves its metadata and displays the feed.
- The same application supports swapping publisher and viewer roles in a second
  run; simultaneous bidirectional publishing is not required for Phase 1.
- The Protocol Inspector explains Info, Catalog, Registry, and Stream activity.
- Message pause/resume and snapshot request are available.
- Blob fetches and verifies the requested snapshot.
- Both peers close endpoints before shutting down.

### Phase 2: deterministic native peers

- Rust and Python publish stable mock or prerecorded feeds.
- Rust and Python can consume at least one browser feed.
- Cross-runtime payload bytes and manifests remain compatible with the Web app.
- Deterministic sources support repeatable local and CI checks.

### Phase 3: iOS consumer

- The foreground iOS app consumes browser, Rust, or Python camera feeds.
- It renders frames and exposes the same protocol inspection model.
- Lifecycle interruption performs ordered subscription, endpoint, and peer
  cleanup.

### Phase 4: iOS publisher

- The foreground iOS app publishes front or back camera frames.
- Web and at least one native runtime consume the feed.
- Capture never blocks the camera callback on network progress; stale frames are
  dropped through a bounded latest-frame bridge.
- Camera switching, interruption, and shutdown are deterministic.

## MVP acceptance criteria

The first meaningful demo is complete when:

1. one browser tab can publish a webcam and a second can consume it over an
   authenticated, relay-backed `AukiPeer` connection, and the same flow passes
   again after swapping publisher and viewer roles;
2. the opt-in DDS tracker can advertise and discover a publisher by the exact
   Stream protocol ID, while manual peer-card entry remains available;
3. the target Peer ID, selected Domain, and exact route are verified before
   protocol data is exposed;
4. Catalog advertises a live camera and control channel;
5. Registry resolves and verifies the camera Sensor, Clock, and Frame entries;
6. Stream carries bounded JPEG `CameraFrame` values without an unbounded latency
   queue;
7. Message can pause/resume the publisher and request a snapshot;
8. Blob returns the snapshot only after full SHA-256 verification;
9. the inspector shows readable and raw views plus a chronological protocol
   trace without exposing secrets;
10. an authenticated same-Domain peer that is not on the session access list
    cannot enumerate camera resources, resolve camera Registry entries,
    subscribe to frames, fetch snapshots, or cause a camera control action;
11. terminal failure is visible rather than silently leaving a frozen feed; and
12. subscriptions, channels, mounted endpoints, relay bookings, discovery
    publication, and peers shut down in the documented order.

## Boundaries and non-goals

- Relay allocation provides reachability. The separate opt-in DDS tracker
  provides short-lived same-Domain discovery; it is not libp2p Rendezvous.
- Discovered candidates, peer cards, routes, Catalog entries, and `known_peers()`
  observations are not authorization.
- Domain authentication grants transport admission, not camera access. The MVP
  uses an in-memory, session-scoped allow/deny list of exact authenticated Peer
  IDs populated by an explicit local approval. Catalog, Registry, Stream, and
  Blob providers enforce it; the Message consumer checks it before applying a
  control. The list expires when the publisher stops.
- Browser and initial iOS identities may be ephemeral.
- App secrets remain restricted to trusted native hosts and are never embedded
  in browser or mobile clients.
- The MVP does not provide adaptive bitrate, congestion-aware video encoding,
  audio/video synchronization, recording retention, multi-party conferencing,
  or background iOS camera streaming.
- If the product later requires high-resolution, high-frame-rate interactive
  video, a dedicated media transport such as WebRTC may carry media while Auki
  continues to own identity, authorization, resource description, coordination,
  and inspectability.
