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
- [ ] Expose one high-level browser peer handle that hides frames, streams, and
      transport setup from app developers.

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
  publishPreview(source: PreviewSource, options: PreviewOfferOptions): Promise<PublicationHandle>
  stop(): Promise<void>
}
```

## Phase 3 - Sentinel Preview Profile

- [ ] Define the minimal offer profile for live Sentinel preview.
- [ ] Use `subscribe` access.
- [ ] Use JPEG payload bytes in `auki.spatial_message.v1` for the first demo.
- [ ] Reference Sensor, Clock, and Frame registry entries by id/hash.
- [ ] Add a native Subscribe provider adapter over Sentinel's existing preview
      latch.
- [ ] Keep the old Park polling endpoint alive as compatibility.

Candidate names:

- offer kind: `auki.sensor.rgb_camera.preview`
- payload type: `auki.camera.jpeg_frame.v1`

## Phase 4 - Browser Mesh Demo

- [ ] Add a demo app under the SDK examples tree.
- [ ] Start one native demo node that can run bootstrap, WebRTC Direct, relay,
      and optional Sentinel preview.
- [ ] Let browser tabs load bootstrap JSON for addresses only.
- [ ] Keep all protocol data on libp2p streams.
- [ ] Let each browser publish a generated preview stream.
- [ ] Show local peer id, connected peers, transport path, relay status,
      offers, and live preview tiles.
- [ ] Support multiple Sentinels/native nodes in one demo session.

Bootstrap JSON is allowed only for address/session discovery. It must not carry
preview frames, protocol messages, offer catalogs, or authority decisions.

## Test Work

Rust:

- [x] `cargo test -p auki-p2p`.
- [x] Test WebRTC Direct config and browser-dialable observed addresses.
- [x] Test relay server address emission.
- [ ] Test that node-to-node TCP/QUIC still works.

Browser:

- [x] Vitest frame encode/decode tests through `auki-protocol-wasm` against
      Rust vectors.
- [x] Browser identity derivation compatibility tests.
- [x] WASM-backed peer binding create/verify tests.
- [x] WASM-backed offer catalog request/response tests.
- [x] WASM-backed Subscribe accept/data/end tests.

End-to-end:

- [ ] Playwright smoke: one native node plus three browser pages.
- [ ] Verify all browsers appear in the roster.
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
