# Networking RFCs

This directory contains the draft peer-to-peer cluster lifecycle RFCs.

Read in this order:

1. [`baseline.md`](baseline.md) is the first implementable protocol baseline.
2. [`drafts.md`](drafts.md) parks future extension drafts.
3. [`backlog.md`](backlog.md) is the working task list. It is not part of the
   protocol requirements.
4. [`glossary.md`](glossary.md) defines recurring terminology.

## Baseline Compliance Profile

| Area | Baseline status |
| --- | --- |
| Peer binding | Required |
| Domain declaration | Required for serving |
| Domain delegation | Required when serving for another wallet |
| Configured/manual peers | Required |
| Discovery | Optional; not required for baseline interop |
| Discovery record shape | Excluded from first implementable version |
| Discovery data-type hints | Excluded from first implementable version |
| Dynamic served-domain updates | Excluded; reconnect or fresh handshake required |
| Peer graph hints | Excluded |
| Clock sync protocol | Excluded; time and clock semantics still apply |
| Offer catalog | Required for producers |
| Get | Required only if advertised |
| Subscribe | Required only if advertised |
| Minimum spatial offer kind | Not required in baseline |
| Offer kind semantics | Application, deployment profile, or later RFC defined |
| Shared offer-kind profiles | Excluded from first implementable version |

## Current Status

The baseline is a bootstrapping protocol for configured or private peer-to-peer
relationships. Discovery is optional rendezvous/presence infrastructure and is
not required for the baseline peer-to-peer path.

The authority and identity path is implementable now: peer bindings, domain
ids, domain declarations, domain delegations, authority-chain validation,
served-domain-set computation, peer authorization, handshake message shape,
offer objects, message envelopes, and status objects are specified.

End-to-end clean-room implementation is not complete yet. The next blocker is
post-handshake path binding and framing for Offer Catalog, Get, and Subscribe.
The backlog tracks that as `P0-1`.

## Drafts And Parked Work

The following work is intentionally outside the first implementable baseline:

- concrete Discovery record shape;
- Discovery data-type hints;
- peer graph hints;
- dynamic served-domain updates;
- concrete NTP or clock-sync message flow;
- compatibility examples, expected results, and validation transcripts;
- later product-scope features such as replay, resume, chunking, large object
  transfer, and shared offer-kind profiles.

## Resume Here

Start with [`backlog.md`](backlog.md), especially
`P0-1: Post-Handshake Path Binding And Framing`.
