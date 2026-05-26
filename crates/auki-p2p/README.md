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
