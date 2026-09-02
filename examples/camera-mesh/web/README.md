# Auki Camera Mesh — Web

Camera Mesh is a complete browser-to-browser application built on the portable
Auki peer and protocol facades. One tab publishes a bounded JPEG camera feed;
another discovers it, asks for access, and connects through an authenticated
WSS relay route.

The polished interface is intentional: unlike the minimal SDK teaching
examples, Camera Mesh shows how the six standard protocols compose in a real
application.

- **Info** identifies the remote camera participant.
- **Catalog** advertises the live camera and its control channel.
- **Registry** verifies the Sensor, Clock, and Frame definitions.
- **Stream** carries bounded, independently decodable JPEG frames.
- **Message** carries pause, resume, and snapshot coordination.
- **Blob** transfers the snapshot and verifies its SHA-256 hash.

## Run it

```bash
npm ci
npm run dev
```

Open the printed loopback URL in two tabs. Sign both into the same Domain, start
one as **Publisher** and the other as **Viewer**, then:

1. start the publisher with the synthetic source or grant webcam permission;
2. discover cameras from the viewer and request the selected feed;
3. approve the pending Viewer Peer ID in the publisher tab;
4. retry the viewer connection; and
5. try pause, resume, and a verified snapshot.

If DDS discovery is unavailable, copy the publisher's sanitized peer card and
paste it into **Use a copied peer card instead** in the viewer tab.

The Protocol Inspector shows the verified metadata and chronological protocol
operations without exposing credentials. Stop both peers and swap the tab roles
to prove that either browser can publish.

## Deterministic smoke test

With the development server running, the smoke test performs the complete flow
in both directions:

```bash
AUKI_EMAIL=... \
AUKI_PASSWORD=... \
AUKI_DOMAIN_ID=... \
npm run smoke -- http://127.0.0.1:5173/
```

`AUKI_DOMAIN_ID` is optional; the test uses the first accessible Domain when it
is omitted. The forward run uses DDS plus the synthetic camera. The reverse run
uses a copied peer card plus Chromium's fake webcam device. Together they cover
explicit approval, all six protocol families, ordered shutdown, and role
reversal without requiring physical camera hardware in CI.

Browser identities and the publisher allow-list are deliberately ephemeral.
The initial stream is fixed at 480×270, 5 fps, and JPEG quality 0.65. Capture
retains only the newest encoded frame, so a slow consumer cannot create an
ever-growing latency queue.
