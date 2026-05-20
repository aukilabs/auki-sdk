# Cluster Lifecycle Requirements

Status: draft requirements baseline. This is not an implementation spec.

Owner: TBD.

Last updated: 2026-05-20.

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Define the product requirements for domain discovery, peer connectivity,
spatial data exchange, and failure recovery before continuing SDK networking
redesign.

The current SDK implementation is evidence, not the source of truth. Existing
Manager election, shared membership, and Discovery cluster APIs may be useful
prototypes, but they are not baseline requirements unless this document or an
RFC explicitly makes them so.

## Working Principles

- Requirements come before design.
- Product behavior comes before implementation mechanics.
- Every participant owns and maintains its own domain.
- Clusters are peer connectivity/session graphs by default, not shared
  authority objects.
- Discovery is optional infrastructure for findability. It is not required for
  private participants or direct configuration.
- Spatial data exchange happens peer-to-peer after discovery/configuration and
  authorization.
- libp2p is a transport mechanism. It should not define the product model by
  itself.

## Audit Summary

The documentation set currently contains two competing models:

- Legacy `auki/` concept and capability docs describe a Domain Cluster with a
  current Manager, shared Cluster Registry, Manager-written membership, and
  Manager failover.
- Newer SDK/app docs and live app behavior mostly need participants to find
  each other, connect directly, advertise what they can share, and stream or
  fetch data peer-to-peer.

The v1 foundation should choose the smaller model: participant-owned domains
and direct exchange. Shared cluster authority remains a future overlay that
must justify itself with a concrete workflow.

## Terminology

Terms used by this requirements draft are defined in the related glossary.

This document may use product examples to explain requirements, but protocol
wording should use the glossary terms consistently. In particular,
`participant`, `peer`, `wallet`, `domain`, `domain id`, `peer binding`,
`Discovery record`, `offer`, `Get`, and `Subscribe` are defined there.

## Architecture Decision

### Decision: Participant-Owned Domains Are The Baseline

Date: 2026-05-20.

Owner: RFC working session.

Decision:

Each participant owns and maintains its own domain. Participants may
discover or be configured with each other, connect peer-to-peer, authorize as
needed, advertise what spatial data they can share, and exchange that data
directly.

Reasoning:

This is the smallest model that satisfies the core product requirement:
participants maintain spatial state, form peer-to-peer clusters, and exchange
spatial data. It also matches the ad-hoc robot story: each robot keeps its own
domain and creates transforms when it learns enough about another domain.

Implications:

- Park viewing robot streams does not require Park and robots to share one
  cluster Manager.
- Robots continue operating without Park.
- Losing one participant does not invalidate other participants' domains.
- Discovery advertises entrypoints, not "who owns the world".
- Shared Manager handoff is not a v1 requirement.

Follow-up:

Legacy docs that present shared Domain Clusters as the only model should be
reworked after this RFC stabilizes.

### Decision: Discovery Provides Entrypoints Only

Date: 2026-05-20.

Owner: RFC working session.

Decision:

Discovery records advertise domains or runtime presences and the dialable
entrypoints needed to attempt connection. Discovery is not authoritative for
membership, data ownership, domain ownership, authorization, or the full peer
graph.

Reasoning:

Discovery should make participants findable without becoming a central control
plane. It must support private participants and tolerate stale records.

Implications:

- Discovery registration is optional.
- A private participant can dial a discoverable participant if it has the
  address/invitation/configuration and passes authorization.
- Existing peer connections can continue while Discovery is unavailable.
- Discovery TTL and explicit deregistration are operational hygiene, not
  authority.

### Decision: KISS Exchange Surface Is Offer / Get / Subscribe

Date: 2026-05-20.

Owner: RFC working session.

Decision:

The v1 spatial exchange requirement is a simple peer-to-peer contract:

- `Offer`: a peer advertises named spatial data it can share now.
- `Get`: a peer fetches a snapshot, descriptor, or bounded data product by
  offer id.
- `Subscribe`: a peer opens live updates by offer id.

Reasoning:

