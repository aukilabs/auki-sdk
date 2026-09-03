# Auki Camera Mesh

Camera Mesh is a small camera application built from the six standard Auki
protocol families. It is the interoperability example to use after the smaller
[`portable-echo`](../portable-echo/README.md) and
[`standard-protocols`](../standard-protocols/README.md) playgrounds.

Camera Mesh supports publisher and viewer roles in all four runtimes:

| Runtime | Publisher | Viewer | Source |
| --- | --- | --- | --- |
| Web | Yes | Yes | synthetic image or webcam |
| Native Rust | Yes | Yes | checked-in deterministic JPEG |
| Python | Yes | Yes | the same deterministic JPEG |
| Swift/iOS | Yes | Yes | foreground iPhone camera or JPEG viewer |

The Rust and Python programs have a JSON Lines control surface so people and
test runners can drive the same application flow without a UI. The browser has
a responsive CCTV wall for up to 16 simultaneous feeds, with protocol and live
stream diagnostics kept in a drawer. Headless viewers validate the metadata and
JPEG bytes, then report frame counts and hashes.

## How the protocols fit together

| Protocol | Camera Mesh use |
| --- | --- |
| Info | identify the remote participant |
| Catalog | advertise the camera and control channel |
| Registry | define the camera, clock, and frame metadata |
| Stream | carry bounded, independently decodable JPEG frames |
| Message | pause, resume, and coordinate snapshots |
| Blob | fetch and SHA-256 verify a snapshot |

A publisher exposes camera resources only after the operator approves the
viewer's authenticated Peer ID. DDS discovery and copied peer cards provide
route hints; the selected Peer ID and Domain are still authenticated when a
protocol connection opens.

## Try it

- [Web app](web/README.md): the visual publisher and viewer
- [Native Rust peer](native/README.md): deterministic publisher or viewer
- [Python peer](python/README.md): deterministic publisher or viewer
- [Swift/iOS app](swift/README.md): physical-device foreground publisher or viewer

Publishers default to DDS `discover_and_advertise`; viewers default to
`discover_only`. Headless Rust and Python peers can override either default
with `AUKI_DISCOVERY_MODE`. Every runtime gets both TCP and WSS relay routes,
then selects the route it can use when dialing another peer.

## Native and Python JSONL contract

Both headless peers print a `ready` event whose `card` can be passed directly
to another peer:

```json
{"event":"ready","runtime":"native","role":"publisher","card":{"version":1,"domainId":"...","peerId":"...","protocols":["/auki/auth/1/info/1.0.0","/auki/auth/1/resources/0.3.0","/auki/auth/1/resources/0.4.0","/auki/auth/1/registries/0.3.0","/auki/auth/1/blobs/0.1.0","/auki/auth/1/message/0.1.0","/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}}}
```

Send one JSON object per line on stdin:

```json
{"command":"discover","id":"find","protocol":"/auki/auth/1/stream/0.2.0"}
{"command":"view","id":"view","target":{"version":1,"runtime":"browser","domainId":"...","peerId":"...","protocols":["/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}},"frames":3}
{"command":"exercise_live","id":"live","target":{"version":1,"runtime":"swift","domainId":"...","peerId":"...","protocols":["/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}},"requestId":"live-snapshot"}
{"command":"approve","id":"allow","peerId":"12D3KooW..."}
{"command":"pause","id":"pause","target":{"version":1,"runtime":"browser","domainId":"...","peerId":"...","protocols":["/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}}}
{"command":"resume","id":"resume","target":{"version":1,"runtime":"browser","domainId":"...","peerId":"...","protocols":["/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}}}
{"command":"snapshot","id":"snapshot","requestId":"snapshot-1","target":{"version":1,"runtime":"browser","domainId":"...","peerId":"...","protocols":["/auki/auth/1/stream/0.2.0"],"routes":{"tcp":"...","wss":"..."}}}
{"command":"shutdown","id":"stop"}
```

The first `view` is expected to fail with `approval_required` and emit an
`approval_required` event on the publisher. Approve that exact viewer Peer ID,
then retry. Command results keep the supplied `id`, which makes the interface
safe for simple scripts. Treat `requestId` as unique within one camera session;
omit it when you want the headless peer to generate a UUID.

| Send to | Command | Result or event |
| --- | --- | --- |
| Viewer | `discover` | `discovery_result` |
| Viewer | `view` | `view_result`; first attempt also emits `approval_required` on the publisher |
| Viewer | `exercise_live` | one continuous frames → pause → resume → verified-snapshot report |
| Publisher | `approve` | `approve_result` |
| Viewer | `pause` / `resume` | `control_result`; publisher emits `control_received` |
| Viewer | `snapshot` | `snapshot_result`; publisher emits `snapshot_staged` |
| Each peer | `shutdown` | `shutdown_ack`, then process exit |

## Phase 2 acceptance gates

The two gates cover different network paths and both must pass. Prerequisites
are Rust with `wasm32-unknown-unknown`, `wasm-pack`, Node.js/npm, Python,
`maturin` (or `uv`), and Playwright Chromium:

```sh
cd examples/camera-mesh/web
npm ci
npx playwright install chromium

export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
```

First, keep `npm run dev` running in one terminal and run the unique
browser-to-browser multi-camera flow in another:

```sh
npm run smoke -- http://127.0.0.1:5173/
```

Then run the self-contained cross-runtime matrix (it builds and starts all six
peers itself):

```sh
npm run smoke:matrix
```

The matrix proves these six directed edges:

```text
Rust   -> Web       Rust   -> Python
Python -> Web       Python -> Rust
Web    -> Rust      Web    -> Python
```

Each edge proves rejection before approval, all camera metadata checks, two
frames, pause/resume, and a verified snapshot. The matrix then shuts down all
six peers and their temporary state. The separate browser smoke proves that one
browser viewer keeps multiple browser publishers independent while mixing DDS
discovery and copied peer cards.

## Swift/iOS gates

The [Swift/iOS app](swift/README.md) uses an ephemeral identity, explicit Domain
selection, and ordered foreground lifecycle cleanup. Its viewer discovers and
consumes Web or native publishers. Its publisher advertises a bounded 480×270,
5 fps iPhone camera and keeps approval, protocol serving, controls, and
snapshots in the shared Rust Camera Mesh application.

Phase 3 proves Web/native publisher to iPhone viewer. Phase 4 reverses the
direction: a physical iPhone publisher must reject an unapproved viewer, then
serve live frames, pause/resume, and a verified snapshot to Web and native
viewers. The Swift guide contains the physical-device sequence.
