# Peer-To-Peer Cluster Drafts

Status: future extension drafts. This file is not part of the first
implementable baseline.

Last updated: 2026-05-22.

Related baseline:
[`baseline.md`](baseline.md).

Related backlog:
[`backlog.md`](backlog.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

This file parks planned protocol work that may become a future extension or
later baseline revision. Implementers do not need these drafts to implement the
first baseline in `baseline.md`.

## Drafts

### Dynamic Served-Domain Updates

Baseline behavior: served-domain changes require reconnect or a fresh
handshake. The changed served-domain set is not accepted until authority-chain
validation succeeds again.

Future work can define how a peer adds, removes, refreshes, or replaces served
domains during an active peer relationship.

That work needs to define:

- update message shape;
- validation trigger;
- authority-chain reuse;
- stale offer and subscription handling;
- failure mapping.

### Discovery Record Shape

Baseline behavior: Discovery is optional rendezvous/presence infrastructure.
Configured or manual peer-to-peer connectivity does not require a concrete
Discovery record.

Future work can define the concrete Discovery advertisement:

- domain id and optional display label;
- peer id and dialable advertised addresses;
- freshness fields such as `ttl`, `expires_at`, or `last_seen_at`;
- coarse, non-authoritative data-type hints;
- refresh, update, remove, and expiry behavior.

The record shape needs to preserve entrypoint advertisement semantics and avoid
becoming an authoritative offer catalog.

### Discovery Data-Type Hints

Baseline behavior: Discovery hints are implementation-defined and are not
required for baseline peer-to-peer interop.

Future work can define coarse data-type hints for Discovery records:

- hint vocabulary;
- how hints differ from offers;
- whether hints are free-form, registered, or both;
- freshness behavior for hints;
- client handling for missing, stale, or unsupported hints.

### Peer Graph Hints

Baseline behavior: learned peers are non-authoritative candidate dial targets
or offer sources. Baseline interop does not require a peer-graph hint exchange.

Future work can define how a peer shares additional peer candidates after
connection:

- whether learned peers are dialed automatically or surfaced as candidates;
- what metadata can be shared;
- whether a peer may hide known peers;
- how the exchange avoids becoming authoritative membership;
- whether DHT-style peer discovery is in scope.

### Concrete Clock-Sync Protocol

Baseline behavior: `RFC-0035` in `baseline.md` owns timestamp and clock
semantics. A concrete clock-sync protocol is not required for Offer, Get, or
Subscribe.

Future work can define:

- message flow;
- fields;
- offset calculation;
- delay calculation;
- sampling;
- retry behavior;
- failure behavior;
- status fields;
- any needed failure codes.

### Shared Offer-Kind Profiles

Baseline behavior: `RFC-0031` in `baseline.md` defines offer-kind
extensibility. The baseline does not define required data-kind semantics or
required payload schemas.

Future work can define shared offer-kind profiles and payload schemas. A shared
offer-kind profile should define:

- supported access modes;
- expected request `params`, if any;
- payload descriptor compatibility rules;
- payload object compatibility rules;
- required or optional registry-reference roles;
- path-specific size expectations, if narrower than `RFC-0028`;
- path-specific failure mapping, if more specific than `RFC-0028`.
