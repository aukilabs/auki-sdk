# Networking RFCs

This directory contains the draft peer-to-peer cluster lifecycle RFCs and
networking extension drafts.

Read in this order:

1. [`baseline.md`](baseline.md) is the draft implementable baseline.
2. [`security-profile-v1.md`](security-profile-v1.md) is the short production
   guardrail profile for baseline deployments.
3. [`decentralized-peer-discovery.md`](decentralized-peer-discovery.md) defines
   the draft candidate pipeline for decentralized peer discovery.
4. [`drafts.md`](drafts.md) parks future extension drafts.
5. [`backlog.md`](backlog.md) is the working task list. It is not part of the
   protocol requirements.
6. [`glossary.md`](glossary.md) defines recurring terminology.

## Baseline Compliance Profile

| Area | Baseline status |
| --- | --- |
| Peer binding | Required |
| Domain declaration | Required for serving |
| Domain delegation | Required when serving for another wallet |
| Configured/manual peers | Required |
| Discovery | Optional; not required for baseline interop |
| Discovery candidate shape | Excluded from baseline; draft extension exists |
| Discovery data-type hints | Excluded from baseline; draft extension exists |
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

The baseline is not a complete production security profile by itself. Production
or production-like deployments should also enforce
[`security-profile-v1.md`](security-profile-v1.md).

The authority and identity path is implementable now: peer bindings, domain
ids, domain declarations, domain delegations, authority-chain validation,
served-domain-set computation, peer authorization, handshake stream behavior,
handshake message shape, offer objects, catalog filters, registry-reference
hashes, payload-type matching, message envelopes, and status objects are
specified.

The first SDK path is now in progress across `auki-protocol`, `auki-p2p`,
`auki-protocol-wasm`, and `auki-p2p-browser`. The preview examples prove the
RFC-shaped path for browser-to-node and browser-to-browser demos without using
Park, HTTP cache, or the legacy `auki-network` runtime for preview bytes.

The current implementation backlog is no longer a protocol TODO. It is a demo
and SDK validation queue for completing the remaining transport matrix,
especially node-to-node direct/relay proofs and relay-fallback edge cases.

`decentralized-peer-discovery.md` is a draft protocol/RFC slice. It is intended
as the prerequisite for later protocol crate validators, vectors, SDK helpers,
runtime integration, and demos. Those later slices must not invent different
candidate authority, expiry, state, or failure semantics.

Current `develop` still ships the legacy HTTP Discovery/ClusterManager runtime
path in `auki-network`. That runtime is current implementation evidence, not the
target decentralized architecture specified by this draft.

## Authority Boundary

Discovery is not a centralized Auki service and is not authority. Discovery
outputs candidate dial targets and hints only. Authority remains owned by the
transport-authenticated libp2p peer id, wallet-signed peer binding, domain
declarations/delegations, offer policy, and local application policy.

## Open Decisions Before Implementation

The draft intentionally leaves these human/protocol decisions open before SDK,
runtime, helper, app, demo, or example work:

- whether `advertise` delegation material is required inside candidate domain
  hints or only validated at publication/handshake boundaries;
- canonical candidate wire encoding, including whether this RFC should become
  normative JSON/JCS now or remain a draft object until protocol crate work;
- exact public API compatibility surface for later SDK helper methods;
- any authority/security semantic change beyond documenting the
  non-authoritative candidate boundary.

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

Start with [`backlog.md`](backlog.md), especially the remaining connectivity
matrix.
