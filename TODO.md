# Auki P2P Browser-To-Browser TODO

Status: planning checklist for the next connectivity-matrix slice.

Last updated: 2026-05-29.

This TODO tracks the work needed to prove browser-to-browser networking through
the RFC-shaped `auki-protocol` / `auki-p2p` path. The native Sentinel/node may
act as bootstrap, signaling, and Circuit Relay infrastructure, but browser
preview bytes must flow over Auki protocol streams between browser peers, not
through Park, HTTP cache, or legacy network runtime paths.

Protocol requirements remain in `docs/networking/`.

Useful libp2p references:

- Browser-to-browser WebRTC with js-libp2p:
  https://libp2p.io/docs/webrtc-browser-connectivity/
- Browser node connectivity:
  https://libp2p.io/docs/browser-connectivity/
- WebRTC transport overview:
  https://libp2p.io/docs/webrtc/

## Current Findings

- `auki-p2p-browser` can already publish local offers and serve inbound Offer
  Catalog, Get, and Subscribe streams.
- Native `auki-p2p` already has the developer shape we should mirror:
  `AukiNode` hides frames/streams, `PublishOfferInput` publishes a generic byte
  source, `PublishedOfferHandle` identifies the offer, and
  `LatestPublishedByteSource` gives live producers one source of truth for Get
  and Subscribe.
- The Sentinel example already uses that source-of-truth model:
  `publish_preview_offer_with_latest_source(...)` registers one preview offer,
  Get returns `source.latest()`, and every Subscribe receives frames from the
  same producer stream.
- Native requires offers to be scoped to registered local domains. Browser
  publication will follow the same model: browser-published preview offers use
  a browser-local demo domain declared through lifecycle, not a silently reused
  Sentinel domain.
- Browser outbound lifecycle exists, but browser inbound lifecycle is missing.
  Another browser can dial us, but cannot yet complete the lifecycle handshake
  against us.
- Browser transport currently supports Circuit Relay and WebRTC pieces, but the
  browser-to-browser dialable address shape needs to be explicit. The expected
  target is a browser-dialable address through the relay, e.g.
  `/p2p-circuit/webrtc/p2p/<browser-peer-id>`.
- WebRTC Direct remains the right first target for browser-to-native Sentinel.
  Browser-to-browser should target WebRTC via relay signaling.
- The preview browser demo can receive Sentinel offers. It does not yet publish
  its own generated preview stream or expose a clean local browser bootstrap
  record for another browser tab.

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
lifecycle request/response ordering, retry details, and transport cleanup from
application code.

Expected native/browser parity:

| Concept | Native `auki-p2p` | Browser `auki-p2p-browser` target |
| --- | --- | --- |
| Peer handle | `AukiNode` / `AukiServeRuntime` | `AukiBrowserPeer` |
| Peer creation | `AukiNodeBuilder` | `createAukiBrowserPeer(...)` |
| Bootstrap export | `browser_bootstrap_record()` | `localBootstrapRecord()` |
| Connect/bootstrap | configured peers / explicit dial | `connectBootstrap(...)` |
| Peer state | status/relationships | `listPeers()` plus connection paths |
| Offer discovery | `load_remote_offers(...)` | `listOffers(peerId?)` |
| Snapshot | `get(GetInput)` | `get(GetRequest)` / preview helper |
| Stream | `subscribe(SubscribeInput)` | `openSubscription(...)` / `subscribe(...)` |
| Publish generic bytes | `publish_offer(PublishOfferInput)` | `publishOffer(PublishOfferOptions)` |
| Publish preview profile | `publish_preview_offer...` | `publishPreviewOffer(...)` |
| Live source of truth | `LatestPublishedByteSource` | browser equivalent |
| Publication handle | `PublishedOfferHandle` | `PublicationHandle` with `domainId`, `offerId`, `stop()` |
| Withdraw | `unpublish_offer(&handle)` | `handle.stop()` |
| Backpressure | `AukiSubscriptionBackpressurePolicy` | browser equivalent with `LatestOnly`, `Bounded`, `CloseOnFull` |

Allowed differences:

- Browser identity persistence uses IndexedDB by default.
- Browser connectivity needs browser transports, relay reservation/signaling,
  secure-context constraints, and user permission prompts for camera capture.
- Native owns local domain registration and richer authority setup first.
- Browser examples may use manual JSON bootstrap before discovery exists.

