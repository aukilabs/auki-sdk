# Auki Camera Mesh — Web

The Web app is the visual Camera Mesh publisher and CCTV-style viewer. Browser
tabs can publish bounded JPEG feeds while one viewer discovers and monitors up
to 16 of them over independent authenticated WSS relay routes.

It composes all six standard protocol families: Info, Catalog, Registry,
Stream, Message, and Blob. Open **Diagnostics** from the session or camera menu
to inspect authenticated metadata, stream health, controls, and snapshot
verification without filling the monitoring wall with developer details.

## Run the app

Use Node.js `^20.19.0` or `>=22.12.0`, then run from the SDK root:

```sh
cd examples/camera-mesh/web
npm ci
npm run dev
```

Open the printed loopback URL in at least two tabs, then:

1. sign every tab into the same Domain;
2. start one peer as **Share this camera**, choose its resolution and rate, and
   start another as **Monitor cameras**;
3. start sharing a synthetic source or grant webcam permission;
4. choose **Add camera** in the monitor, then add the discovered publisher;
5. allow the pending Viewer Peer ID in the publisher tab; and
6. retry the camera tile, then try local freeze, source pause/resume, fullscreen,
   and a verified snapshot.

If DDS discovery is unavailable, copy the publisher's sanitized peer card and
paste it under **Add camera → Connect with a peer card**. Add more publisher
tabs to exercise the one- through four-column wall layouts. Camera Mesh keeps a
maximum of 16 peers on the wall.

The discovery sheet's **Add all** control intentionally opens every discovered
camera concurrently, capped by the 16-camera wall. It is a burst/stress path
for validating SDK relay-dial backpressure; adding cameras individually is the
paced operator flow.

The column selector changes density; it does not divide cameras into square
pages. Two, three, and four columns keep every camera in one scrolling wall.
On mobile those choices render as two columns. Choose **1** for focus mode,
where one camera fills the available wall and previous/next controls switch the
focused peer.

Browser identities and publisher approvals are intentionally ephemeral. A Web
publisher selects one immutable profile before starting its peer:

- resolution: 480×270, 960×540, or 1920×1080;
- rate: 5, 25, or 50 fps.

The default remains 480×270 at 5 fps so Rust, Python, and Swift peers keep
interoperating. The Web viewer accepts all nine Web profiles after verifying
their Registry metadata and checks every JPEG against the advertised size.
Restart the publisher peer to change profile; this keeps the Registry hash and
Stream manifest stable for the whole session. The 1920×1080 at 50 fps profile
is deliberately a stress target, not a performance promise. Camera Mesh rejects
JPEG frames larger than 1 MiB.

The application retains only the newest captured frame before each transport
write. On the viewer, one JPEG is decoded at a time into a persistent canvas;
the previous completed frame stays visible until its replacement is ready.
Superseded pending frames are dropped instead of flashing an empty image. The
Stream itself remains reliable and ordered, so sustained capture, network, or
rendering pressure can still reduce throughput or increase frame age; the
diagnostics separate those stages.

The Diagnostics drawer exposes:

- publisher capture/encode FPS, encode p50/p95, and missed capture deadlines;
- received and displayed frames per second over the latest five-second window;
- received KiB/s, with average JPEG size in the tooltip and Diagnostics drawer;
- frame age from the publisher's capture timestamp until the image renders.

Frame age includes clock offset between different devices. It is exact enough
for two tabs or processes on one machine; compare changes over time when the
publisher and viewer have independently synchronized clocks.

## Browser-to-browser smoke

Keep the development server running, then run in another terminal:

```sh
AUKI_EMAIL='developer@example.com' \
AUKI_PASSWORD='...' \
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
npm run smoke -- http://127.0.0.1:5173/
```

`AUKI_DOMAIN_ID` is optional for this test; without it, the first accessible
Domain is selected. The test starts two browser publishers and one browser
viewer by default. It proves simultaneous independent feeds, DDS discovery,
peer-card fallback, approval, source pause/resume, diagnostics, snapshot,
layout changes, removal, reconnection, and responsive mobile layout. Set
`AUKI_CAMERA_WALL_COUNT` from `2` through `16` to increase the publisher count.
Set `AUKI_CAMERA_TEST_RESOLUTION=low|medium|high` and
`AUKI_CAMERA_TEST_RATE=low|medium|high` to choose the first synthetic
publisher's profile while the second publisher remains at the compatibility
default.

## Rust, Python, and Web matrix

The cross-runtime runner builds the Web app and native Rust peer, creates a
temporary Python environment, and starts a publisher and viewer in all three
runtimes:

```sh
AUKI_EMAIL='developer@example.com' \
AUKI_PASSWORD='...' \
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
npm run smoke:matrix
```

This gate requires `AUKI_DOMAIN_ID`, Rust with the WebAssembly target,
`wasm-pack`, Python, `maturin` (or `uv`), and Playwright Chromium. Add
`-- --headed` to watch the two browser peers, or `-- --list` to print the six
directed edges without starting them.

The matrix intentionally uses exact peer cards rather than DDS discovery. It
isolates protocol interoperability across Web↔Rust, Web↔Python, and
Rust↔Python; the browser smoke separately covers the unique browser↔browser
path and the discovery flow.

See the [Camera Mesh guide](../README.md) for the shared JSONL contract and the
complete Phase 2 gate.
