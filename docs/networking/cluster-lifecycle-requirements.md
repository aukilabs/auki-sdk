# Cluster Lifecycle Requirements

Status: draft for requirements gathering. This is not an implementation spec.

Owner: TBD.

Last updated: 2026-05-19.

## Purpose

Define what the SDK must provide for domain discovery, peer connectivity,
spatial data exchange, and failure recovery before we design or change the
cluster implementation.

This document intentionally avoids prescribing Rust structs, protocol frames,
or libp2p behavior except where a requirement directly constrains them. The
current SDK implementation is evidence, not the source of truth.

## Working Principles

- Requirements come before design.
- Product behavior comes before implementation mechanics.
- The SDK should define clustering behavior; app repos should configure and use
  it, not reimplement it.
- Discovery is optional infrastructure for discoverability. It is not assumed
  to be required for every peer.
- libp2p is a transport mechanism. It should not define the product model by
  itself.

## Sources To Review

This doc should be filled by extracting requirements and contradictions from:

- `auki-sdk` docs, changelogs, parking lots, tests, and current APIs.
- Discovery service API and operational behavior.
- Park, Sentinel, BoosterApp, BracketApp, and RealmanApp current integration
  assumptions.
- Nils direction.
- Live test reports from Park/Sentinel/robots.

Each source should be treated as evidence. If two sources disagree, record the
conflict here and resolve it as a decision.

## Vocabulary To Define

These terms must be agreed before implementation design continues.

### Participant

An actor running SDK networking code. Examples: Park, Sentinel, BoosterApp,
BracketApp, RealmanApp, or a future relay service.

Open questions:

- Is every participant expected to publish spatial data? Not necessarily
- Can a participant be read-only, for example Sentinel? yes, but sentinel is producing rgb ... 
- Can a participant be private and never appear in Discovery? yes

### Domain

Candidate meaning: an authority boundary for spatial data, resources, frames,
sensors, streams, and identity.

Open questions:

- Is a domain owned by one participant by default? yes, imo, domain manager = domain owner
- Can a domain be shared by many participants? IMO... every participant have its own domain and they form a p2p cluster where they can exchange data between peer... so it is not like a domain is "shared" but more a collection of map that exchange data on a p2p cluster.
- If a participant owns a domain, is that participant always the Manager of its
  own domain? Not necessarely... we want the domain owner to "delegate" the manager role... domain owner is the private key that own the domain, while the manager role is more the runtime ?
- What does "domain death" mean when the owning participant exits? I m not sure what you mean here... 

### Cluster

Candidate meanings:

1. A shared authority group with one current Manager.
2. A connectivity group of participants exchanging data.
3. A UI/session grouping over discovered participants.

Open questions:

- Which meaning is required for v1?
- Do we need a cluster-wide Manager at all?
- Is cluster membership authoritative, or only a convenience view?

### Manager

Candidate meaning: the participant authorized to mutate membership and publish
the current authoritative state for a domain or cluster.

Open questions:

- Is Manager a property of a participant-owned domain?
- Is Manager a property of a shared cluster?
- Is Manager handoff required for v1?
- If every participant owns its own domain, what does Manager handoff mean?

### Discovery

Candidate meaning: optional rendezvous service for participants that want to be
discoverable by others.

Open questions:

- Is Discovery only an index of presence and dialable addresses? discovery should essentially be "publicly advertise domain"... 
- Does Discovery ever store authoritative membership? no
- Is Discovery allowed to be stale? this is something we need to handle carefully... they should be allowed but not affect the usage
- What should happen when Discovery points at a dead participant? i dont think it shouuld point to participant.. mostly to domain and how to talk to them?  

## Candidate Architecture Requirement

The current leading candidate is:

> Each participant must own and manage its own domain. Participants discover or
> are configured with each other, connect peer-to-peer, authorize as needed,
> and exchange spatial data. Discovery is an optional way for participants to
> advertise presence and dialability.

This is not yet a decision. It should be validated with CEO and Nils before any
SDK redesign.