Do not let these platform differences leak into application-level Get,
Subscribe, or Publish workflows.

## Frozen Decisions

These decisions are locked for the browser-to-browser slice.

| Topic | Decision |
| --- | --- |
| Browser offer authority | Browser-published preview offers belong to a browser-local demo domain created with `auki-protocol-wasm`, declared in lifecycle handshakes, and used for the generated preview offer. Do not reuse a Sentinel domain unless we implement delegation. |
| Browser domain API | Expose a small SDK-facing local-domain helper. Avoid exposing raw protocol plumbing as the app-facing browser API. |
| Lifecycle ordering | Mirror native: write the local handshake first, then read and validate the remote handshake. |
| Lifecycle policy | For the local demo, accept any peer whose handshake has a valid peer binding matching the authenticated libp2p peer id. Add app-policy hooks later. |
| Browser bootstrap schema | Keep the existing bootstrap record type for native nodes, relay nodes, and browser peers. Keep address roles strict: browser target addresses go in `relayAddresses` / `bootstrapAddresses`; relay server addresses are reservation/signaling hints. |
| Browser dialable timing | Export a local browser bootstrap record only after the browser has a real dialable/reserved browser-target address. Before that, report "not dialable yet". |
| Browser-to-browser paths | QA both WebRTC and plain relayed browser-to-browser paths. Prefer WebRTC first, but relay is a required fallback path to prove. |
| Relay after direct setup | Treat relay shutdown after WebRTC setup as an observation test. Record whether the active path is direct WebRTC or still relay-dependent. |
| Publication handle ergonomics | Browser `PublicationHandle` exposes readonly `domainId`, `offerId`, and `stop()`. Do not add `peer.unpublishOffer(...)` in this slice. |
| Byte source compatibility | Support existing `Uint8Array` source chunks and frame metadata chunks shaped like `{ bytes, sequence?, generatedAt? }`. |
| Browser backpressure | Implement browser equivalents for all native policies: `LatestOnly`, `Bounded`, and `CloseOnFull`. |
| Local self-actions | Show local offers in the demo, but disable Get and Subscribe for the same tab. |
| Smoke automation | Use unit tests plus deterministic manual matrix steps. Do not add a local smoke script in this slice. |

## Phase 1 - SDK Foundation

- [x] Add browser inbound lifecycle serving in `auki-p2p-browser`.
- [x] Register and unregister the lifecycle protocol with the other inbound
      protocol handlers.
- [x] Mirror native lifecycle ordering: write the local handshake first, then
      read the remote handshake.
- [x] Parse inbound peer handshakes with `auki-protocol-wasm`.
- [x] Validate remote peer authority against the authenticated libp2p peer id.
- [x] Track lifecycle-authorized browser peers.
- [x] Add tests for successful inbound browser lifecycle.
- [x] Add tests for rejecting lifecycle when the authenticated peer id does not
      match the signed peer binding.
- [x] Add tests for lifecycle handler cleanup on peer stop.
- [x] Implement browser local-domain state for browser-published offers.
- [x] Expose a small SDK-facing browser local-domain helper backed by
      `auki-protocol-wasm`.
- [x] Include browser-local declared domains in lifecycle handshakes when
      browser-published offers are meant to be authoritative.
- [ ] Review browser public method names against native and document intentional
      differences.
- [x] Expose `domainId`, `offerId`, and `stop()` on browser `PublicationHandle`.
- [x] Keep `handle.stop()` as the browser withdraw/unpublish API for this slice.

## Phase 2 - Browser Dialability

- [ ] Make browser listen-address configuration explicit for browser-to-browser
      WebRTC.
- [ ] Ensure browser peers can reserve/listen via Circuit Relay after connecting
      to a native relay/bootstrap node.
- [ ] Add an SDK method for exporting the local browser bootstrap record,
      probably `localBootstrapRecord()`.
- [ ] Ensure local browser bootstrap records advertise browser-target addresses,
      not only relay-server addresses.
- [ ] Keep relay server addresses separate from browser peer addresses. A remote
      browser should dial the browser peer through the relay, not accidentally
      dial the relay as if it were the browser.
- [ ] Add tests for local browser bootstrap record shape.
- [ ] Add tests that preferred dial addresses prioritize actual browser target
      addresses correctly.
- [ ] Add tests or manual QA steps for both WebRTC browser target addresses and
      plain relayed browser target addresses.

## Phase 3 - Browser Publishing

