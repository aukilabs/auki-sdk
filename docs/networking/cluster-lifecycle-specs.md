# Cluster Lifecycle Specs

Status: draft normative baseline.

Last updated: 2026-05-19.

Related requirements draft:
[`cluster-lifecycle-requirements.md`](cluster-lifecycle-requirements.md).

## Status Of This Document

This document records cluster-lifecycle contracts that are stable enough to
guide SDK design and review. It intentionally does not contain the full future
architecture. Open product questions belong in the requirements document until
they are resolved.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", "MAY",
and "OPTIONAL" are to be interpreted as described in RFC 2119.

## Non-Normative Context

The SDK currently has code for shared cluster membership, Manager election, and
Manager handoff. That implementation is not treated as the source of truth for
this document. This document defines only the contracts we are prepared to rely
on while requirements gathering continues.

## RFC-0001: Discovery Is Optional Rendezvous

### Requirement

A participant MUST NOT be required to register with Discovery merely to use SDK
networking or to connect to another peer.

A participant MAY register with Discovery when it wants to be discoverable by
other participants.

A participant that does not register with Discovery MAY still connect to other
participants through manual configuration, invitation, direct address exchange,
or another discovery mechanism.

### Discovery Authority

Discovery MUST be treated as rendezvous/presence infrastructure unless a later
RFC explicitly expands its authority.

Discovery MUST NOT be treated as authoritative for:

- domain ownership;
- spatial-data ownership;
- global cluster membership;
- private participant existence;
- authorization to consume or publish data.

### Discovery Records

A Discovery record SHOULD answer:

- what domain or runtime presence is being advertised;
- how that presence can be dialed;
- enough metadata for a peer to decide whether to attempt connection.

A Discovery record MAY advertise an entrypoint into a cluster peer graph rather
than listing every participant in that graph.

After connecting to a Discovery-advertised entrypoint, a participant MAY learn
about additional peers in that cluster graph through SDK peer-to-peer
mechanisms. Those additional peers MAY each own their own domains and are not
REQUIRED to have individual Discovery records.

A Discovery record MAY be stale. Stale Discovery data MUST NOT invalidate
existing peer-to-peer connections by itself.

### Consequences

Existing peer relationships SHOULD continue when Discovery is temporarily
unavailable, assuming the underlying peer-to-peer transport remains healthy.

SDK status/diagnostics SHOULD distinguish "Discovery presence degraded" from
"peer relationship degraded".

## RFC-0002: Private And Discoverable Participants

### Requirement

The SDK MUST support both private and discoverable participants.

A discoverable participant registers presence through Discovery or an equivalent
index.

A private participant does not register presence in Discovery but can still:

- dial a discoverable participant;
- be dialed through explicit configuration;
- participate in authorized peer-to-peer exchange once connected.

### Consequences

Discovery queries MUST NOT be used to prove that a private participant does not
exist.

Peer authorization MUST NOT depend solely on whether the peer appeared in
Discovery.

## RFC-0003: Listen Addresses And Advertised Addresses Are Different

### Requirement

The SDK MUST distinguish listen addresses from advertised addresses.

- A listen address is where the local network runtime binds.
- An advertised address is what another participant should dial.

The SDK MUST NOT automatically advertise non-dialable bind addresses as
cross-host dial addresses.

Examples of addresses that MUST NOT be auto-advertised for cross-host use:

- `/ip4/0.0.0.0/...`
- loopback addresses;
- link-local addresses;
- unspecified IPv6 addresses.

Operator-supplied advertised addresses MAY include addresses that auto-detection
would filter, including loopback addresses for same-machine tests and
relay-mediated multiaddrs.

### Discovery Interaction

If a participant registers with Discovery, the registered dial addresses SHOULD
be dialable by the intended peers or SHOULD be explicit relay-mediated
addresses.

### Consequences

Apps SHOULD expose listen and advertised address configuration separately.

SDK diagnostics SHOULD report the final advertised address set and identify
whether each address was auto-detected, operator-supplied, or relay-mediated.

## RFC-0004: Relay Is Connectivity, Not Authority