Implications if accepted:

- Manager handoff is not a baseline requirement for every peer group.
- Shared cluster authority is optional and higher-level.
- Failure is localized: if one participant dies, its own domain disappears or
  becomes unavailable, but other participants continue.
- Park viewing robot streams does not require Park and all robots to share one
  cluster Manager.
- Discovery answers "who is advertising themselves and how can I dial them?",
  not "who owns the world?"

## Actors

### Park

Expected role:

- User-facing viewer/control app.
- Discovers or is configured with robot participants.
- Fetches resources, frames, sensors, and streams.
- May publish its own resources or streams, for example microphone audio.

Requirements to confirm:

- Must Park remain usable when no robots are online?
- Must robots remain usable when Park exits?
- Should Park ever be Manager for another participant's domain?

### Sentinel

Expected role:

- Diagnostic and observation peer.
- Can join/connect without being the source of spatial data.
- Should help inspect cluster/domain state without changing authority.

Requirements to confirm:

- Should Sentinel register in Discovery by default?
- Should Sentinel ever be eligible for Manager/election?

### Robot Apps

Examples: BoosterApp, BracketApp, RealmanApp.

Expected role:

- Own robot-local sensors, resources, frames, and streams.
- Advertise themselves when they want Park or other peers to find them.
- Continue operating without Park.

Requirements to confirm:

- Does each robot own its own domain?
- Do robot apps need to connect to each other directly?
- Should one robot ever become Manager for another robot's domain?

### Discovery Service

Expected role:

- Optional rendezvous/index for discoverable participants.
- Stores current advertised presence, dialable addresses, and perhaps resource
  summaries.
- Expires stale presence.

Requirements to confirm:

- What is the TTL/liveness expectation?
- Is explicit deregistration required?
- Can a private participant connect to discovered participants without
  registering itself?

### Relay Service

Expected role:

- Optional connectivity fallback when direct dialing fails.
- Does not change domain authority or ownership.

Requirements to confirm:

- Is relay required for v1 production?
- Is LAN-only acceptable for the current milestone?

## Functional Requirements

### R1: Participants Can Be Private Or Discoverable

A participant must not be required to register with Discovery merely to use the
SDK.

A participant that wants to be found through Discovery must be able to register
presence and dialable addresses.

A participant that does not register must still be able to connect through
manual configuration, invitation, or direct address exchange.

### R2: Discovery Is Optional Rendezvous

Discovery should answer:

- which domains or runtime presences are advertising themselves;
- how to dial them;
- what high-level capabilities or resources they claim.

Discovery should not be assumed to answer:

- who owns a domain;
- who is allowed to publish spatial data;
- who is the global Manager;
- whether a private participant exists.

Decision needed: should Discovery ever store authoritative membership
snapshots, or is that out of scope for v1?

### R2.1: Discovery Provides Entrypoints, Not The Full Peer Graph

Discovery should not be assumed to return every peer in the cluster peer graph
around an advertised domain.

A participant may advertise Domain A in Discovery. Another participant may ask
Discovery how to connect to Domain A, dial the advertised entrypoint, and then
learn about additional peers in the related cluster after connection. Those
additional peers may each own their own domains and do not necessarily need
their own Discovery records.

Example:

1. Park advertises Domain A through Discovery.
2. Robot 1 discovers Domain A, dials Park, and begins exchanging data about
   Domain A.
3. Robot 2 later discovers Domain A and dials Park.
4. After connecting, Robot 2 may learn that Robot 1 is already in the cluster
   peer graph around Domain A and may be able to connect to Robot 1 directly,
   even if Robot 1 never advertised itself through Discovery.

Open questions:

- What protocol or SDK surface teaches a newly connected peer about associated
  non-discoverable peers?
- Are associated peers merely suggested dial targets, or do they imply
  authorization?
- Can the entrypoint choose which associated peers to reveal?
- Is the peer graph scoped to one advertised domain, or can it span several
  participant-owned domains that are currently exchanging data?

