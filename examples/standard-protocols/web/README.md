# Browser standard-protocol node

This page starts one ephemeral relay-backed browser peer and mounts all six
standard protocol families. Paste a peer card from another tab, native node,
or Python node and select **Probe all six**.

```bash
npm ci
npm run dev
```

Open the printed loopback URL in two tabs and start both peers in the same
Domain. Each tab intentionally receives a fresh peer identity. Paste Browser
B's complete peer card into Browser A and select **Probe all six**, then paste
A's card into B and repeat. These are distinct A → B and B → A checks; each
caller uses the target card's exact WSS circuit route. The app does not pretend
discovery exists yet.

The [protected four-peer matrix](../README.md#protected-four-peer-matrix) also
drives the narrow `window.aukiE2e` hook. That hook is example-only and is not
part of the SDK's public browser API.
