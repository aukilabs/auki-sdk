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

## Suggested Finish Order

1. Figure out the existing SDK NTP or clock-sync protocol and decide how it
   maps into the RFC set.
2. Add the missing time-sync RFC text and any needed failure/status fields.
3. Update the glossary so it matches the filled spec language.
4. Add compatibility fixtures, signed-object test vectors, and validation
   scenarios.
5. Run a final AI/expert review pass for consistency, business logic, and RFC
   writing quality.
6. Leave Discovery record shape, Discovery data-type hints, and peer graph
   hints for later unless they become blockers for the first implementation.

## Remaining Deliverables

Use this section as the implementation-ready checklist for work that remains
after the core v1 lifecycle, authority, Offer, Get, Subscribe, and status
sections have been filled.

### SDK NTP / Clock-Sync Protocol

Spec slots:

- New RFC slot, likely near `RFC-0022: Spatial Message Envelope` or after
  `RFC-0025: Minimum Offer Kinds`.
- Existing clock and timestamp references in `RFC-0020`, `RFC-0022`,
  `RFC-0023`, `RFC-0024`, and `RFC-0028`.

Deliverables:

- Reverse-engineer the current SDK NTP or clock-sync protocol from the SDK.
- Document the message flow, request/response fields, timestamp fields, offset
  calculation, delay calculation, sampling behavior, retry behavior, and failure
  handling.
- Decide whether the protocol is required for v1, optional diagnostics, or a
  later temporal-accuracy layer.
- Define how clock-sync results relate to `timestamp_ns`, `generated_at`, clock
  registry references, Subscribe sequencing, and status diagnostics.
- Define whether clock-sync state is local-only diagnostic state or something
  peers may advertise.
- Define any needed failure codes, status fields, and glossary terms.
- Preserve the rule that clock sync does not prove domain authority, data
  correctness, or timestamp truth by itself.

Acceptance checks:

- An implementer can reproduce the SDK clock-sync behavior from the RFC text or
  intentionally replace it with a documented v1 behavior.
- A receiver can distinguish producer event time, producer wall-clock metadata,
  local receive time, and estimated peer clock offset.
- Clock-sync failure is observable without making Offer / Get / Subscribe
  unusable unless local policy requires time synchronization.

### Glossary Update

Spec slots:

- [`glossary.md`](glossary.md)
- All filled RFC sections in [`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md)

Deliverables:

- Add terms introduced or sharpened during the spec pass, including status
  snapshot, failure record, peer binding freshness, domain declaration, domain
  delegation, accepted served domain set, offer catalog, payload descriptor,
  registry reference, Get request, Subscribe accept start result, Subscribe end
  message, sequence gap, and clock-sync terms.
- Remove or rewrite glossary wording that no longer matches the filled RFCs.
- Keep glossary text descriptive and non-normative.

Acceptance checks:

- Every recurring term in the filled RFCs has one glossary meaning.
- Glossary terms do not add requirements that are missing from the RFC text.

### Compatibility Fixtures And Test Vectors

Spec slots:

- `RFC-0002: Peer Binding Schema`
- `RFC-0004: Domain Declaration Schema`
- `RFC-0005: Domain Delegation Schema`
- `RFC-0020: Offer Catalog`
- `RFC-0022: Spatial Message Envelope`
- `RFC-0023: Get`
- `RFC-0024: Subscribe`
- `RFC-0028: Status And Observability API`

Deliverables:

- Define compatibility fixtures for every accepted wire shape.
- Define locked field-name tests for signed objects and protocol messages.
- Provide valid and invalid vectors for peer bindings, domain declarations,
  delegations, domain id derivation, Offer Catalog, Get, Subscribe, envelopes,
  errors, and status snapshots.
- Define upgrade rules for additive fields, removed fields, renamed fields,
  required fields, unknown fields, and semantic changes.

Acceptance checks:

- A compatibility test can prove that old valid messages still decode.
- Two independent implementations can produce the same domain id from the same
  owner public key and nonce.
- Expired, malformed, wrong-peer, wrong-domain, wrong-signature, unsupported
  kind, unsupported payload, and stale-offer examples produce deterministic
  failure codes.

### Validation Scenarios And Failure Paths

Spec slots:

- All filled lifecycle and spatial-data RFCs.

Deliverables:

- Turn the scenarios below into concrete transcripts or high-level message
  sequences.
- Add at least one success and one failure variant per scenario.
- Include expected served-domain-set result, offer usability result, Get or
  Subscribe behavior, lifecycle state, status snapshot fields, and stable
  failure codes.

Acceptance checks:

- Operators can tell whether a failure is in Discovery, dialing, identity,
  domain validation, authorization, offer loading, Get, Subscribe, envelope
  validation, payload validation, clock sync, or local policy.
- Diagnostics expose enough state to debug the Park/robot scenarios without
  relying on noisy per-frame logs.

### Final AI / Expert Review

Spec slots:

- [`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md)
- [`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md)
- [`glossary.md`](glossary.md)

Deliverables:

- Run a final consistency review across the spec, backlog, and glossary.
- Check RFC writing quality: normative keyword use, stable terminology, field
  naming, versioning, failure-code consistency, and duplicate requirements.
- Check business logic: authority boundaries, offline validation, local policy,
  Discovery non-authority, Offer / Get / Subscribe semantics, and clock-sync
  assumptions.
- Check implementability: whether a clean-room implementer can build v1 without
  reading existing SDK code except where explicitly referenced by the NTP task.

Acceptance checks:

- No unresolved duplicate deliverables remain in this backlog.
- Discovery is clearly marked as later work and not a blocker for the core v1
  peer-to-peer spatial exchange.
- The remaining TODOs are either clock-sync, glossary, fixtures, validation
  scenarios, or explicitly later Discovery/peer-graph work.

## Later Deliverables

### Discovery And Reachability

Discovery record shape, Discovery data-type hints, peer graph hints, DHT-style
discovery, and relay milestone scope are later work unless they block the first
implementation.

Spec slots:

- `RFC-0011: Discovery Record Shape`
- `RFC-0012: Discovery Data-Type Hints`
- `RFC-0018: Peer Graph Hints`

Acceptance checks for the later pass:

- A private peer can connect through explicit configuration without registering
  in Discovery.
- An expired Discovery record does not invalidate an already healthy
  peer-to-peer connection.
- A client can explain whether a dial failure came from Discovery freshness,
  advertised address reachability, relay availability, or authorization.

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
