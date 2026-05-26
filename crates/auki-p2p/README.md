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
  dialing, local domain/offer registration, remote offer loading, SDK-facing
  Get/Subscribe, high-level peer events, relationship tracking, and in-process
  status snapshots without exposing protocol frames or stream internals.
- `AukiP2pNode` is a small libp2p node wrapper that keeps listen,
  advertised, and relay addresses separate; can dial explicit peer addresses;
  applies local per-peer connection caps for duplicate/simultaneous dials;
  surfaces connection events; projects local peer status; and exposes a raw
  stream control for protocol runtimes.
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
`AukiNode`'s remote offer cache.