### Requirement

Relay support MAY be used to establish peer-to-peer connectivity when direct
dialing fails or is unavailable.

Relay support MUST NOT change:

- domain ownership;
- runtime management authority;
- peer authorization;
- spatial data ownership;
- stream/resource semantics.

### Consequences

A relay-mediated connection MUST be treated as a transport path to the same
remote peer identity, not as a different authority model.

Discovery MAY advertise relay-mediated multiaddrs when direct addresses are not
sufficient.

## RFC-0005: Peer Connectivity State Is Tracked Per Remote Peer

### Requirement

A participant SHOULD track connectivity and readiness state independently for
each remote participant.

Failure of one peer relationship MUST NOT force unrelated peer relationships to
restart or become invalid unless a higher-level requirement explicitly says so.

### Candidate State Model

The following states are non-normative names, but the SDK SHOULD expose
equivalent diagnostic information:

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

### Consequences

Park losing one robot SHOULD NOT imply that Park lost all robots.

A robot exiting SHOULD make that robot unavailable; it SHOULD NOT by itself
invalidate other robots' domains or peer relationships.

## RFC-0006: Peers Exchange Spatial Knowledge Directly

### Requirement

After discovery/configuration and authorization, participants SHOULD exchange
spatial knowledge peer-to-peer.

Each participant MAY maintain and expose its own local spatial state.

The SDK SHOULD provide peer-to-peer mechanisms for participants to:

- identify each other;
- understand what spatial data a remote peer can share;
- request or subscribe to that data;
- receive the data directly;
- understand why an exchange failed.

### Consequences

Discovery MAY help locate an entrypoint, but Discovery MUST NOT be required as
the transport for spatial data exchange.

## RFC-0007: Protocol Versions Are Compatibility Contracts

### Requirement

A protocol ID, such as `/auki/example/0.0.1`, identifies a wire contract
between SDK versions. Once a protocol version is used by deployed peers,
changes to that protocol MUST either remain backward compatible or use a new
protocol version.

For an existing protocol version, implementations:

- MUST keep decoding previously valid messages;
- MUST NOT add a new required field unless old messages still decode with a
  safe default;
- MUST NOT rename existing fields;
- MUST NOT change the meaning of an existing field;
- MUST ignore unknown additive fields when feasible;
- SHOULD include locked field-name tests;
- SHOULD include compatibility tests for any accepted legacy shape.

Incompatible wire changes MUST use a new protocol ID.

### Example

If `/auki/example/0.0.1` originally accepted:

```json
{
  "value": "abc"
}
```

then adding a required `sender_peer_id` to the same protocol ID is
incompatible unless the reader can still handle frames without it.

An incompatible version should instead use a new protocol ID such as
`/auki/example/0.0.2`.

## RFC-0008: Observability Must Explain State Transitions

### Requirement

SDK diagnostics MUST make core lifecycle state explainable without noisy
per-frame logs.

Diagnostics SHOULD answer:

- whether this participant is discoverable;
- what it is advertising;
- which peers are known;
- how each peer was learned;
- whether each peer is dialable;
- whether each peer is connected;
- whether each peer is authorized;
- what spatial data each peer claims it can share;
- why a peer became degraded or lost.

### Consequences

Heartbeat-frame logs, stream-frame logs, and repeated dial retry logs SHOULD be
rate-limited or omitted by default.

State transitions and failures SHOULD be logged once with enough context to
debug the lifecycle.

## Explicitly Not Specified Yet

The following are intentionally not normative in this document:

- whether the baseline architecture is participant-owned domains or shared
  domain clusters;
- whether a domain owner and runtime Manager are always the same actor;
- whether shared Manager handoff is required for v1;
- whether Discovery ever stores authoritative membership snapshots;
- whether `/auki/sensors/0.0.1` remains first-class or is replaced by
  `/auki/resources/0.0.1`;
- relay requirements for the current production milestone;
- authority/signature requirements for producer-declared spatial resources.

Those topics remain in
[`cluster-lifecycle-requirements.md`](cluster-lifecycle-requirements.md) until
explicitly decided.