### R3: Participant-Owned Domains Are The Baseline Candidate

Each participant should be able to own its own authority boundary for its
spatial data and resources.

The SDK should not require a shared cluster Manager just for Park to view robot
data.

Decision needed: confirm whether this is the intended model.

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
- syncing resources;
- ready;
- degraded;
- lost.

Failure of one remote participant should not invalidate unrelated peer
relationships.

Example: if Park is connected to Robot A, Robot B, and Robot C, and Robot C
goes offline, Park should mark Robot C as lost or degraded while keeping Robot A
and Robot B ready if their connections still work.

### R5: Peers Exchange Spatial Knowledge Directly

Each participant may maintain its own local spatial state: observations,
resources, maps, frames, streams, or other domain-specific spatial knowledge.

After participants discover or are configured with each other, they should be
able to exchange relevant spatial knowledge directly with each other.

Discovery may help participants find an entrypoint, but Discovery should not be
the data exchange path. Discovery should not proxy spatial data.

The SDK should support:

- identifying a remote peer;
- understanding what spatial data a remote peer can share;
- requesting or subscribing to that data;
- receiving the data directly;
- understanding why an exchange failed.

Open questions:

- What is the minimum spatial state a peer must expose?
- Which parts of a peer's local spatial state are public, private, or
  permissioned?
- Do peers exchange snapshots, deltas, subscriptions, streams, or some
  combination?
- Which current SDK surfaces are requirements, and which are implementation
  details?

### R6: Shared Cluster Authority Is Optional Until Proven Required

The SDK may eventually support shared domains/clusters with Manager election,
but the requirements must state why they are needed.

Shared cluster authority should not be the default answer to:

- showing robot streams in Park;
- finding peers;
- exchanging resources;
- keeping a UI directory updated.

Decision needed: identify any product workflow that truly requires one shared
domain cluster with a current Manager.

## Failure Requirements

### F1: Park Exits

Expected behavior to confirm:

- Robots continue owning and serving their own data.
- Other observers can still discover/connect to robots if robots are
  discoverable.
- No robot should lose its own domain solely because Park exited.

### F2: Robot App Exits

Expected behavior to confirm:

- That robot becomes unavailable.
- Other participants remain available.
- Park should show the robot as lost/stale/offline with a clear reason if
  known.

### F3: Discoverable Participant Loses Discovery

Expected behavior to confirm:

- Existing peer connections may continue.
- New peers may not discover it until Discovery registration returns.
- The participant should report degraded discovery presence, not necessarily
  degraded local domain operation.

### F4: Direct Dial Fails

Expected behavior to confirm:

- SDK tries all advertised addresses with bounded timeouts.
- SDK reports whether failure was address parse, dial timeout, connection
  refused, handshake failure, authorization failure, or protocol failure.
- Relay may be used if configured/available.

### F5: Discovery Has Stale Presence

Expected behavior to confirm:

- Dial should fail clearly.
- Discovery should eventually expire stale presence.
- SDK should not invent authority or membership from stale Discovery data.

### F6: Mixed SDK Versions

Expected behavior to confirm:

- Rolling deploys should be supported when possible.
- Stable protocol ids should not gain new required fields without backward
  compatibility.
- If compatibility cannot be maintained, protocol ids must bump.

### F7: Shared Manager Dies, If Shared Manager Exists

Only applies if shared clusters/domains are confirmed as a requirement.

Expected behavior to define:

- Who is eligible to become Manager?
- How is the winner chosen?
- What happens under partition?
- What state must Discovery update?
- What state must peers update?
- What happens if there is only one survivor?

## Networking Requirements

### N1: Listen And Advertised Addresses Are Different

The SDK must distinguish:

- listen addresses: where the local swarm binds;
- advertised addresses: what other participants should dial.

Non-dialable bind addresses such as `/ip4/0.0.0.0/...` must not be treated as
cross-machine advertised addresses unless explicitly intended for local-only
testing.

### N2: Discovery Should Store Dialable Addresses

