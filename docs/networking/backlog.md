# Peer-To-Peer Networking Backlog

Status: live implementation and demo matrix queue. This file is not part of
the protocol requirements.

Last updated: 2026-05-29.

Protocol requirements live in [`baseline.md`](baseline.md). Future protocol
extension sketches live in [`drafts.md`](drafts.md). This backlog tracks SDK,
example, and validation work only.

## Current State

The RFC-shaped networking path now has three layers:

- `auki-protocol` owns pure v1 protocol objects, JSON frames, signed peer and
  domain authority objects, lifecycle handshakes, offer catalogs, Get,
  Subscribe, spatial messages, status snapshots, and locked vectors.
- `auki-protocol-wasm` exposes the same Rust protocol helpers and validators to
  browser code so browser peers do not grow a second TypeScript protocol
  implementation.
- `auki-p2p` and `auki-p2p-browser` provide high-level native and browser peer
  handles over libp2p. Application code deals with peers, bootstrap records,
  offers, Get, Subscribe, and publication handles instead of raw protocol
  frames or stream ordering.

The demo path is now strong enough to show:

- Browser to node over direct WebSocket.
- Browser to node over WebRTC Direct.
- Browser to browser over plain Circuit Relay.
- Browser to browser over browser WebRTC established through relay/signaling.
- A native node acting as browser bootstrap, WebSocket relay server, and preview
  producer.
- Browser tabs publishing generated preview streams through the same generic
  offer/Get/Subscribe model as native.
- Manual transport switching with one active connection path retained per peer.
- Diagnostics for peers, offers, active paths, relay involvement, Get,
  Subscribe, frames, bytes, and recent failures.

Legacy `auki-network`, Park HTTP cache, and old cluster manager paths remain
untouched. They are not in the RFC preview data path.

## Covered Matrix

| Pair | Transport path | Status | Evidence |
| --- | --- | --- | --- |
| Browser <-> node | WebSocket direct | Working | Preview browser can connect to Sentinel WebSocket address, Get, and Subscribe. |
| Browser <-> node | WebRTC Direct | Working | Preview browser can switch to Sentinel WebRTC Direct and Get/Subscribe. |
| Browser <-> browser | Circuit Relay over native relay server | Working | Browser B can Get/Subscribe Browser A through Sentinel relay. |
| Browser <-> browser | Browser WebRTC through relay/signaling | Working | Browser B can switch to browser WebRTC path and Get/Subscribe Browser A. |
| Multiple browser/node offers | Mixed | Working manually | Browser UI can add multiple Sentinels and browser peers and render independent offer tiles. |

## Remaining Matrix

These are the real gaps before we can say the connectivity matrix is broadly
covered.

| Pair | Transport path | Status | Next proof |
| --- | --- | --- | --- |
| Node <-> node | TCP direct | Implemented in `auki-p2p`; needs demo/matrix smoke | Add a small native peer example or scripted smoke using Get/Subscribe. |
| Node <-> node | QUIC direct | Transport is in `auki-p2p`; needs demo/matrix smoke | Add the same native smoke over QUIC listen/dial addresses. |
| Node <-> node | WebSocket direct | Transport is in `auki-p2p`; needs demo/matrix smoke | Add a WebSocket-native variant or option. |
| Node <-> node | Circuit Relay | Not proven in demo matrix | Use one native relay node and two native peers. |
| Browser <-> node | Circuit Relay fallback | Not yet isolated | Prove browser can reach a native producer when direct WebSocket/WebRTC Direct are not used. |
| Browser <-> browser | Relay shutdown after WebRTC setup | Observation pending | Establish browser WebRTC, stop relay, record whether stream survives. |
| N browsers + N nodes | Mixed transports under load | Manual partial | Run two Sentinels plus two browsers with concurrent Get/Subscribe. |

## Next Work Queue

1. **Native node-to-node smoke/demo.**
   Build the smallest native example or test harness that starts two RFC
   `auki-p2p` nodes, publishes one preview-like byte offer on one node, and
   proves lifecycle, offer catalog, Get, and Subscribe over direct TCP.

2. **Add QUIC and WebSocket variants to the same native smoke.**
   Keep the protocol flow identical and change only listen/dial addresses.
   This gives us a clean node-to-node direct transport proof.

3. **Prove native relay between native peers.**
   Start native relay node C, native producer A, native consumer B. B reaches A
   through C, loads the catalog, Gets, and Subscribes.

4. **Prove browser-to-node relay fallback.**
   Configure the Sentinel/native producer so the browser uses a relayed target
   address rather than direct WebSocket or WebRTC Direct. Verify Get/Subscribe
   and UI path details.

5. **Run relay-shutdown observation for browser WebRTC.**
   After Browser B subscribes to Browser A over browser WebRTC, stop the native
   relay. Record whether the stream continues. This is evidence, not a protocol
   requirement.

6. **Consolidate manual matrix notes into repeatable smoke scripts only after
   the flow stops changing.**
   The current manual demo is useful. Do not over-automate until the remaining
   native matrix is stable.

## Demo Work

The current browser demo is good enough for live walkthroughs. Keep improving it
only where it helps explain the network:

- Keep the first screen simple: start peer, then add peers.
- Keep peer detail focused on active connection paths, dialable addresses,
  transport family, relay involvement, and status.
- Keep offer cards compact with local/remote state and Get/Subscribe actions.
- Keep diagnostics available but not always on screen.
- Preserve one active selected path per peer when users switch transports.

Future demo improvements:

- Add MacBook camera source to `examples/p2p-preview-sentinel`.
- Add browser camera publishing behind a user action.
- Add saved/importable matrix fixtures for two Sentinels and two browser peers.
- Add a compact "matrix pass" panel that shows which pair/transport proofs have
  been observed in the current session.

## SDK Ergonomics Contract

Native and browser APIs should keep the same mental model unless the platform
forces a difference.

App developers should think in these steps on both targets:

1. Create/start a peer.
2. Add bootstrap/connectivity records.
3. List peers and offers.
4. Get snapshots or Subscribe to streams.
5. Publish local offers from byte sources.
6. Stop subscriptions, withdraw publications, and stop the peer.

The SDK should hide protocol frames, stream muxers, libp2p protocol ids,
lifecycle request/response ordering, retries, relay cleanup, and transport
switching from application code.

Intentional platform differences:

- Browser identity persistence uses IndexedDB by default.
- Browser connectivity needs browser transports, relay reservation/signaling,
  secure-context constraints, and user permission prompts for camera capture.
- Native owns richer authority setup and local domain registration first.
- Browser examples may use manual JSON bootstrap until discovery exists.

## Production And Protocol Work Not Needed For The Demo

These remain parked until product requirements need them:

- Discovery record shape and data-type hints.
- Peer graph hints.
- Dynamic served-domain updates.
- Concrete clock-sync protocol.
- Production relay reservation policy and relay authorization grants.
- Subscribe reliability, replay, resume, chunking, and large-object transfer.
- Shared offer-kind profiles beyond the preview profile.
- Sensor, Clock, and Frame registry references for real Sentinel metadata.

## Definition Of Done For Matrix Coverage

- Browser <-> browser works over both relay fallback and browser WebRTC.
- Browser <-> node works over WebSocket direct, WebRTC Direct, and relay
  fallback.
- Node <-> node works over TCP, QUIC, WebSocket, and relay.
- At least two browser peers and two native nodes can coexist in one demo
  session without Get/Subscribe starvation or transport switching resets.
- Diagnostics clearly show active transport family, relay involvement, peer id,
  offer id, active streams, and last failure for each path.
- No preview bytes flow through Park, HTTP cache, or legacy `auki-network`.