- [ ] Add demo support for publishing a generated browser preview stream.
- [ ] Keep generated preview as example/demo source code, not SDK core logic.
- [ ] Use the existing generic `publishOffer(...)` and preview helper profile.
- [ ] Add a browser equivalent of native `PublishedByteFrame`:
      `bytes`, optional `sequence`, optional `generatedAt`.
- [ ] Add a browser equivalent of native `LatestPublishedByteSource` with:
      `publish(frame)`, `latest()`, `close()`, and per-subscriber streams.
- [ ] Use one shared/latest generated source so Get and Subscribe observe the
      same logical stream.
- [ ] Make browser Get return the latest frame from that shared source, matching
      `publish_preview_offer_with_latest_source(...)` on native.
- [ ] Make every browser Subscribe consume from the same producer source, not an
      independent generator instance per subscriber.
- [ ] Preserve producer sequence numbers and generated timestamps in browser
      spatial messages when the source provides them.
- [ ] Add browser backpressure policy support for live published streams:
      `LatestOnly`, `Bounded`, and `CloseOnFull`.
- [ ] Use `LatestOnly` as the browser generated-preview default to match native
      preview behavior.
- [ ] Disable local Get/Subscribe buttons for offers published by the same tab,
      unless we explicitly add local loopback later.
- [ ] Add tests that browser A published offers are visible through browser B's
      Offer Catalog request.
- [ ] Add tests that browser B can Get from browser A.
- [ ] Add tests that browser B can Subscribe and Stop against browser A.
- [ ] Add tests that Get while Subscribe is active does not reset or starve
      streams.
- [ ] Add tests that two browser subscribers can consume the same published
      source concurrently.
- [ ] Add tests that Get and Subscribe report the same latest source sequence
      instead of diverging generator offsets.
- [ ] Add tests that closing the shared source ends active browser Subscribe
      streams cleanly.

## Phase 4 - Demo UX

- [ ] Keep the first screen simple: Start Peer.
- [ ] After start, show the local browser peer and an Add Peer action.
- [ ] After connecting to a Sentinel/bootstrap node, expose a Copy Local Browser
      Bootstrap action.
- [ ] Add a Publish Generated Preview action.
- [ ] Show browser-published offers in the same offer grid as Sentinel offers.
- [ ] Mark local offers clearly and disable remote-only actions on them.
- [ ] Show active connection path details in the peer modal:
      transport, relay involvement, direct/relayed, connection id, and address.
- [ ] Show enough event detail to troubleshoot browser-to-browser dialing:
      lifecycle, offer catalog, Get, Subscribe, stream close, and transport
      switch events.

## Phase 5 - Manual Matrix Smokes

- [ ] One Sentinel relay/bootstrap plus one browser receiver still works.
- [ ] One Sentinel relay/bootstrap plus two browsers: browser A publishes,
      browser B subscribes over WebRTC.
- [ ] One Sentinel relay/bootstrap plus two browsers: browser A publishes,
      browser B subscribes over plain relay fallback.
- [ ] Browser A subscribes to browser B while browser B subscribes to browser A.
- [ ] Both browser tabs subscribe to the Sentinel preview while one browser also
      publishes its own preview.
- [ ] Two Sentinel nodes plus two browser tabs: all offers appear and can be
      subscribed independently.
- [ ] Stop browser A publishing and verify browser B sees stream end cleanly.
- [ ] Stop browser B subscription and verify browser A releases the stream
      cleanly.
- [ ] With browser A publishing, verify Browser B Get and Subscribe observe the
      same monotonically increasing sequence.
- [ ] With browser A publishing, verify two subscribers receive the same source
      sequence family without opening independent producers.
- [ ] Stop relay/signaling after browser-to-browser WebRTC establishment and
      record whether existing streams continue.
- [ ] Confirm no Park backend, HTTP cache, or legacy `auki-network` runtime is in
      the preview data path.

## Definition Of Done

- Two browser tabs can discover each other through a native relay/bootstrap
  path.
- Browser A can publish a generated preview offer.
- Browser B can load Browser A's offer catalog.
- Browser B can Get a snapshot from Browser A.
- Browser B can Subscribe to Browser A and receive frames.
- Stop/cancel paths work reliably.
- Browser-to-browser activity uses `auki-protocol` frames and
  `auki-p2p-browser` streams.
- Diagnostics clearly show which peers, offers, transports, and relay paths are
  active.