If a participant registers with Discovery, the registered addresses should be
dialable by the intended peers or should be explicit relay-mediated addresses.

### N3: Relay Is Connectivity, Not Authority

Relay support should not change who owns a domain, who can publish data, or who
is authorized. It only changes how peers connect.

Decision needed: is relay a v1 production requirement?

## Authority Requirements

### A1: Data Authority

A participant should be authoritative for the spatial data it produces unless a
different authority model is explicitly required.

Open questions:

- Do consumers trust producer-declared frames/resources by default?
- Is signing required in v1?
- Are successor tokens relevant if domains are participant-owned?

### A2: Membership Authority

If a shared cluster exists, membership authority must be explicit.

Open questions:

- Who admits participants?
- Can a participant remove another participant?
- What happens if the authority participant disappears?
- Is membership even required for peer-to-peer resource exchange?

## Observability Requirements

Logs/status should answer these without noisy frame-level output:

- Am I discoverable?
- What domain do I own?
- Which peers do I know about?
- How did I learn about each peer?
- Can I dial each peer?
- Am I connected to each peer?
- Am I authorized with each peer?
- What resources did each peer advertise?
- Why did a peer become degraded or lost?
- If a shared Manager exists, who is it and why?

Status should be available through SDK-facing APIs, not only ad hoc app logs.

## Scenarios To Validate

### S1: Park Finds One Robot

Given a robot registers with Discovery, Park should discover it, dial it, fetch
resources, and open streams.

Questions:

- Does Park need to register too?
- Does this require shared cluster membership?

### S2: Park Finds Many Robots

Given several robots register with Discovery, Park should discover each one and
track each relationship independently.

Questions:

- Does one robot failure affect other robot relationships?
- Is there any required robot-to-robot connection?

### S3: Robot Exists Without Park

Given Park is offline, a robot should continue operating and, if configured,
advertising itself.

Questions:

- What user-visible state should Sentinel show?

### S4: Private Peer Connects To Discoverable Peer

Given a peer is not registered with Discovery but knows a discoverable peer's
address, it should be able to connect if authorized.

Questions:

- Does the discoverable peer need to know about the private peer in advance?
- How is authorization established?

### S4.1: Peer Learns Additional Peers After Discovery Entrypoint

Given Domain A is advertised through one Discovery entrypoint, a newly
connected peer should be able to learn about additional peers in the cluster
peer graph around that domain after connecting, without requiring every peer in
that graph to have its own Discovery record.

Questions:

- What state is exchanged after the initial entrypoint connection?
- Which peer is allowed to reveal the additional peer graph?
- Are learned peers dialed automatically or surfaced as candidates?
- How does this avoid becoming an implicit authoritative membership list?

### S5: Shared Cluster Required Workflow

Identify a real workflow that cannot be solved by participant-owned domains and
peer relationships.

Questions:

- What shared state exists?
- Who owns it?
- Why must it survive the owner's death?

## Decisions Needed

1. Is the baseline model participant-owned domains or shared domain clusters?
2. Is "cluster" an authority object or a connectivity/session grouping?
3. Is Discovery optional for participants that do not want to be discoverable?
4. Should Park ever manage another participant's domain?
5. Should robots operate normally without Park?
6. Is shared Manager handoff a v1 requirement?
7. Is relay a v1 production requirement?
8. Is mixed SDK rolling deploy compatibility required?
9. What should replace or deprecate `/auki/sensors/0.0.1`, if anything?
10. What is the minimum debug/status surface required before more field tests?

## Decision Log

Add decisions here as they are confirmed.

### Decision: TBD

Date: TBD.

Owner: TBD.

Decision:

Reasoning:

Implications:

Follow-up:

## Non-Goals Until Requirements Are Confirmed

- Implementing a new Manager election algorithm.
- Adding relay-specific app logic.
- Treating the current SDK cluster implementation as normative.
- Requiring every peer to register with Discovery.
- Designing protocol changes before the domain/cluster model is agreed.