This captures the current useful behavior of resource catalogs and live streams
without locking the RFC to current protocol names or designing a full spatial
query language too early.

Implications:

- Current `/auki/resources/0.0.1` and `/auki/stream/0.1.0` are evidence for
  the shape, not necessarily final names.
- Generic `Get` / `Query` over arbitrary spatial primitives is deferred.
- Type-specific stream protocols can exist underneath the subscription path,
  but the product model is "what can you offer?" followed by "give me that" or
  "keep me updated".

## Actor Requirements

### Park

Expected role:

- User-facing viewer/control app.
- Discovers or is configured with robot participants.
- Tracks each remote participant independently.
- Fetches offers and subscribes to streams.
- May publish its own offers, for example microphone audio.

Requirements:

- Park must remain usable when no robots are online.
- Robots must remain usable when Park exits.
- Park should not need to manage another participant's domain to view or
  control that participant.

### Sentinel

Expected role:

- Diagnostic and observation peer.
- May publish camera or diagnostic offers.
- Can connect without becoming a shared authority.
- Helps inspect domain/peer state without changing ownership.

Requirements:

- Sentinel registration in Discovery should be configurable.
- Sentinel should not be eligible for Manager/election because Manager/election
  is not a v1 baseline requirement.

### Robot Apps

Examples: BoosterApp, BracketApp, RealmanApp.

Expected role:

- Own robot-local sensors, resources, frames, streams, maps, and logs.
- Advertise themselves when Park or other peers should find them.
- Continue operating without Park.
- Expose offers for live streams, transform edges, pose logs, maps, or other
  spatial products as they become available.

Requirements:

- Each robot should own its own domain.
- Robot-to-robot connections are optional and workflow-driven.
- One robot should not become Manager for another robot's domain in the v1
  baseline.

### Discovery Service

Expected role:

- Optional index of advertised domains/presences.
- Stores dialable entrypoint metadata and enough summary data for a peer to
  decide whether to dial.
- Expires stale advertisements.

Requirements:

- Discovery must not store authoritative peer membership.
- Discovery must not be required for private participants.
- Discovery should expose freshness/TTL semantics so stale advertisements are
  explainable.

### Relay Service

Expected role:

- Optional connectivity fallback when direct dialing fails.
- Does not change domain authority, data authority, or authorization.

Requirements:

- Relay is not a v1 authority feature.
- Whether relay is required for a production milestone remains an operational
  deployment decision.

## Functional Requirements

### R1: Participants Can Be Private Or Discoverable

A participant must not be required to register with Discovery merely to use the
SDK.

A participant that wants to be found through Discovery must be able to register
presence, dialable entrypoints, and high-level offer metadata.

A participant that does not register must still be able to connect through
manual configuration, invitation, direct address exchange, or another discovery
mechanism.

### R2: Discovery Provides Entrypoints Only

Discovery should answer:

- which domains or runtime presences are advertising themselves;
- how to dial an advertised entrypoint;
- what high-level offers or capabilities are worth attempting a connection for;
- how fresh the advertisement is.

Discovery should not answer:

- who owns a domain;
- who is allowed to publish spatial data;
- who is allowed to consume spatial data;
- who is the global Manager;
- who belongs to a cluster;
- whether a private participant exists.

### R3: Participants Own Their Domains

Each participant should own and maintain its own authority boundary for spatial
data and resources.

The SDK should not require a shared cluster Manager just for:

- Park to view robot streams;
- peers to discover each other's offers;
- peers to fetch transform metadata;
- peers to subscribe to live sensor streams;
- a UI directory to stay updated.

### R4: Peer Connectivity State Is Tracked Per Remote Peer

A local participant should track connectivity and readiness independently for
each remote participant.

Candidate relationship states:

- unknown;
- discovered;
- configured;
- dialing;
- connected;
- authorized;
- syncing offers;
- ready;
- degraded;
- lost.

Failure of one remote participant should not invalidate unrelated peer
relationships.

