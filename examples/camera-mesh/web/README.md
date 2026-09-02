# Auki Camera Mesh — Web

This is the first application built on the portable Auki peer and protocol
facades. One browser tab publishes a bounded JPEG camera feed and another tab
discovers, requests access to, and views it through an authenticated WSS relay
route.

```bash
npm ci
npm run dev
```

Open the printed loopback URL in two tabs. Sign both into the same Domain, start
one as **Publisher** and the other as **Viewer**, then:

1. start the publisher with the synthetic source or grant webcam permission;
2. discover cameras from the viewer and request the selected feed;
3. approve the pending Viewer Peer ID in the publisher tab; and
4. retry the viewer connection.

Browser identities and the publisher allow-list are deliberately ephemeral.
The initial stream is fixed at 480×270, 5 fps, and JPEG quality 0.65. Capture
retains only the newest encoded frame, so a slow consumer cannot create an
ever-growing latency queue.

