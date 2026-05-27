# Peer-To-Peer Networking Backlog

Status: implementation and follow-up work queue. This file is not part of the
protocol requirements.

Owner: TBD.

Last updated: 2026-05-27.

Related baseline:
[`baseline.md`](baseline.md).

Related production guardrails:
[`security-profile-v1.md`](security-profile-v1.md).

Related drafts:
[`drafts.md`](drafts.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track implementation work that follows the RFC-shaped networking baseline.

The protocol requirements live in `baseline.md`. Future extension sketches live
in `drafts.md`. This file is only the work queue.

## Current State

The first RFC-shaped SDK foundation exists in `auki-protocol` and `auki-p2p`.

`auki-protocol` owns the pure v1 protocol surface: JSON frames, signed peer and
domain authority objects, lifecycle handshakes, offer catalogs, Get,
Subscribe, spatial messages, status snapshots, and locked vectors.

`auki-protocol-wasm` exposes the same Rust protocol validators, constructors,
frame helpers, and failure-code mapping to browser code so the browser path
does not grow a second TypeScript protocol implementation.

`auki-p2p` owns the clean native libp2p runtime path: configured peer dialing,
lifecycle authorization, local domain and offer registration, offer loading,
Get and Subscribe consumers/providers, relationship status, and a full
two-peer native smoke flow. It does not reuse `NetworkRuntime`, Discovery,
Manager election, or legacy cluster membership semantics.

Discovery remains optional and non-authoritative. The next work is proving the
new path across browser and native peers without breaking the shipped
`auki-network` / `auki-domain` runtime.

`auki-p2p-browser` now provides the browser-side peer path: browser peer
identity, bootstrap parsing, js-libp2p transport setup, lifecycle handshakes,
remote offer-catalog loading, Subscribe consumption, generic local offer
publication, inbound offer-catalog serving, and inbound Subscribe serving
through `auki-protocol-wasm` validators. Preview publishing is a helper/profile
on top of the generic offer API.

`examples/p2p-preview-sentinel` now provides the first native demo slice: a
browser-reachable Sentinel peer that publishes the shared preview offer profile,
generates JPEG preview frames, and prints/writes address-only browser bootstrap
JSON plus compact P2P state.

## Networking Matrix

The SDK demo path should cover every peer-type pairing:

| Pair | Required | First transport target | Notes |
| --- | --- | --- | --- |
| Browser <-> browser | Yes | js-libp2p WebRTC | Circuit Relay v2/signaling is expected for setup. Data should be direct when WebRTC is established. |
| Browser <-> node | Yes | WebRTC Direct | Native Sentinel/operator node advertises browser-dialable multiaddr. |
| Node <-> node | Yes | Existing `auki-p2p` TCP/QUIC | Preserve the tested native path. |
| Multiple browsers + multiple nodes | Yes | Mixed | The demo should prove the matrix scales beyond a two-peer toy. |

Relay and signaling are connectivity infrastructure only. They must not replace
peer binding, lifecycle authority, domain authorization, offer policy, or
application-level access decisions.

## Next Vertical Slice

Build a browser + Sentinel mesh demo around RFC protocol frames.

Architecture target:

- Sentinel is a native `auki-p2p` producer.
- Browser tabs are Auki peers, not HTTP clients of Park.
- A native node can optionally run bootstrap/signaling/Circuit Relay v2.
- Browsers can connect to browsers, Sentinels, and other native nodes.
- Browsers run lifecycle, load offer catalogs, and open Subscribe streams.
- Sentinel sends preview JPEG frames as `auki.spatial_message.v1` data on a
  Subscribe stream.
- Browser tabs can publish generated preview streams so browser-to-browser
  networking is testable without camera permission.
- Both the native CLI and browser app expose simple diagnostic state so operators
  can see peers, transports, relay involvement, offers, active subscriptions,
  frame counts, and recent failures without reading debug logs.

Keep legacy behavior intact:

- Do not replace Park's existing `ClusterManager` / `auki-network` stream path
  yet.
- Do not require Discovery for the first local proof.
- Do not make relay or signaling authoritative.
- Do not route RFC preview frame bytes through Park's Rust backend or HTTP
  cache.

## Phase 1 - Native Browser Reachability

- [x] Add optional WebRTC Direct listener support to `auki-p2p` for
      browser-to-node dialing.
- [x] Add optional Circuit Relay v2 server support to `auki-p2p` over
      WebSocket.
- [x] Keep listen addresses, advertised addresses, relay addresses, and
      bootstrap addresses distinct in config and status.
- [x] Add status output that reports observed transport path and relay
      involvement.
- [x] Preserve current node-to-node TCP/QUIC behavior and tests.

Useful prior art:

- `auki-network` browser probe for native WebRTC Direct setup.
- `auki-domain-relay` for native Circuit Relay v2 server setup.
- `auki-network-browser-wasm` and `auki-domain-browser` for browser transport
  experiments.

Do not import legacy cluster membership, Manager election, or old stream
protocols from those crates.

## Phase 2 - Browser Peer Package

- [x] Add a clean RFC-first browser package, likely `crates/auki-p2p-browser`.
- [x] Use js-libp2p first.
- [x] Configure browser transports: WebRTC, WebRTC Direct, WebSocket, and
      Circuit Relay v2.
- [x] Persist browser peer identity in IndexedDB.
- [x] Match Rust peer identity derivation vectors.
- [x] Add `auki-protocol-wasm` so browser code can use Rust `auki-protocol`
      frame helpers, peer/domain authority constructors, lifecycle handshake,
      offer catalog, Get, Subscribe, spatial message, error object, and status
      validators.
- [x] Wire `auki-p2p-browser` to consume `auki-protocol-wasm` and retire the
      temporary TypeScript frame helper from the public protocol surface.
- [x] Validate browser package behavior against Rust `auki-protocol` vectors
      through the WASM adapter.
- [x] Expose one high-level browser peer handle that hides frames, streams, and
      transport setup from app developers.
  - [x] Load remote offer catalogs over RFC libp2p streams while returning
        app-facing `OfferSummary` objects.
  - [x] Subscribe over RFC libp2p streams while yielding app-facing spatial
        messages.
  - [x] Publish generic byte sources as local offers.
  - [x] Keep generated preview publishing as a helper/profile outside the core
        peer method surface.
  - [x] Serve local offer catalogs to inbound browser/native peers.
  - [x] Serve finite local byte streams over inbound Subscribe.
- [x] Align native `auki-p2p` with the browser producer shape through generic
      `PublishOfferInput`, `PublishedOfferHandle`, and finite byte-source
      Subscribe serving helpers.

Candidate browser API shape:

```ts
createAukiBrowserPeer(config): Promise<AukiBrowserPeer>

interface AukiBrowserPeer {
  peerId: string
  multiaddrs(): string[]
  dial(addr: string): Promise<void>
  connectBootstrap(addrs: string[]): Promise<void>
  listPeers(): PeerSummary[]
  listOffers(peerId?: string): Promise<OfferSummary[]>
  subscribe(request: SubscribeRequest): AsyncIterable<SpatialMessage>
  publishOffer(options: PublishOfferOptions): Promise<PublicationHandle>
  stop(): Promise<void>
}
```

## Phase 3 - Preview Offer Profile

- [x] Define one shared preview profile over the generic offer APIs, not inside
      core runtime logic.
- [x] Native helper wraps `AukiNode::publish_offer(...)` and
      `PublishOfferInput`.
- [x] Browser helper wraps `AukiBrowserPeer.publishOffer(...)`.
- [x] Use `subscribe` access.
- [x] Use JPEG payload bytes in `auki.spatial_message.v1` for the first demo.
- [x] Keep camera capture and generated-frame production outside SDK core.
- [ ] Reference Sensor, Clock, and Frame registry entries by id/hash when the
      profile needs real Sentinel metadata.
- [ ] Keep the old Park polling endpoint alive as compatibility.

Settled initial names:

- offer kind: `auki.sensor.rgb_camera.preview`
- payload type: `auki.camera.jpeg_frame.v1`
- payload descriptor: `encoding = binary`, `media_type = image/jpeg`,
  `schema_version = 1`

## Phase 4 - Examples Preview Demo

Build two standalone examples under `examples/` before touching real
Sentinel/Park integration.

- [x] Add `examples/p2p-preview-sentinel/`.
  - [x] Native Rust `auki-p2p` node.
  - [x] Publishes a preview offer through the Phase 3 helper.
  - [x] Prints/writes address-only browser bootstrap JSON.
  - [x] Enables WebRTC Direct and WebSocket relay/server development config.
  - [x] Supports `--source generated` first.
  - [ ] Adds `--source camera` later for MacBook camera JPEG capture.
  - [x] CLI prints and refreshes local peer id, listen/browser bootstrap
        addresses, relay role, connected peers, published offers, frames sent,
        and recent failure codes.
  - [ ] CLI reports active served subscriptions and per-transport path details
        once the browser subscriber exists.
- [ ] Add `examples/p2p-preview-browser/`.
  - [ ] Small web app using `auki-p2p-browser`.
  - [ ] Loads address-only bootstrap JSON.
  - [ ] Connects lifecycle, loads offers, subscribes, and renders JPEG frames.
  - [ ] Publishes its own generated preview stream.
  - [ ] Adds browser camera publishing later, behind a user action.
  - [ ] Shows local peer id, connected peers, transport path, relay status,
        offers, and live preview tiles.
  - [ ] Shows active subscriptions, frames received, last frame time, selected
        source, and recent connection/path failures.
- [ ] Support multiple Sentinels/native nodes in one demo session.
- [ ] Support browser-to-browser preview where the native node is only
      bootstrap/signaling/relay, not the media data path.

Diagnostic state is observability only. It must not become a new authority
source for peer admission, domain access, offer policy, or media routing.

Implementation order:

1. Native generated JPEG stream -> browser render.
2. Browser generated stream -> second browser render.
3. Multi-browser + multi-Sentinel roster.
4. MacBook camera source for the native Sentinel example.
5. Browser camera source for browser publishers.

Bootstrap JSON is allowed only for address/session discovery. It must not carry
preview frames, protocol messages, offer catalogs, or authority decisions.

## Test Work

Rust:

- [x] `cargo test -p auki-p2p`.
- [x] Test WebRTC Direct config and browser-dialable observed addresses.
- [x] Test relay server address emission.
- [x] Test native generic published-offer registration, withdrawal, and
      Subscribe byte streaming.
- [x] Test shared preview profile construction on native and browser helpers.
- [ ] Test that node-to-node TCP/QUIC still works.

Browser:

- [x] Vitest frame encode/decode tests through `auki-protocol-wasm` against
      Rust vectors.
- [x] Browser identity derivation compatibility tests.
- [x] WASM-backed peer binding create/verify tests.
- [x] WASM-backed offer catalog request/response tests.
- [x] WASM-backed Subscribe accept/data/end tests.
- [x] Browser peer lifecycle, offer-catalog, and Subscribe tests over
      in-memory protocol streams.
- [x] Browser Subscribe tests keep `maxMessageBytes` scoped to data messages,
      not Subscribe start/end control frames.
- [x] Browser peer producer tests for inbound offer-catalog and Subscribe
      streams.
- [x] Browser preview helper test matches the shared profile descriptor.

End-to-end:

- [ ] Playwright smoke: one generated native preview node plus one browser page.
- [ ] Playwright smoke: one native node plus three browser pages.
- [ ] Verify all browsers appear in the roster.
- [ ] Verify generated native preview renders in the browser.
- [ ] Verify the CLI and browser state panels expose peer, transport, relay,
      offer, subscription, and frame-count state.
- [ ] Verify browser A/B/C generated previews are visible cross-window.
- [ ] Verify Sentinel preview is visible in all browsers.
- [ ] Add a second Sentinel and verify it appears.
- [ ] Stop relay/signaling after browser-to-browser stream establishment and
      record whether existing streams continue.
- [ ] Confirm no Park backend or legacy HTTP cache is in the preview data path.

## Drafts To Pull Forward When Needed

Detailed draft text lives in [`drafts.md`](drafts.md).

Pull these forward only when product or implementation work needs them:

- Dynamic Served-Domain Updates.
- Discovery Record Shape.
- Discovery Data-Type Hints.
- Peer Graph Hints.
- Concrete Clock-Sync Protocol.
- Shared Offer-Kind Profiles.
- Production relay reservation and relay policy grants.
- Subscribe reliability, replay, resume, and large-object transfer.

## Browser Producer Design Checkpoint

Keep these boundaries as browser producer behavior grows:

- `protocol.ts` remains a thin WASM adapter; it must not grow independent RFC
  rules.
- `stream.ts` owns JSON-frame read/write glue over libp2p-style streams.
- `transport.ts` owns js-libp2p setup and should expose only small transport
  capabilities such as dialing and registering protocol handlers.
- `peer.ts` owns the high-level SDK handle, local offer registry, publication
  handles, and app-facing methods.
- `publication.ts` owns generic local offer/message construction for byte
  sources.
- `preview.ts` is only a generated-preview helper/profile over
  `publishOffer(...)`; camera capture, infinite/live source lifecycle, and
  reliable delivery remain later work.
