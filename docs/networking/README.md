# Networking RFCs

This directory contains the draft peer-to-peer cluster lifecycle RFCs.

Read in this order:

1. [`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md) is the normative
   draft spec.
2. [`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md) is the
   non-normative work queue.
3. [`glossary.md`](glossary.md) defines recurring terminology.

## Current Status

The current spec is a bootstrapping protocol for configured or private
peer-to-peer relationships. Discovery is optional rendezvous/presence
infrastructure and is not required for the currently specified peer-to-peer
path.

The authority and identity path is implementable now: peer bindings, domain
ids, domain declarations, domain delegations, authority-chain validation,
served-domain-set computation, peer authorization, handshake message shape,
offer objects, spatial envelopes, and status objects are specified.

End-to-end clean-room implementation is not complete yet. The next blocker is
post-handshake path binding and framing for Offer Catalog, Get, and Subscribe.
The backlog tracks that as `P0-1`.

## Parked Work

The following work is intentionally parked:

- concrete Discovery record shape;
- Discovery data-type hints;
- peer graph hints;
- dynamic served-domain updates;
- concrete NTP or clock-sync message flow;
- compatibility examples, expected results, and validation transcripts;
- later product-scope features such as replay, resume, chunking, large object
  transfer, map queries, and future offer kinds.

## Resume Here

Start with [`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md),
especially `P0-1: Post-Handshake Path Binding And Framing`.