Example: if Park is connected to Robot A, Robot B, and Robot C, and Robot C
goes offline, Park should mark Robot C as lost or degraded while keeping Robot A
and Robot B ready if their connections still work.

### R5: Peers Exchange Spatial Data Directly

Each participant may maintain its own local spatial state: observations,
resources, maps, frames, streams, logs, transforms, or other domain-specific
spatial data.

After participants discover or are configured with each other, they should be
able to exchange relevant spatial data directly with each other.

Discovery may help participants find an entrypoint, but Discovery should not be
the data exchange path. Discovery should not proxy spatial data.

### R6: Offer / Get / Subscribe Is The Minimum Exchange Shape

The SDK should support a simple peer-to-peer exchange shape:

- `Offer`: identify what spatial data a peer can share now.
- `Get`: fetch a snapshot, descriptor, or bounded data product by offer id.
- `Subscribe`: receive live updates by offer id.

An offer should include enough information for a consumer to decide whether and
how to request it:

- offer id;
- producer peer id;
- producer domain id;
- kind, for example sensor stream, transform edge, pose log, map, or point
  cloud;
- payload or schema identifier;
- relevant clock and frame references when spatial/temporal interpretation is
  needed;
- supported access mode: snapshot, subscription, or both;
- protocol hint for the concrete fetch/subscribe path;
- optional freshness or status metadata.

Offer metadata is producer-declared and not automatically trusted as truth. It
is a menu of available exchange paths, not proof of authority.

### R7: Shared Cluster Authority Is Optional Until Proven Required

The SDK may eventually support shared domains/clusters with Manager election,
but the requirements must state why they are needed.

Shared cluster authority should not be the default answer to:

- showing robot streams in Park;
- finding peers;
- exchanging resources;
- keeping a UI directory updated;
- subscribing to live streams;
- replaying another peer's pose/path data.

Any proposal for shared authority must identify the shared state, owner,
failure semantics, partition behavior, and migration path from participant-owned
domains.

## Failure Requirements

### F1: Park Exits

Expected behavior:

- Robots continue owning and serving their own domains.
- Other observers can still discover/connect to robots if robots are
  discoverable.
- No robot loses its own domain solely because Park exited.

### F2: Robot App Exits

Expected behavior:

- That robot's live offers become unavailable.
- Other participants remain available.
- Park should show the robot as lost/stale/offline with a clear reason if
  known.

### F3: Discoverable Participant Loses Discovery

Expected behavior:

- Existing peer connections may continue.
- New peers may not discover it until Discovery registration returns.
- The participant should report degraded discovery presence, not degraded local
  domain operation unless local operation is also affected.

### F4: Direct Dial Fails

Expected behavior:

- SDK tries advertised addresses with bounded timeouts.
- SDK reports whether failure was address parse, dial timeout, connection
  refused, handshake failure, authorization failure, or protocol failure.
- Relay may be used if configured/available.

### F5: Discovery Has Stale Presence

Expected behavior:

- Dial fails clearly.
- Discovery eventually expires stale presence.
- SDK does not invent authority or membership from stale Discovery data.

### F6: Mixed SDK Versions

Expected behavior:

- Rolling deploys should be supported when possible.
- Stable protocol ids should not gain new required fields without backward
  compatibility.
- If compatibility cannot be maintained, protocol ids must bump.

### F7: Shared Manager Dies, If Shared Manager Exists

Out of scope for the v1 baseline. Any later shared Manager RFC must define:

- who is eligible to become Manager;
- how the winner is chosen;
- what happens under partition;
- what state Discovery updates;
- what state peers update;
- what happens if there is only one survivor.

## Networking Requirements

### N1: Listen And Advertised Addresses Are Different

The SDK must distinguish:

- listen addresses: where the local swarm binds;
- advertised addresses: what other participants should dial.

Non-dialable bind addresses such as `/ip4/0.0.0.0/...` must not be treated as
cross-machine advertised addresses unless explicitly intended for local-only
testing.

### N2: Discovery Should Store Dialable Entrypoints

If a participant registers with Discovery, the registered addresses should be
dialable by the intended peers or should be explicit relay-mediated addresses.

