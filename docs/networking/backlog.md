# Peer-To-Peer Cluster Backlog

Status: working backlog. This file is not part of the protocol requirements.

Owner: TBD.

Last updated: 2026-05-22.

Related baseline:
[`baseline.md`](baseline.md).

Related drafts:
[`drafts.md`](drafts.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track the remaining work needed before a clean-room SDK implementer can build
the baseline protocol from `baseline.md` alone.

The protocol requirements live in `baseline.md`. Future extension sketches live
in `drafts.md`. This file is only the work queue.

## Current State

Implementable now:

- authority and identity objects;
- domain id derivation;
- domain declaration and delegation validation;
- authority-chain validation;
- served-domain-set computation;
- peer authorization model;
- lifecycle handshake stream behavior and message shape;
- Offer Catalog, Get, Subscribe, and message-envelope object shapes;
- registry-reference hash format;
- status object shapes;
- time and clock semantics.

Baseline path:

- configured/manual peer-to-peer connectivity;
- Discovery is optional and does not block the baseline path.

## Work Order

1. `P1-1`: Tighten deterministic failure mapping where interop needs exact
   outcomes.
2. `P1-2`: Tighten payload-type matching.
3. Start compatibility examples, expected results, and validation transcripts.

## P1

### P1-1: Deterministic Failure Mapping

Owner sections:

- `RFC-0010: Failure Code Registry`
- `RFC-0019: Peer Handshake`
- `RFC-0024: Offer Catalog`
- `RFC-0027: Spatial Message Envelope`
- `RFC-0028: Get And Subscribe Common Path Rules`
- `RFC-0029: Get`
- `RFC-0030: Subscribe`
- `RFC-0034: Status And Observability API`

Patch:

- Keep the failure precedence rule in `RFC-0010` and authority-chain precedence
  in `RFC-0009` as the baseline ordering model.
- Decide which malformed, unsupported, unauthorized, oversized, and stale cases
  need exact failure codes.
- Upgrade only those mappings that must be deterministic for interop.
- Keep advisory diagnostics advisory.

Done when:

- Common bad inputs produce predictable failure codes without forcing every
  diagnostic path to become a protocol requirement.

### P1-2: Payload-Type Matching

Owner sections:

- `RFC-0024: Offer Catalog`
- `RFC-0027: Spatial Message Envelope`
- `RFC-0028: Get And Subscribe Common Path Rules`
- `RFC-0029: Get`
- `RFC-0030: Subscribe`

Patch:

- Define whether `accepted_payload_types` matches `payload.type` by exact string
  equality.
- Define how responders choose a payload type when more than one is possible.
- Clarify how Get response payloads and Subscribe data-message payloads relate
  to offer and Subscribe accept payload descriptors.

Done when:

- A responder can decide whether it can satisfy `accepted_payload_types`.
- A receiver can reject an unexpected payload family deterministically.

## Drafts Outside The Baseline

Detailed draft text lives in [`drafts.md`](drafts.md).

Tracked drafts:

- Dynamic Served-Domain Updates
- Discovery Record Shape
- Discovery Data-Type Hints
- Peer Graph Hints
- Concrete Clock-Sync Protocol
- Shared Offer-Kind Profiles

These do not block the baseline configured/manual peer-to-peer path.

## Interop/Test Work

Do this after P0 and P1 patches.

Create compatibility examples and expected results for:

- JSON frame encoding and JSON wire conventions;
- signed-object canonicalization;
- peer bindings;
- domain id derivation;
- domain declarations;
- domain delegations;
- authority-chain validation;
- lifecycle handshake;
- Offer Catalog;
- Get;
- Subscribe;
- spatial message envelopes;
- error objects;
- status snapshots;
- time fields.

Create validation transcripts for:

- configured peer connects to configured peer;
- peer connects without Discovery;
- peer connects after optional Discovery lookup;
- producer declares no served domains;
- producer declares one accepted served domain;
- producer declares multiple domains with mixed accept/reject results;
- consumer fetches an offer catalog;
- consumer performs Get for a usable offer;
- consumer starts and ends Subscribe for a usable offer;
- malformed or unauthorized inputs produce stable failure codes.

## Later / Product-Scope Work

Keep these parked unless product scope pulls them forward:

- DHT-style discovery.
- Relay milestone expansion beyond `RFC-0017` and `RFC-0018`.
- Shared offer-kind profiles and payload schemas.
- Chunking, replay, resume, reliable history, and large object transfer.
- Implementation migration notes, if needed after the baseline stabilizes.
