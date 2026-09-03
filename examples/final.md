# Camera Mesh delivery target

Camera Mesh is the end-to-end SDK example: authenticated peers advertise a
camera, grant access to an exact viewer Peer ID, stream JPEG frames, exchange
controls, and transfer a verified snapshot.

It is a teaching and interoperability app, not a production video stack.

## Application contract

- **Info** identifies the participant.
- **Catalog** advertises `camera/main` and its control channel.
- **Registry** defines the camera, clock, and frame metadata.
- **Stream** carries bounded, independently decodable JPEG frames.
- **Message** carries pause, resume, and snapshot coordination.
- **Blob** transfers the snapshot and verifies its SHA-256 hash.

DDS discovery and peer cards provide short-lived route hints. Opening a
protocol still authenticates the expected Peer ID and Domain. Domain admission
does not grant camera access: the publisher keeps an in-memory, session-scoped
allow-list, and camera resources remain unavailable until the operator approves
the viewer.

## Delivered

### Phase 1 — Web camera

- one browser app can publish or view;
- the publisher uses a synthetic source or webcam;
- DDS discovery and copied peer-card fallback both work;
- explicit approval protects Catalog, Registry, Stream, Blob, and control
  effects; and
- the Protocol Inspector explains the six-family flow.

### Phase 2 — deterministic native peers

- Native Rust and Python can each publish and view.
- Both deterministic publishers use the same checked-in 480×270 JPEG fixture.
- Their JSONL control surfaces support discover, view, approve, pause, resume,
  snapshot, and ordered shutdown.
- The browser-to-browser smoke runs both browser directions.
- The six-edge matrix proves Web↔Rust, Web↔Python, and Rust↔Python.
- Every matrix edge proves pre-approval rejection, metadata resolution, frames,
  pause/resume, and a SHA-256-verified snapshot; the runner then shuts down all
  six peers and its temporary state.

The executable commands and environment are documented in the
[Camera Mesh guide](camera-mesh/README.md).

### Phase 3 — Swift/iOS viewer

- The foreground SwiftUI viewer logs in a User, explicitly selects a Domain,
  and starts an ephemeral discover-only peer.
- It selects DDS Stream publishers or a pasted peer card, authenticates the
  exact Peer ID and Domain, and supports approval-and-retry.
- It resolves and validates camera metadata, displays bounded JPEG frames and
  sequence counts, sends pause/resume, and fetches a SHA-256-verified snapshot.
- Backgrounding or selecting Stop performs generation-fenced, ordered Stream,
  endpoint, and peer cleanup.
- Offline contract/app tests and an unsigned generic iOS arm64 build pass.

The implementation and phone commands are documented in the
[Swift/iOS Camera Mesh guide](camera-mesh/swift/README.md).

## Phase 4 — Swift/iOS publisher

- The same SwiftUI app can publish its foreground iPhone camera or view another
  Camera Mesh publisher.
- AVFoundation owns camera permission and produces a bounded 480×270 JPEG feed
  at 5 fps; it contains no SDK or protocol policy.
- Rust owns exact-viewer approval, Catalog and Registry metadata, Stream
  delivery, Message controls, Blob snapshots, and ordered shutdown.
- Publishing uses DDS `discover_and_advertise`; viewing remains
  `discover_only`.

The acceptance gate is a physical iPhone publisher consumed by both a Web and
a deterministic native viewer. Each direction must prove pre-approval
rejection, live frames, pause/resume, a SHA-256-verified snapshot, and clean
foreground shutdown.

## Deliberate boundaries

- Browser and initial iOS identities may be ephemeral.
- App secrets stay on trusted native hosts; browser and mobile clients use User
  authentication.
- The feed is fixed and bounded; adaptive bitrate, audio, recording retention,
  multi-party conferencing, and background iOS capture are out of scope.
- Relay allocation provides reachability. DDS tracker discovery only tells the
  application which current candidates it may choose to dial.
