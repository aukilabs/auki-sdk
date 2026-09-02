# Auki Camera Mesh — Web

The Web app is the visual Camera Mesh publisher and viewer. One browser tab can
publish a bounded JPEG feed; another can discover it, request access, and view
it over an authenticated WSS relay route.

It composes all six standard protocol families: Info, Catalog, Registry,
Stream, Message, and Blob. Open the Protocol Inspector to see the authenticated
metadata, stream activity, controls, and snapshot verification.

## Run the app

Use Node.js `^20.19.0` or `>=22.12.0`, then run from the SDK root:

```sh
cd examples/camera-mesh/web
npm ci
npm run dev
```

Open the printed loopback URL in two tabs, then:

1. sign both tabs into the same Domain;
2. start one peer as **Publisher** and the other as **Viewer**;
3. publish the synthetic source or grant webcam permission;
4. discover the publisher and try to connect;
5. approve the pending Viewer Peer ID in the publisher tab; and
6. reconnect, then try pause, resume, and snapshot.

If DDS discovery is unavailable, copy the publisher's sanitized peer card and
paste it into **Use a copied peer card instead**. Stop both peers and swap their
roles to prove the reverse browser-to-browser direction.

Browser identities and publisher approvals are intentionally ephemeral. The
default feed is 480×270 at 5 fps. Only the newest encoded frame is retained, so
a slow consumer does not create an ever-growing latency queue.

## Browser-to-browser smoke

Keep the development server running, then run in another terminal:

```sh
AUKI_EMAIL='developer@example.com' \
AUKI_PASSWORD='...' \
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
npm run smoke -- http://127.0.0.1:5173/
```

`AUKI_DOMAIN_ID` is optional for this test; without it, the first accessible
Domain is selected. The test runs both directions. One direction discovers the
publisher through DDS with the synthetic source; the other uses a copied peer
card and Chromium's fake webcam device.

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
