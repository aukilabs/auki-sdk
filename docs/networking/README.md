# Networking RFCs

This directory contains the draft peer-to-peer cluster lifecycle RFCs.

Read in this order:

1. [`baseline.md`](baseline.md) is the draft implementable baseline.
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

## Peer Role Quick Reference

This table is a reading guide. The protocol requirements live in
[`baseline.md`](baseline.md).

| Capability | Consumer-only peer | Producer peer | Producer + consumer |
| --- | --- | --- | --- |
| libp2p peer identity | Required | Required | Required |
| Peer binding | Required | Required | Required |
| Declared domains | May be empty | Required for served domains | Required for served domains |
| Offer-catalog fetch path | Omitted when exposing no offers | Required when exposing offers | Required when exposing offers |
| Fetch remote catalog | Allowed | Allowed | Expected when consuming |
| Get responder | Only when advertised | Required for offers advertising `get` | Required for offers advertising `get` |
| Subscribe responder | Only when advertised | Required for offers advertising `subscribe` | Required for offers advertising `subscribe` |

A peer does not need to own or declare a domain merely to connect,
authenticate, authorize a peer relationship, fetch remote offers, or consume
remote data.

## Current Status

The baseline is a bootstrapping protocol for configured or private peer-to-peer
relationships. Discovery is optional rendezvous/presence infrastructure and is
not required for the baseline peer-to-peer path.

The authority and identity path is implementable now: peer bindings, domain
ids, domain declarations, domain delegations, authority-chain validation,
served-domain-set computation, peer authorization, handshake stream behavior,
handshake message shape, offer objects, catalog filters, registry-reference
hashes, payload-type matching, message envelopes, and status objects are
specified.

End-to-end clean-room implementation still needs deterministic failure-mapping
cleanup before broad compatibility work.

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

Start with [`backlog.md`](backlog.md), especially the remaining P1 cleanup
items.