### N3: Relay Is Connectivity, Not Authority

Relay support should not change who owns a domain, who can publish data, or who
is authorized. It only changes how peers connect.

## Authority Requirements

### A1: Data Authority

A participant is authoritative for the spatial data it produces unless a
different authority model is explicitly required.

Offer metadata is not proof. Consumers may need signatures, allowlists,
operator trust, or application policy before using a producer's data.

Open questions:

- Do consumers trust producer-declared frames/resources by default in trusted
  lab deployments?
- Is signing required in v1, or is it a later hardening layer?
- How does a domain owner wallet delegate runtime management without creating a
  cluster-wide Manager?

### A2: Membership Authority

Baseline peer relationships do not require an authoritative shared membership
list.

Open questions for future shared clusters:

- Who admits participants?
- Can a participant remove another participant?
- What happens if the authority participant disappears?
- Is membership required for this workflow, or is peer authorization enough?

## Observability Requirements

Logs/status should answer these without noisy frame-level output:

- Am I discoverable?
- What domain do I own or manage locally?
- What am I advertising?
- Which peers do I know about?
- How did I learn about each peer?
- Can I dial each peer?
- Am I connected to each peer?
- Am I authorized with each peer?
- What offers did each peer advertise?
- Why did a peer become degraded or lost?
- Is Discovery degraded separately from peer connectivity?

Status should be available through SDK-facing APIs, not only ad hoc app logs.

## Scenarios To Validate

### S1: Park Finds One Robot

Given a robot registers its domain or runtime presence with Discovery, Park
should discover it, dial it, fetch offers, and subscribe to streams.

This must not require Park to register in Discovery or require shared cluster
membership.

### S2: Park Finds Many Robots

Given several robots register with Discovery, Park should discover each one and
track each relationship independently.

One robot failure must not affect other robot relationships. Robot-to-robot
connections are optional and workflow-driven.

### S3: Robot Exists Without Park

Given Park is offline, a robot should continue operating and, if configured,
advertising itself.

### S4: Private Peer Connects To Discoverable Peer

Given a peer is not registered with Discovery but knows a discoverable peer's
address, it should be able to connect if authorized.

Authorization may be open, preconfigured, invite-based, wallet-based, or
application-specific. It must not depend solely on Discovery presence.

### S5: Peer Learns Additional Peers After Discovery Entrypoint

Given Domain A is advertised through one Discovery entrypoint, a newly
connected peer may learn about additional peers through peer-to-peer exchange.

Learned peers are candidate dial targets or offers, not authoritative
membership. The revealing peer may choose what to reveal according to its own
policy.

### S6: Offer / Get / Subscribe

Given two connected and authorized peers, Peer A should fetch Peer B's offers,
choose one, and either fetch a snapshot or subscribe to updates.

The same shape should support a live camera stream, live point cloud, direct
transform edge, pose/path replay, or future map fragment without requiring a
new cluster authority model.

### S7: Shared Cluster Required Workflow

Identify a real workflow that cannot be solved by participant-owned domains and
peer relationships.

Questions:

- What shared state exists?
- Who owns it?
- Why must it survive the owner's death?
- Why are signed offers and peer authorization insufficient?

## Remaining Decisions

1. What is the exact v1 offer wire shape and protocol id?
2. What offer kinds are required for the next Park/robot milestone?
3. What authorization model is sufficient for trusted lab deployments?
4. What status API should expose peer and offer lifecycle state?
5. Is relay required for the next production milestone?
6. What should replace or deprecate `/auki/sensors/0.0.1`, if anything?
7. Which legacy docs should be updated first after this RFC stabilizes?

## Non-Goals For The V1 Foundation

- Implementing a new Manager election algorithm.
- Treating the current SDK cluster implementation as normative.
- Requiring every peer to register with Discovery.
- Requiring authoritative shared membership for peer-to-peer exchange.
- Designing a generic spatial query DSL.
- Designing payment, slashing, or booking semantics.
- Designing canonical shared maps before a concrete workflow requires them.
