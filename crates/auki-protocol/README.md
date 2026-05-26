# auki-protocol

Pure protocol crate for the RFC-first Auki networking path.

This crate owns deterministic protocol behavior: v1 framing, signed authority
objects, handshakes, offers, Get, Subscribe, status objects, validation helpers,
and conformance vectors. It deliberately does not own a libp2p swarm, tokio
runtime, Discovery client, or app-specific lifecycle facade.

**Status:** Initial scaffold.

## Public surface

- `v1::frame` — unsigned LEB128 length prefixes plus v1 JSON frame encode/decode.
- `v1::error` — stable v1 failure-code constants.

## Depends on

Nothing in the workspace yet. Future slices may depend on:

- [`auki-identity`](../auki-identity) for wallet signatures and verification;
- [`auki-jcs`](../auki-jcs) for RFC 8785 canonical JSON;
- `libp2p-identity` for peer-id parsing and formatting.
