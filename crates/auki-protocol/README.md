# auki-protocol

Pure protocol crate for the RFC-first Auki networking path.

This crate owns deterministic protocol behavior: v1 framing, signed authority
objects, handshakes, offers, Get, Subscribe, status objects, validation helpers,
and conformance vectors. It deliberately does not own a libp2p swarm, tokio
runtime, Discovery client, or app-specific lifecycle facade.

**Status:** Initial scaffold.

## Public surface

- `v1::authority` — authority-chain validation for peer bindings, declarations, and delegations.
- `v1::base64url` — canonical base64url-without-padding encode/decode helpers.
- `v1::domain` — v1 domain id, declaration, and delegation helpers.
- `v1::frame` — unsigned LEB128 length prefixes plus v1 JSON frame encode/decode.
- `v1::get` — v1 Get request and response helpers.
- `v1::handshake` — v1 peer handshake parsing and authority validation entrypoint.
- `v1::identity` — wallet-signed peer binding creation and verification.
- `v1::message` — v1 spatial message, payload, and error-object helpers.
- `v1::offer` — v1 offer-catalog, offer, payload, and registry-reference helpers.
- `v1::status` — v1 status snapshot and diagnostic status helpers.
- `v1::subscribe` — v1 Subscribe request, start result, data, and end helpers.
- `v1::json` — strict JSON object parsing with duplicate member rejection.
- `v1::error` — stable v1 failure-code constants.
- `vectors/v1_json_frames.json` — locked v1 JSON frame examples for cross-language conformance.
- `vectors/v1_signed_objects.json` — locked signed-object examples and negative vectors.
- `vectors/v1_handshakes.json` — locked handshake examples and authority-validation vectors.
- `vectors/v1_offer_catalogs.json` — locked offer-catalog request, response, and offer vectors.
- `vectors/v1_get.json` — locked Get request and response vectors.
- `vectors/v1_subscribe.json` — locked Subscribe request, start, data, and end vectors.

## Depends on

The current slice uses:

- [`auki-identity`](../auki-identity) for wallet signatures and verification;
- [`auki-jcs`](../auki-jcs) for RFC 8785 canonical JSON;
- `libp2p-identity` for peer-id parsing and formatting.
