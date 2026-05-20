# Cluster Lifecycle RFC Backlog

Status: non-normative working backlog.

Owner: TBD.

Last updated: 2026-05-20.

Related protocol spec:
[`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track the gaps that must be resolved before the first minimal peer-to-peer
cluster protocol can be implemented confidently.

The protocol baseline lives in
[`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md). This backlog is
not normative.

## Current Baseline

The current baseline is a bootstrapping protocol: peers use wallet-bound
runtime identities, declare domains, discover or configure reachable peers,
authorize connections, and exchange spatial data through Offer / Get /
Subscribe.

## Gaps To Resolve

### Identity And Domain Validation

Spec slots:

- `RFC-0002: Peer Binding Schema`
- `RFC-0004: Domain Declaration Schema`
- `RFC-0005: Domain Delegation Schema`
- `RFC-0006: Authority Chain Validation`
- `RFC-0008: Served Domain Set`

Open questions:

- What are the concrete Peer Binding, Domain Declaration, and Delegation wire
  shapes?
- How does handshake validate the authority chain and produce the set of
  domains the remote peer is accepted to serve?
- How do domain revocation, delegation replacement, and delegation expiry work?
- Which failures are invalid identity/domain failures versus authorization
  failures?

### Discovery And Reachability

Spec slots:

- `RFC-0011: Discovery Record Shape`
- `RFC-0012: Discovery Data-Type Hints`
- `RFC-0018: Peer Graph Hints`

Open questions:

- What is the concrete Discovery record shape?
- Which coarse data-type hints may Discovery publish?
- How should clients treat missing, stale, or unsupported hints?
- Is DHT-style peer discovery in scope for this baseline?
- Is relay required for the next production milestone?

### Authorization

Spec slot:

- `RFC-0016: Authorization Model`

Open questions:

- What authorization model is sufficient for trusted lab deployments?
- Which defaults are open, allowlist-based, invite-based, or app-specific?
- Are per-offer policy hooks in the baseline or a later hardening layer?

### Spatial Data Exchange

Spec slots:

- `RFC-0020: Offer Catalog`
- `RFC-0021: Offer Domain Scope And Authority`
- `RFC-0022: Spatial Message Envelope`
- `RFC-0023: Get`
- `RFC-0024: Subscribe`
- `RFC-0025: Minimum Offer Kinds`

Open questions:

- What is the exact offer-catalog wire shape and protocol id?
- How are offers scoped to served domains?
- What common spatial message envelope do Get and Subscribe share?
- What offer kinds are required for the next Park/robot milestone?
- What are the baseline backpressure, stream drop, reconnect, and stale-offer
  behaviors?
- What should replace or deprecate `/auki/sensors/0.0.1`, if anything?

### Compatibility And Observability

Spec slot:

- `RFC-0028: Status And Observability API`

Open questions:

- What status API should expose peer, domain, and offer lifecycle state?
- Which diagnostics are required for served-domain validation, offer loading,
  Get, and Subscribe failures?
- Which compatibility tests are required for accepted protocol shapes?

## Validation Scenarios

### Park Finds One Robot

Given a robot advertises an entrypoint, Park should discover or be configured
with that entrypoint, dial it, authorize, fetch offers, and subscribe to a
stream.

This must not require Park to register in Discovery.

### Park Finds Many Robots

Given several robots are discoverable or configured, Park should track each
peer relationship independently.

One robot failure must not affect other robot relationships.

### Robot Exists Without Park

Given Park is offline, a robot should continue serving its declared domain and,
if configured, advertising itself.

### Private Peer Connects To Discoverable Peer

Given a peer is not registered with Discovery but knows a discoverable peer's
address, it should be able to connect if authorized.

Authorization must not depend solely on Discovery presence.

### Peer Learns Additional Peers After Entrypoint Dial

Given a peer dials one entrypoint, it may learn about additional peers through
peer-to-peer exchange.

Learned peers are candidate dial targets or offer sources, not authoritative
membership.

### Offer / Get / Subscribe

Given two connected and authorized peers, Peer A should fetch Peer B's offers,
choose one, and either fetch a snapshot or subscribe to updates.

The same shape should support live streams, transform edges, pose/path replay,
and future map fragments through peer-to-peer exchange.
