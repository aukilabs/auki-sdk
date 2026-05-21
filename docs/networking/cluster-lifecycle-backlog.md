# Cluster Lifecycle RFC Backlog

Status: non-normative working backlog.

Owner: TBD.

Last updated: 2026-05-21.

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
runtime identities, declare served domains when exposing domain-scoped data,
discover or configure reachable peers, authorize connections, and exchange
spatial data through Offer / Get / Subscribe.

## Suggested Fill Order

1. Fill the end-to-end lifecycle, handshake, authority-chain validation, and
   served-domain-set behavior first.
2. Fill the signed object schemas and test vectors.
3. Fill the authorization model.
4. Fill the offer catalog, spatial message envelope, Get, Subscribe, and
   minimum offer kinds.
5. Fill Discovery record details, data-type hints, peer graph hints, and relay
   milestone scope.
6. Fill compatibility fixtures, observability API, diagnostics, and validation
   scenarios.

## Deliverables To Fill

Use this section as the implementation-ready checklist for the first minimal
protocol. Each deliverable should result in concrete normative text in
[`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md), plus at least one
success example and one failure example when the behavior has observable
failure modes.

### End-To-End Lifecycle Contract

Spec slots:

- `RFC-0006: Authority Chain Validation`
- `RFC-0008: Served Domain Set`
- `RFC-0015: Peer Handshake`
- `RFC-0017: Peer Connectivity State Is Tracked Per Remote Peer`

Deliverables:

- Define the lifecycle state machine from discovered or configured peer to
  ready peer relationship.
- Define the handshake order: protocol negotiation, peer binding, declared
  domains, domain declarations, delegations, authorization material, accepted
  served domain set, offer-catalog path, and liveness/status initialization.
- Define which handshake fields are required, optional, repeatable, or
  extensible.
- Define whether a peer relationship may continue with zero accepted served
  domains, one accepted served domain, or partial acceptance across many
  declared domains.
- Define state transition triggers for `discovered`, `configured`, `dialing`,
  `connected`, `authorized`, `loading offers`, `ready`, `degraded`, and `lost`.
- Define the canonical failure taxonomy shared by handshake, authority-chain
  validation, authorization, offer loading, Get, and Subscribe.

Acceptance checks:

- An implementer can read a handshake transcript and determine whether to keep
  the connection, reject it, degrade it, or continue with partial domains.
- A failed connection produces one stable failure code instead of an ambiguous
  log message.
- One peer relationship can fail without changing the lifecycle state of
  unrelated peer relationships.

### Identity, Domain, And Delegation Schemas

Spec slots:

- `RFC-0002: Peer Binding Schema`
- `RFC-0004: Domain Declaration Schema`
- `RFC-0005: Domain Delegation Schema`
- `RFC-0006: Authority Chain Validation`

Deliverables:

- Define the exact peer-binding wire shape.
- Define the exact domain-declaration wire shape.
- Define the exact delegation wire shape.
- Define canonical signed bytes for each signed object.
- Define wallet public key encoding, peer id encoding, domain id encoding,
  nonce size, timestamp encoding, expiry encoding, and signature encoding.
- Define hash function, domain-separation prefixes, and canonical hash input
  for domain ids.
- Define required fields, optional fields, unknown-field handling, and version
  field behavior.
- Define delegation scopes such as advertise, serve, update, consume, or other
  baseline actions.
- Define delegation validity windows, expiry handling, replacement behavior,
  and any baseline revocation mechanism.
- Define the future external-binding authority model for transferable
  NFT-backed domains, including controller resolution and chain-finality
  assumptions.
- Provide test vectors for valid and invalid peer bindings, domain
  declarations, delegations, and domain id derivation.

Acceptance checks:

- Two independent implementations can produce the same domain id from the same
  owner public key and nonce.
- A receiver can validate or reject every signed object without Discovery,
  blockchain access, or an online registry.
- Expired, malformed, wrong-peer, wrong-domain, and wrong-signature examples
  have deterministic failure reasons.

### Served Domain Set And Offer Authority

Spec slots:

- `RFC-0008: Served Domain Set`
- `RFC-0020: Offer Catalog`
- `RFC-0021: Offer Domain Scope And Authority`

Deliverables:

- Define how declared domains become accepted, rejected, or degraded served
  domains for one peer relationship.
- Define direct-owner validation versus delegated-authority validation.
- Define whether served domains can be added, removed, refreshed, or replaced
  after the initial handshake.
- Define what happens to offers, active Gets, and active subscriptions when a
  served domain expires, is removed, is replaced, or becomes invalid.
- Define whether offers outside the served domain set are rejected, ignored, or
  surfaced as degraded diagnostics.
- Define whether one offer may reference multiple domains and, if so, how
  authority is validated for each referenced domain.
- Define how consumers distinguish verified authority fields from
  producer-declared metadata.

Acceptance checks:

- An offer cannot become usable unless its domain is in the accepted served
  domain set for that peer relationship.
- A delegation expiry produces a deterministic result for loaded offers and
  active subscriptions.
- Partial acceptance of declared domains is either explicitly supported or
  explicitly forbidden.

### Discovery And Reachability

Spec slots:

- `RFC-0011: Discovery Record Shape`
- `RFC-0012: Discovery Data-Type Hints`
- `RFC-0013: Listen Addresses And Advertised Addresses Are Different`
- `RFC-0014: Relay Is Connectivity, Not Authority`
- `RFC-0018: Peer Graph Hints`

Deliverables:

- Define the exact Discovery record wire shape.
- Define required and optional Discovery fields: domain id, display label, peer
  id, advertised addresses, freshness fields, data-type hints, and entrypoint
  hints.
- Define refresh, update, remove, expiry, and stale-record behavior.
- Define the baseline data-type hint vocabulary and whether custom hints are
  allowed.
- Define how clients treat missing, stale, unknown, or unsupported hints.
- Define how peers share additional peer candidates after entrypoint dial.
- Decide whether DHT-style peer discovery is in scope for the first baseline.
- Decide whether relay support is required for the next production milestone
  or remains optional.

Acceptance checks:

- A private peer can connect through explicit configuration without registering
  in Discovery.
- An expired Discovery record does not invalidate an already healthy
  peer-to-peer connection.
- A client can explain whether a dial failure came from Discovery freshness,
  advertised address reachability, relay availability, or authorization.

### Authorization Model

Spec slot:

- `RFC-0016: Authorization Model`

Deliverables:

- Harden authorization beyond the experimental baseline modes: `all`,
  `whitelisted-only`, and `app-policy`.
- Define invite token or signed challenge behavior, if needed.
- Define whether authorization is peer-level, domain-level, offer-level, or a
  combination.
- Define whether per-offer policy hooks are in the baseline or reserved for a
  later hardening layer.
- Define authorization failure codes separately from identity, domain, and
  transport failure codes.

Acceptance checks:

- Trusted lab deployments have a minimal path that does not require external
  infrastructure.
- A peer rejected by policy is distinguishable from a peer with invalid
  identity material.
- Authorization never depends solely on Discovery presence.

### Spatial Data Exchange

Spec slots:

- `RFC-0020: Offer Catalog`
- `RFC-0021: Offer Domain Scope And Authority`
- `RFC-0022: Spatial Message Envelope`
- `RFC-0023: Get`
- `RFC-0024: Subscribe`
- `RFC-0025: Minimum Offer Kinds`

Deliverables:

- Define protocol ids for offer catalog, Get, Subscribe, and any shared
  envelope version.
- Define the offer-catalog request, response, update, removal, and error
  shapes.
- Define offer id scope, domain id scope, data-kind vocabulary,
  payload/schema versioning, access-mode flags, frame references, clock
  references, freshness, and availability status.
- Define the common spatial message envelope shared by Get responses and
  Subscribe updates.
- Define Get request parameters, maximum response size, chunking rules,
  snapshot consistency, stale-offer behavior, and error shape.
- Define Subscribe start response, update message shape, sequencing,
  end-of-stream reasons, error reasons, backpressure or drop policy,
  reconnect behavior, and payload compatibility rules.
- Choose the minimum offer kinds required for the next Park/robot milestone.
- Decide what replaces, preserves, or deprecates `/auki/resources/0.0.1`,
  `/auki/stream/0.1.0`, and `/auki/sensors/0.0.1`.

Acceptance checks:

- A consumer can fetch offers before opening a stream.
- A consumer can distinguish unknown offer, unauthorized offer, stale offer,
  unavailable producer, unsupported payload, and transport/protocol failure.
- Subscribe behavior is deterministic under slow consumers, dropped messages,
  reconnects, and producer-side stream end.

### Compatibility, Fixtures, And Observability

Spec slots:

- `RFC-0026: Protocol Versions Are Compatibility Contracts`
- `RFC-0027: Observability Must Explain State Transitions`
- `RFC-0028: Status And Observability API`

Deliverables:

- Define compatibility fixtures for every accepted wire shape.
- Define locked field-name tests for signed objects and protocol messages.
- Define upgrade rules for additive fields, removed fields, renamed fields,
  required fields, unknown fields, and semantic changes.
- Define the concrete status API for local domain state, Discovery state, known
  peers, per-peer lifecycle state, served-domain validation, loaded offers,
  active Gets, active subscriptions, and last failure reasons.
- Define which lifecycle transitions must emit diagnostics and which
  high-volume events should be rate-limited or omitted by default.
- Define a stable set of diagnostic reason codes shared by logs, status APIs,
  and conformance tests.

Acceptance checks:

- A compatibility test can prove that old valid messages still decode.
- Operators can tell whether a failure is in Discovery, dialing, identity,
  domain validation, authorization, offer loading, Get, or Subscribe.
- Diagnostics expose enough state to debug the Park/robot validation scenarios
  without relying on noisy per-frame logs.

## Validation Scenarios

Each scenario should be filled with:

- initial peer, wallet, domain, Discovery, and authorization configuration;
- expected handshake transcript or high-level message sequence;
- expected served-domain-set result;
- expected offer, Get, and Subscribe behavior when relevant;
- expected lifecycle states and diagnostics;
- at least one failure variant.

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
