# Browser standard-protocol node

This page starts one ephemeral relay-backed browser peer, mounts all six
standard protocol families, and explicitly chooses DDS **discover only** or
**discover and advertise** behavior. The latter is selected by default.

```bash
npm ci
npm run dev
```

Open the printed loopback URL in two tabs and start both peers in the same
Domain with **discover and advertise**. Each tab intentionally receives a
fresh peer identity. Select **Discover**, choose Browser B in Browser A, and
select **Probe selected peer**. Repeat A from B. These are distinct A → B and
B → A checks; each caller selects the discovered WSS route only when probing.

DDS results are short-lived, untrusted candidates. The protocol connection
still verifies the exact Peer ID and selected Domain before exposing data. The
manual peer-card field remains available for discover-only/private peers and
tracker-outage testing.

The [protected four-peer matrix](../README.md#protected-four-peer-matrix) also
drives the narrow `window.aukiE2e` hook. That hook is example-only and is not
part of the SDK's public browser API.
