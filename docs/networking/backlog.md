# Peer-To-Peer Cluster Backlog

Status: implementation and follow-up work queue. This file is not part of the
protocol requirements.

Owner: TBD.

Last updated: 2026-05-22.

Related baseline:
[`baseline.md`](baseline.md).

Related drafts:
[`drafts.md`](drafts.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track the work that follows the draft implementable baseline.

The protocol requirements live in `baseline.md`. Future extension sketches live
in `drafts.md`. This file is only the work queue.

## Current State

The baseline text is ready to drive a first SDK implementation path for
configured/manual peer-to-peer connectivity.

Discovery remains optional and does not block the baseline path.

## Next Work

Start implementing a first SDK path from `baseline.md`.

Suggested first vertical slice:

1. Configured peer dial to a known peer id and multiaddr.
2. V1 JSON frame encode/decode.
3. Peer binding creation and verification.
4. Lifecycle handshake over `/auki/cluster-lifecycle/0.0.1`.
5. Authority-chain validation for zero or more declared domains.
6. Accepted served-domain set computation.
7. Ready/degraded peer relationship state.
8. Minimal status surface for local peer, remote peer, served domains, and last
   failures.

Suggested second vertical slice:

1. Offer-catalog request/response over `/auki/offer-catalog/0.0.1`.
2. Offer usability evaluation.
3. Get request/response over `/auki/get/0.0.1`.
4. Subscribe request/accept/reject/data/end over `/auki/subscribe/0.0.1`.
5. Spatial message envelope validation.
6. Deterministic failure-code reporting for common bad inputs.

## Interop/Test Work

Create these from the first SDK implementation, so the examples match running
code:

- compatibility examples;
- expected results;
- validation transcripts;
- fixture JSON;
- signed-object test vectors;
- frame encoding examples;
- machine-readable schema aids where they are useful.

Suggested coverage:

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

## Drafts To Pull Forward When Needed

Detailed draft text lives in [`drafts.md`](drafts.md).

Pull these forward only when product or implementation work needs them:

- Dynamic Served-Domain Updates
- Discovery Record Shape
- Discovery Data-Type Hints
- Peer Graph Hints
- Concrete Clock-Sync Protocol
- Shared Offer-Kind Profiles

These do not block the baseline configured/manual peer-to-peer path.

## Later / Product-Scope Work

Keep these parked unless product scope pulls them forward:

- DHT-style discovery.
- Relay milestone expansion beyond `RFC-0017` and `RFC-0018`.
- Shared offer-kind profiles and payload schemas.
- Chunking, replay, resume, reliable history, and large object transfer.
- Implementation migration notes, if needed after the baseline implementation
  exists.
