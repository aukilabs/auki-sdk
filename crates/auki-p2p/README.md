# auki-p2p

`auki-p2p` is the clean libp2p runtime for the RFC-first Auki networking path.

This crate owns runtime concerns:

- configured peer dialing;
- lifecycle streams;
- peer relationship state;
- accepted served-domain tracking;
- offer loading;
- Get and Subscribe stream orchestration;
- SDK-facing APIs that hide protocol frames and validation order.

It is intentionally built on `auki-protocol` and must not wrap or depend on the
legacy `auki-network` runtime.

The crate uses libp2p concepts at its boundary (`PeerId`, `Multiaddr`, protocol
ids, connection roles, and stream direction), while keeping validation, policy,
limits, and state reducers testable without a running swarm.

Current public surface:

- `LocalPeerIdentity` derives deterministic libp2p peer keys from the wallet
  and maintains the wallet-signed `PeerBinding`.
- `AukiP2pConfig` captures RFC-shaped runtime limits and policy knobs.
- `AukiNode` is the SDK-facing runtime handle for configured peer management,
  dialing, local domain/offer registration, high-level lifecycle
  authorization and serving, remote offer loading and catalog serving,
  SDK-facing Get/Subscribe consumers and providers, high-level peer events,
  relationship tracking, and in-process status snapshots without exposing
  protocol frames or stream internals.
- `AukiP2pNode` is a small libp2p node wrapper that keeps listen,
  advertised, and relay addresses separate; can dial explicit peer addresses;
  applies local per-peer connection caps for duplicate/simultaneous dials;
  surfaces connection events; projects local peer status; and exposes a raw
  stream control for protocol runtimes.
- The optional `browser-webrtc-direct` Cargo feature adds native WebRTC Direct
  transport support for browser-to-node dialing. Use
  `AukiP2pNodeConfig::loopback_webrtc_direct_development()` for the local
  browser MVP and publish `observed_dialable_listen_addresses()` to browser
  peers after the listener emits an address.
- `lifecycle` helpers accept/open `/auki/cluster-lifecycle/0.0.1` streams and
  exchange the first peer-handshake frame using the configured frame limit,
  with optional strict helpers for duplicate lifecycle streams and extra
  lifecycle data.
- `handshake_policy` validates decoded remote handshakes without swarm access:
  cheap limits, peer-binding freshness, peer admission, authority-chain checks,
  local domain policy, required authorization material, and handshake-time
  authority deadlines.
- `relationship` tracks per-peer lifecycle state, accepted and rejected
  domains, bounded failure history, path summaries, and projects pure state into
  v1 status snapshots.
- `offer_loading` fetches one catalog response through an internal client
  boundary, enforces runtime limits, evaluates offer usability, and updates the
  relationship without exposing catalog frames to SDK callers.
- `offer_catalog_streams` wires offer loading to `/auki/offer-catalog/0.0.1`
  libp2p streams and includes a small serving helper for local catalog
  responses.
- `get_serving` accepts inbound `/auki/get/0.0.1` streams, decodes one Get
  request, writes one Get response, and lets `AukiNode` serve local offers
  through registered application providers without exposing raw frames.
- `subscribe_serving` accepts inbound `/auki/subscribe/0.0.1` streams, decodes
  one Subscribe request, writes accept/reject/data/end frames, and lets
  `AukiNode` serve local subscriptions through registered application
  providers without exposing raw frames.
- `paths` defines high-level Get and Subscribe orchestration over loaded offers:
  request shaping, response validation, data-message validation, path status,
  and sequence-gap diagnostics before transport wiring.
- `path_streams` wires that orchestration to `/auki/get/0.0.1` and
  `/auki/subscribe/0.0.1` libp2p streams, opening one stream per logical path
  and applying frame limits before handing frames to the pure validators.

The first runtime test proves two deterministic local peers can connect over an
OS-assigned loopback TCP port and observe each other's authenticated `PeerId`.
The lifecycle stream test then proves the same peers can exchange RFC
`PeerHandshake` frames both ways without pulling in offers, Get, Subscribe,
Discovery, or app adapters.
Path stream tests now cover one loopback Get and one loopback Subscribe start
followed by a data frame.
Offer-catalog stream tests now cover one loopback catalog load into
`AukiNode`'s remote offer cache after high-level lifecycle authorization.
Get serving tests now cover a registered local provider responding to a remote
`AukiNode::get(...)` over the RFC Get stream.
Subscribe serving tests now cover a registered local provider accepting a remote
`AukiNode::subscribe(...)`, sending a data message, and ending the stream.
The full `AukiNode` smoke test now covers configured dial, lifecycle
authorization, offer loading, Get, Subscribe data, and relationship/status
assertions in one two-peer flow.
With `--features browser-webrtc-direct`, the node tests also verify a loopback
WebRTC Direct listener emits a browser-dialable multiaddr.
