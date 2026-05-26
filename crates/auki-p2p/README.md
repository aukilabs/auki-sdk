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
- `AukiP2pNode` is a small libp2p node wrapper that can listen, dial explicit
  peer addresses, surface connection events, and expose a raw stream control for
  the lifecycle protocol.

The first runtime test proves two deterministic local peers can connect over an
OS-assigned loopback TCP port and observe each other's authenticated `PeerId`.
