# Peer-To-Peer Cluster Protocol Specs

Status: draft normative baseline.

Last updated: 2026-05-20.

Related RFC backlog:
[`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md).

Related glossary:
[`glossary.md`](glossary.md).

## Scope Of This Version

This document specifies the first minimal version of the peer-to-peer cluster
protocol. Its scope is bootstrapping: peers identify each other, declare served
domains when they expose domain-scoped data, discover or configure reachable
peers, authorize connections, and exchange spatial data through simple
peer-to-peer relationships.

The goal is not centralized runtime control. The goal is a small protocol
foundation that lets peers form clusters and exchange spatial data directly.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", "MAY",
and "OPTIONAL" are to be interpreted as described in RFC 2119.

Terminology used by this document is defined in the related glossary.

Sections marked "To Fill" are placeholders for future RFC work. They are not
normative until their status changes.

## Protocol Structure

This document is ordered from protocol foundations to runtime behavior:

- identity and authority;
- peer and domain model;
- discovery and reachability;
- connection lifecycle;
- spatial data exchange;
- compatibility and observability.

## Identity And Authority

### RFC-0001: Peer Identity And Wallet Binding

#### Requirement

A peer id MUST be bound to a wallet identity by a wallet-signed peer binding.

Each peer MUST present a wallet identity through a verified peer binding.

A wallet MAY bind one or more peer ids.

A peer binding MUST include:

- wallet public key;
- peer id;
- signature by the wallet key.

A peer binding SHOULD include an issued-at timestamp.

A peer binding MAY include an expiry or metadata label.

#### Verification

When a peer presents a peer binding, the receiver MUST verify that:

- the connected libp2p peer id matches the bound peer id;
- the signature verifies against the wallet public key.

#### Consequences

A peer binding proves only that the wallet recognizes the connected libp2p
peer id as one of its runtime peers.

A peer binding MUST NOT be treated as proof of domain ownership, runtime
authority, data correctness, or dialability at any advertised address.

### RFC-0002: Peer Binding Schema (To Fill)

Define the concrete peer binding format:

- canonical signed bytes;
- wallet public key encoding;
- peer id encoding;
- signature encoding;
- issued-at, expiry, and metadata-label fields.

### RFC-0003: Domain Identity And Ownership

#### Requirement

A domain MUST have a stable domain id that can be verified without Discovery,
blockchain access, or any online registry.

A domain id MUST be derived from the domain owner wallet's public key and a
nonce:

domain_id = hash(domain_owner_wallet_public_key, nonce)

The nonce MUST be unique for domains created by the same domain owner wallet.

The domain owner wallet MUST sign a domain declaration that binds:

- domain id;
- domain owner wallet public key;
- nonce.

A receiver MUST verify the domain declaration by recomputing the domain id and
verifying the signature against the domain owner wallet public key.

The domain owner wallet MAY authorize runtime peers to advertise, serve, or
update data under that domain.

Domain ownership MUST NOT by itself be treated as proof that associated spatial
data is correct, canonical, complete, or trusted.

#### Runtime Authority

A peer MAY serve a domain directly when the peer controls the domain owner
wallet.

A peer MAY serve a domain on behalf of the domain owner wallet when it presents
a valid delegation signed by the domain owner wallet.

A valid delegation proves only the delegated authority it states.

#### External Bindings

External registries, blockchain records, NFTs, or tokenomics systems MAY bind
to a domain id.

Such bindings MUST NOT be required to create, identify, or use a domain in
peer-to-peer mode.

#### Consequences

The same domain id model supports local, LAN-only, offline, externally
registered, and tokenomics-backed domains.

Discovery may help locate peers that claim to serve a domain, but Discovery
does not create domain ownership or prove runtime authority.

### RFC-0004: Domain Declaration Schema (To Fill)

Define the concrete domain declaration format:

- hash function and canonical hash input;
- nonce size and encoding;
- domain owner wallet public key encoding;
- signature encoding;
- optional display-label handling.

### RFC-0005: Domain Delegation Schema (To Fill)

Define the concrete delegation format used when a peer serves a domain on
behalf of a domain owner wallet:

- required fields;
- permitted actions or scopes;
- validity windows;
- revocation or replacement behavior;
- how the delegation is presented during peer handshake.

### RFC-0006: Authority Chain Validation (To Fill)

Define the validation path from runtime connection to spatial data exchange:

- connected libp2p peer id;
- peer binding and wallet public key;
- domain declaration;
- delegation, when the peer does not control the domain owner wallet;
- accepted served domain set;
- offer catalog entries scoped to served domains;
- failure reasons when any link in the chain is missing or invalid.

## Peer And Domain Model

### RFC-0007: Serving Peers Declare Domains

#### Requirement

A peer MAY participate in spatial-data exchange without declaring a local domain
when it only consumes remote offers.

A peer that serves offers, publishes spatial data, or asks a remote peer to
accept it as serving a domain MUST declare that domain and MUST prove that it
controls the domain owner wallet or has a valid delegation.

A local domain is the authority boundary for spatial state served by the peer,
including frames, clocks, sensors, streams, logs, maps, transforms, offers, and
resources.

A peer MAY own or maintain a local domain without advertising it.

When a peer chooses to advertise a domain, it MAY do so through Discovery,
through peer-to-peer handshake or offer exchange, or through both. Discovery
advertisement is optional and does not replace domain declaration and authority
validation when another peer is asked to consume or accept domain-scoped offers.

Connecting to another peer MUST NOT require either peer to abandon its local
domain.

Joining or forming a peer graph MUST NOT by itself create shared ownership over
the connected peers' domains.

#### Cluster Meaning

A cluster is a peer connectivity/session graph. It MAY be used to
describe peers that know about each other, are connected, are authorized, or
are exchanging data.

A cluster MUST NOT be treated as authoritative for:

- who controls a domain;
- who owns or authored spatial data;
- authorization to publish data;
- authorization to consume data.

#### Consequences

A peer can consume another peer's spatial data through a direct peer
relationship without declaring its own local domain. The peers do not need to
merge their domains or share a common runtime authority.

Failure of one peer SHOULD affect that peer's served domains and peer
relationships only; it SHOULD NOT invalidate unrelated domains.

### RFC-0008: Served Domain Set (To Fill)

Define how a peer records which remote domains are accepted for one peer
relationship.

A served domain set is computed from the remote peer's declared domains after
domain declaration and delegation validation. It is used to decide which offers
the remote peer may expose in that relationship.

A peer relationship MAY have an empty served domain set when the remote peer is
only consuming local offers or when none of its declared domains are accepted.

- declared domains presented during handshake;
- validation result for each declared domain;
- whether a peer may add or remove served domains after handshake;
- whether offers for domains outside the served domain set are rejected,
  ignored, or treated as degraded;
- how served-domain changes affect existing offers, gets, and subscriptions;
- diagnostics for rejected or degraded served domains.

### RFC-0009: Private And Discoverable Peers

#### Requirement

The SDK MUST support both private and discoverable peers.

A discoverable peer registers presence through Discovery or an equivalent
index.

A private peer does not register presence in Discovery but can still:

- dial a discoverable peer;
- be dialed through explicit configuration;
- participate in authorized peer-to-peer exchange once connected.

#### Consequences

A Discovery query MUST NOT be used to prove that a private peer does not exist.

Peer authorization MUST NOT depend solely on whether the peer appeared in
Discovery.

## Discovery And Reachability

### RFC-0010: Discovery Is Optional Entrypoint Rendezvous

#### Requirement

A peer MUST NOT be required to register with Discovery merely to use SDK
networking or to connect to another peer.

A peer MAY register with Discovery when it wants to be discoverable by other
peers.

A peer that does not register with Discovery MAY still connect to other peers
through manual configuration, invitation, direct address exchange, or another
discovery mechanism.

#### Discovery Authority

Discovery MUST be treated as rendezvous/presence infrastructure unless a later
RFC explicitly expands its authority.

Discovery MUST NOT be treated as authoritative for:

- who controls a domain;
- who owns or authored spatial data;
- cluster membership;
- the complete set of peers, including private or non-advertised peers;
- authorization to consume or publish data.

#### Discovery Records

A Discovery record SHOULD answer:

- what domain is being advertised;
- how a peer can dial it;
- coarse, non-authoritative metadata about data types that may be available;
- how fresh the advertisement is.

A Discovery record MUST NOT be treated as an authoritative offer catalog.

A Discovery record MAY advertise one or more entrypoints into a peer graph.

A Discovery record MUST NOT be assumed to list every peer in that graph.

A Discovery record MAY be stale until its freshness window expires or the
advertising peer refreshes, updates, or removes it.

Discovery SHOULD attach freshness metadata to each record, such as `expires_at`,
`ttl`, `last_seen_at`, or an equivalent value.

Discovery SHOULD expire records that are not refreshed within their freshness
window.

Stale or expired Discovery data MUST NOT invalidate existing peer-to-peer
connections by itself.

#### Consequences

Existing peer relationships SHOULD continue when Discovery is temporarily
unavailable, assuming the underlying peer-to-peer transport remains healthy.

SDK status/diagnostics SHOULD distinguish "Discovery presence degraded" from
"peer relationship degraded".

### RFC-0011: Discovery Record Shape (To Fill)

Define the concrete Discovery advertisement:

- domain id and optional display label;
- peer id and dialable advertised addresses;
- freshness fields such as `ttl`, `expires_at`, or `last_seen_at`;
- coarse, non-authoritative data-type hints;
- refresh, update, remove, and expiry behavior.

The record shape should preserve entrypoint advertisement semantics and avoid
becoming an authoritative offer catalog.

### RFC-0012: Discovery Data-Type Hints (To Fill)

Define the coarse data-type hints allowed in Discovery records:

- vocabulary for baseline hints;
- how hints differ from offers;
- whether hints are free-form, registered, or both;
- freshness behavior for hints;
- how clients should treat missing, stale, or unsupported hints.

### RFC-0013: Listen Addresses And Advertised Addresses Are Different

#### Requirement

The SDK MUST distinguish listen addresses from advertised addresses.

- A listen address is where the local network runtime binds.
- An advertised address is what another peer should dial.

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

#### Discovery Interaction

If a peer registers with Discovery, the registered dial addresses SHOULD be
dialable by the intended peers or SHOULD be explicit relay-mediated addresses.

#### Consequences

Apps SHOULD expose listen and advertised address configuration separately.

SDK diagnostics SHOULD report the final advertised address set and identify
whether each address was auto-detected, operator-supplied, or relay-mediated.

### RFC-0014: Relay Is Connectivity, Not Authority

#### Requirement

Relay support MAY be used to establish peer-to-peer connectivity when direct
dialing fails or is unavailable.

Relay support MUST NOT change:

- who controls a domain;
- peer authorization;
- who owns or authored spatial data;
- offer, get, subscribe, stream, or resource semantics.

#### Consequences

A relay-mediated connection MUST be treated as a transport path to the same
remote peer id, not as a different authority model.

Discovery MAY advertise relay-mediated multiaddrs when direct addresses are not
sufficient.

## Connection Lifecycle

### RFC-0015: Peer Handshake (To Fill)

Define the first exchange after dialing:

- peer id and peer binding;
- declared domains, domain declarations, and delegations, when the peer claims
  to serve domains;
- authority-chain validation result;
- accepted served domain set;
- supported protocol versions;
- authorization material;
- offer-catalog fetch path;
- liveness/status initialization.

### RFC-0016: Authorization Model (To Fill)

Define the pragmatic authorization model:

- open/trusted-lab mode;
- allowlist by peer id or wallet public key;
- invite token or signed challenge, if needed;
- per-offer policy hooks;
- which parts are required in the baseline and which are pluggable hardening.

### RFC-0017: Peer Connectivity State Is Tracked Per Remote Peer

#### Requirement

A peer SHOULD track connectivity and readiness state independently for each
remote peer.

Failure of one peer relationship MUST NOT force unrelated peer relationships to
restart or become invalid.

#### Candidate State Model

The following states are non-normative names, but the SDK SHOULD expose
equivalent diagnostic information:

- unknown;
- discovered;
- configured;
- dialing;
- connected;
- authorized;
- loading offers;
- ready;
- degraded;
- lost.

#### Consequences

A peer losing connectivity to one remote peer SHOULD NOT drop unrelated peer
connections.

A peer exiting SHOULD make that peer unavailable to other peers. It SHOULD NOT
by itself invalidate unrelated peer relationships or domains.

### RFC-0018: Peer Graph Hints (To Fill)

Define how a peer shares additional peer candidates after connection:

- whether learned peers are dialed automatically or surfaced as candidates;
- what metadata can be shared;
- whether a peer may hide known peers;
- how the exchange avoids becoming authoritative membership;
- whether DHT-style peer discovery is in scope for this baseline.

The baseline default should treat learned peers as non-authoritative candidate
dial targets or offer sources.

## Spatial Data Exchange

### RFC-0019: Peers Exchange Spatial Data With Offer / Get / Subscribe

#### Requirement

Each peer SHOULD maintain local spatial state for the domains it serves.

After discovery/configuration and authorization, peers SHOULD exchange spatial
data peer-to-peer.

A peer MAY choose not to expose spatial data, or MAY expose only a subset of
its spatial data according to local policy.

A peer that only consumes remote offers is not required to expose offers or
declare a local domain.

The minimum baseline exchange shape is:

- `Offer`: a peer advertises named and typed spatial data it can share now.
- `Get`: a peer fetches an offered data item once.
- `Subscribe`: a peer receives ongoing updates from an offer.

Discovery MAY help a peer find how to dial into a peer graph or cluster, and
MAY include coarse, non-authoritative summary metadata about the kinds of data
that may be available there.

A peer that intends to consume spatial data SHOULD fetch offers from remote
peers after connecting and authorizing.

Discovery MUST NOT be required as the transport for spatial data exchange,
and MUST NOT be treated as the authoritative offer registry.

#### Offers

An offer is a connected peer's declaration of one named and typed data item it
is willing to serve.

Offer ids are scoped to the producing peer's served domain. They identify data
the producer exposes from that domain, not global network objects.

An offer SHOULD provide enough information for a consumer to decide whether it
can use the data and whether to fetch it once or subscribe to it:

- name and/or id;
- data kind;
- payload or schema type and version;
- supported access mode: get, subscribe, or both;
- spatial and temporal references needed to interpret the data, when relevant;
- freshness or availability status.

An offer MUST NOT by itself be treated as proof of authority, correctness, or
trustworthiness. It is a reference to data exposed from a domain.

#### Get

`Get` fetches an offered data item once.

Get is for finite responses such as snapshots, descriptors, registry entries,
transform edges, log ranges, or map fragments.

A failed `Get` SHOULD explain whether the offer was unknown, unauthorized,
stale, unavailable, unsupported, or failed at the transport/protocol layer.

#### Subscribe

`Subscribe` SHOULD receive live updates from an offered data item. Examples
include a camera stream, point-cloud stream, pose stream, audio stream, or
future live map updates.

A subscription failure SHOULD explain whether the offer was unknown,
unauthorized, stale, unavailable, unsupported, or failed at the
transport/protocol layer.

#### Current Implementation Mapping

Current `/auki/resources/0.0.1` resource rows and `/auki/stream/0.1.0` typed
streams are implementation examples.

This RFC does not require those protocol names or exact wire shapes to be the
final Offer / Get / Subscribe contract.

#### Consequences

The SDK SHOULD support a peer learning what another peer can share by name or
type before opening a stream or fetching data.

### RFC-0020: Offer Catalog (To Fill)

Define the concrete offer-catalog protocol:

- request and response shape;
- offer id/name scope;
- domain id scope;
- authority reference to the served domain set;
- data-kind vocabulary;
- payload/schema versioning;
- get/subscribe support flags;
- frame and clock references;
- freshness and availability;
- offer removal and update behavior;
- error shape.

This is the likely replacement or evolution path for `/auki/resources/0.0.1`.

### RFC-0021: Offer Domain Scope And Authority (To Fill)

Define how an offer is tied to a served domain:

- required domain id field;
- whether one offer may reference multiple domains;
- how an offer references delegation or served-domain validation;
- behavior when the served domain becomes rejected, expired, or removed;
- how consumers distinguish producer-declared metadata from verified authority.

### RFC-0022: Spatial Message Envelope (To Fill)

Define common metadata for spatial data messages:

- producing peer id;
- domain id;
- offer id;
- payload/schema type and version;
- frame and clock references when spatial/temporal interpretation is needed;
- freshness or sequence metadata;
- error and end-of-stream metadata shared by get and subscribe paths.

### RFC-0023: Get (To Fill)

Define one-shot fetch semantics:

- request by offer id;
- optional parameters for ranges or small filters;
- maximum response size and chunking rules;
- snapshot consistency;
- stale-offer behavior;
- error shape.

The first implementation should keep this narrow: descriptors, registry
entries, transform edges, small snapshots, and possibly log ranges.

### RFC-0024: Subscribe (To Fill)

Define live update semantics:

- subscribe by offer id;
- start response or manifest shape;
- frame/message envelope;
- end and error reasons;
- backpressure or drop policy;
- reconnect behavior;
- payload compatibility rules.

This is the likely replacement or evolution path for `/auki/stream/0.1.0`.

### RFC-0025: Minimum Offer Kinds (To Fill)

Choose the first offer kinds for implementation. Candidate set:

- `sensor_stream`;
- `transform_edge`;
- `pose_stream` or `pose_log_range`;
- `registry_entry`.

Maps, generic spatial query, payment, and booking should stay out of the first
iteration unless a concrete milestone requires them.

## Compatibility And Observability

### RFC-0026: Protocol Versions Are Compatibility Contracts

#### Requirement

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
- SHOULD include compatibility tests for any previously accepted shape.

Incompatible wire changes MUST use a new protocol ID.

#### Example

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

### RFC-0027: Observability Must Explain State Transitions

#### Requirement

SDK diagnostics MUST make core lifecycle state explainable without noisy
per-frame logs.

Diagnostics SHOULD answer:

- whether this peer is discoverable;
- what it is advertising;
- which local domain it serves or manages;
- which peers are known;
- how each peer was learned;
- whether each peer is dialable;
- whether each peer is connected;
- whether each peer is authorized;
- what offers each peer claims it can share;
- why a peer became degraded or lost;
- whether Discovery is degraded independently from peer connectivity.

#### Consequences

Heartbeat-frame logs, stream-frame logs, and repeated dial retry logs SHOULD be
rate-limited or omitted by default.

State transitions and failures SHOULD be logged once with enough context to
debug the lifecycle.

### RFC-0028: Status And Observability API (To Fill)

Define the concrete status surface:

- local domain id and domain declaration state;
- served domain set and validation state;
- Discovery advertisement state;
- known peers and how they were learned;
- per-peer lifecycle state;
- loaded offers and their served-domain scope;
- active gets/subscriptions;
- last failure reason per peer and per offer.
