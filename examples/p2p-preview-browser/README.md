# Auki P2P Preview Browser

Browser peer demo for exercising RFC-first `auki-p2p-browser` connectivity.
It can subscribe to native Sentinel preview offers and publish a generated
browser preview offer for another browser peer.

This example imports `crates/auki-p2p-browser` from source so the demo follows
the SDK package API directly. It assumes `crates/auki-protocol-wasm/pkg-web`
exists; rebuild it from `crates/auki-p2p-browser` with
`npm run build:protocol-wasm` after protocol binding changes.

## Start

Run one native Sentinel relay/bootstrap peer:

```sh
cargo run -p auki-p2p-preview-sentinel -- \
  --peer-label sentinel-a \
  --seed-byte 7 \
  --domain-label sentinel-a-domain \
  --domain-nonce-byte 42 \
  --offer-id sentinel-a-preview \
  --bootstrap-json /tmp/auki-sentinel-a.json \
  --trace-p2p
```

Start the browser app:

```sh
cd examples/p2p-preview-browser
npm run dev -- --port 5174
```

Open `http://127.0.0.1:5174/`, click `Start Peer`, click `Add Peer`, and paste
the Sentinel bootstrap JSON from `/tmp/auki-sentinel-a.json`.

For two distinct browser peers, use two separate browser profiles, or one normal
window and one private/incognito window with separate IndexedDB storage. Two
tabs in the same browser profile intentionally reuse the same persisted peer
seed and therefore the same peer id.

## Manual Matrix

Use the Diagnostics modal and each peer modal as evidence. A passing scenario
should show:

- lifecycle events for each connected peer
- offer catalog request and loaded counts
- active connection path transport, path kind, direction, status, connection id,
  and address
- Get or Subscribe events with peer/domain/offer/payload, bytes, sequence, and
  stream close reason

### 1. Sentinel to Browser

1. Start Sentinel A with the command above.
2. Start one browser peer.
3. Add `/tmp/auki-sentinel-a.json`.
4. Confirm Sentinel A appears in the peer rail.
5. Confirm the `sentinel-a-preview` offer appears.
6. Click `Get`.
7. Click `Subscribe`, verify frames update, then click `Stop`.

Pass criteria:

- Browser events include `Lifecycle handshake starting`,
  `Lifecycle authorized`, `Offer catalog loaded`, `Get snapshot received`,
  `Subscribe accepted`, `Subscribe first frame`, and `Subscribe stream closed`.
- Sentinel state shows one browser peer, one Get, one Subscribe, and no reset or
  starvation failure.

### 2. Browser A Publishes, Browser B Subscribes

1. Start Sentinel A.
2. Open Browser A in one browser profile.
3. Add Sentinel A to Browser A.
4. Click `Publish Preview` in Browser A.
5. Click `Copy Bootstrap` in Browser A.
6. Open Browser B in a separate browser profile.
7. Add Sentinel A to Browser B.
8. Add Browser A's copied bootstrap record to Browser B.
9. Confirm Browser A appears in Browser B's peer rail.
10. Confirm Browser A's generated preview offer appears in Browser B.
11. Click `Get` on Browser B.
12. Click `Subscribe` on Browser B, verify frames update, then click `Stop`.

Pass criteria:

- Browser B events show lifecycle and offer catalog activity for Browser A.
- Browser B `Get` and `Subscribe first frame` events report the same
  peer/domain/offer and monotonically increasing sequence values.
- Browser A peer modal shows an active connection path to Browser B while
  Browser B is subscribed.

### 3. WebRTC Versus Relay Path Evidence

Run scenario 2 and inspect Browser B's modal for Browser A.

WebRTC/WebRTC Direct evidence:

- `Transport` is `webrtc` or `webrtc-direct`.
- `Path` is `direct` or `direct via relay`.
- The active address contains `/webrtc` or `/webrtc-direct`.

Plain relay fallback evidence:

- The active address contains `/p2p-circuit`.
- `Path` is `relayed` or `direct via relay`.
- `Transport` is not `webrtc-direct`.

If both paths are available, use the peer modal `Use` action to select one
address at a time. Stop active streams before switching paths.

### 4. Symmetric Browser Streams

1. Complete scenario 2.
2. In Browser B, click `Publish Preview`.
3. Click `Copy Bootstrap` in Browser B.
4. Add Browser B's bootstrap record to Browser A.
5. Subscribe from Browser A to Browser B.
6. Subscribe from Browser B to Browser A.

Pass criteria:

- Both browsers show one local published offer and one remote subscribed offer.
- Each browser receives frames from the other without resetting the connection.
- Stopping either subscription only closes that subscription.

### 5. Multi-Sentinel And Multi-Browser

Start a second Sentinel with distinct identity and offer values:

```sh
cargo run -p auki-p2p-preview-sentinel -- \
  --peer-label sentinel-b \
  --seed-byte 9 \
  --domain-label sentinel-b-domain \
  --domain-nonce-byte 43 \
  --offer-id sentinel-b-preview \
  --bootstrap-json /tmp/auki-sentinel-b.json \
  --trace-p2p
```

Add both `/tmp/auki-sentinel-a.json` and `/tmp/auki-sentinel-b.json` to both
browser peers.

Pass criteria:

- Both Sentinel offers and browser-published offers appear independently.
- Subscribing to one offer does not block Get or Subscribe on another offer.
- Generated frame patterns are visually distinct for Sentinel A, Sentinel B,
  Browser A, and Browser B.

### 6. Relay Shutdown Observation

After Browser B is subscribed to Browser A:

1. Record Browser B's active connection path to Browser A.
2. Stop Sentinel A with `Ctrl-C`.
3. Watch whether the Browser A to Browser B stream continues.

Interpretation:

- If the stream continues and the active path is WebRTC/direct, relay was used
  for setup/signaling and the media/data path survived without it.
- If the stream stops or the active path is relayed, the stream still depends on
  the relay. That is acceptable for the fallback path but should be recorded.

## Data Path Checks

The preview bytes must flow through `auki-p2p-browser` protocol streams:

- Do not use Park, HTTP cache, or legacy `auki-network` for this matrix.
- Browser offers should be loaded through Offer Catalog.
- Snapshots should use Get.
- Live frames should use Subscribe.
- Diagnostics should show P2P trace rows for Get and Subscribe attempts.
