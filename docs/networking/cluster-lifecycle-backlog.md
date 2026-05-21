# Cluster Lifecycle RFC Backlog

Status: non-normative working backlog.

Owner: TBD.

Last updated: 2026-05-21.

Related protocol spec:
[`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md).

Related glossary:
[`glossary.md`](glossary.md).

## Purpose

Track the remaining work needed before a clean-room SDK implementer can build
the currently specified v1 protocol from the spec alone.

The protocol baseline lives in
[`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md). This backlog is
not normative.

## Implementability Verdict

Authority, identity, domain validation, handshake message shape, offer objects,
spatial envelopes, and status objects are implementable now.

The currently specified v1 protocol is not fully implementable end to end yet.
The blocker is the post-handshake path layer: Offer Catalog, Get, and Subscribe
have object shapes, but Get and Subscribe do not yet have concrete protocol IDs
or path-binding rules, and common JSON framing/exchange mechanics are not
mechanical enough.

Configured/private peer-to-peer connectivity is the currently specified path.
Concrete Discovery records are v1 To Fill work and do not block that path.

## Work Order

1. Define post-handshake path binding and framing.
2. Define registry-reference hash format.
3. Tighten deterministic failure mapping where interop needs exact outcomes.
4. Tighten payload-type matching.
5. Decide minimum offer-kind payload semantics.
6. Re-run final consistency and implementability review.
7. Start Interop/Test work.

## Start Here: Spec Blockers

### P0-1: Post-Handshake Path Binding And Framing

Slots:

- `RFC-0002: V1 JSON Wire Conventions`
- `RFC-0019: Peer Handshake`
- `RFC-0024: Offer Catalog`
- `RFC-0029: Get`
- `RFC-0030: Subscribe`
- `RFC-0032: Protocol Versions Are Compatibility Contracts`

Problem:

The spec defines the handshake protocol ID and offer-catalog protocol ID, but
not the concrete Get/Subscribe protocol IDs or path descriptors. It also does
not fully define JSON object framing and message ordering for lifecycle paths.

Patch:

- Define the JSON object framing rule for lifecycle, Offer Catalog, Get, and
  Subscribe.
- Define how the handshake exchange runs over
  `/auki/cluster-lifecycle/0.0.1`.
- Define Offer Catalog request/response exchange over
  `/auki/offer-catalog/0.0.1`.
- Define fixed v1 protocol IDs or offer-declared path descriptors for Get and
  Subscribe.
- Define Get request/response ordering.
- Define Subscribe request, accept, reject, data, end, and close ordering.
- Define where structured errors appear when a path fails.
- Keep current SDK protocol paths outside the normative spec.

Done when:

- A clean-room implementer can handshake, fetch offers, run Get, and run
  Subscribe without reading SDK code.
- An offer that advertises `get` or `subscribe` gives the consumer enough
  information to open the correct path.

### P0-2: Registry Reference Hash Format

Slots:

- `RFC-0024: Offer Catalog`
- `RFC-0027: Spatial Message Envelope`

Problem:

Registry references include `hash`, and inline `canonical_json` verification
depends on it, but the hash algorithm and string encoding are not defined.

Patch:

- Define the v1 registry-reference hash algorithm.
- Define the v1 hash string encoding.
- State exactly what bytes are hashed for `canonical_json`.

Done when:

- Two implementations can verify the same inline registry entry and produce the
  same `hash`.
- A consumer can reject a mismatched `canonical_json` deterministically.

## Should Fix Before V1

### P1-1: Deterministic Failure Mapping

Slots:

- `RFC-0010: Failure Code Registry`
- `RFC-0019: Peer Handshake`
- `RFC-0024: Offer Catalog`
- `RFC-0027: Spatial Message Envelope`
- `RFC-0028: Get And Subscribe Common Path Rules`
- `RFC-0029: Get`
- `RFC-0030: Subscribe`
- `RFC-0034: Status And Observability API`

Patch:

- Decide which malformed, unsupported, unauthorized, oversized, and stale cases
  need exact failure codes.
- Upgrade only those mappings that must be deterministic for interop.
- Keep advisory diagnostics advisory.

Done when:

- Common bad inputs produce predictable failure codes without forcing every
  diagnostic path to be normative.

### P1-2: Payload-Type Matching

Slots:

- `RFC-0024: Offer Catalog`
- `RFC-0027: Spatial Message Envelope`
- `RFC-0028: Get And Subscribe Common Path Rules`
- `RFC-0029: Get`
- `RFC-0030: Subscribe`

Patch:

- Define whether `accepted_payload_types` matches `payload.type` by exact string
  equality.
- Define how responders choose a payload type when more than one is possible.
- Clarify how Get response payloads and Subscribe data-message payloads relate
  to offer and Subscribe accept payload descriptors.

Done when:

- A responder can decide whether it can satisfy `accepted_payload_types`.
- A receiver can reject an unexpected payload family deterministically.

### P1-3: Minimum Offer-Kind Payload Semantics

Slot:

- `RFC-0031: Minimum Offer Kinds`

Patch:

- Decide whether `sensor_stream`, `transform_edge`, and `registry_entry` need
  minimum v1 payload schemas.
- If they do not, state that payload schemas are offer-defined or
  application-defined unless a later kind RFC defines them.
- Keep large maps, log ranges, replay, resume, and large object transfer out of
  the current baseline.

Done when:

- A clean-room implementer knows whether to implement concrete payload schemas
  or only the transport and envelope layer.

### P1-4: Final Consistency Review

Slots:

- [`cluster-lifecycle-specs.md`](cluster-lifecycle-specs.md)
- [`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md)
- [`glossary.md`](glossary.md)

Patch:

- Re-run implementability review after P0/P1 patches.
- Check stale owner names, RFC numbers, failure-code references, glossary terms,
  and To Fill boundaries.
- Confirm the backlog only lists unresolved work.

Done when:

- No required currently specified v1 path is blocked by a To Fill section.
- No implementation-specific SDK path is required by the normative spec.
- A clean-room implementer can build configured/private peer-to-peer v1 from
  the spec alone.

## Parked V1 To Fill Work

These are in v1 scope but do not block the currently specified
configured/private peer-to-peer path.

### Dynamic Served-Domain Updates

Slot: `RFC-0012: Served Domain Set`, `Dynamic Updates (To Fill)`.

Current baseline: served-domain changes require reconnect or fresh handshake.

Fill later: update message shape, validation trigger, authority-chain reuse,
new delegation scope if needed, cached-offer behavior, active-subscription
behavior, and failure mapping.

### Discovery Record Shape

Slot: `RFC-0015: Discovery Record Shape`.

Current baseline: Discovery is optional rendezvous/presence infrastructure.
Configured/private peer-to-peer connectivity does not require a concrete
Discovery record.

Fill later: concrete fields, dial address semantics, relay semantics, freshness
and expiry behavior, and non-authoritative metadata boundaries.

### Discovery Data-Type Hints

Slot: `RFC-0016: Discovery Data-Type Hints`.

Fill later: hint vocabulary, relationship between hints and offers, and
handling for missing, stale, or unsupported hints.

### Peer Graph Hints

Slot: `RFC-0022: Peer Graph Hints`.

Fill later: candidate-peer sharing shape, candidate handling rules, and
non-authoritative membership boundaries.

### Concrete SDK NTP / Clock-Sync Protocol

Slot: `RFC-0035: Time And Clock Semantics` plus a future concrete clock-sync
message RFC.

Current baseline: `RFC-0035` owns timestamp and clock semantics. A concrete
clock-sync protocol is not required for Offer, Get, or Subscribe.

Fill later: message flow, fields, offset calculation, delay calculation,
sampling, retry, failure behavior, status fields, and any needed failure codes.

## Interop/Test Work

Do this after the blocker patches and final consistency review.

### Compatibility Fixtures And Test Vectors

Create fixtures and vectors for:

- JSON wire conventions and signed-object canonicalization;
- peer bindings, domain declarations, domain delegations, and domain id
  derivation;
- Offer Catalog, Get, Subscribe, envelopes, errors, status snapshots, and time
  fields;
- additive fields, removed fields, renamed fields, required fields, unknown
  fields, and semantic changes.

### Validation Scenarios And Failure Paths

Create success and failure transcripts for:

- Park finds one robot;
- Park finds many robots;
- Robot exists without Park;
- private peer connects to discoverable peer;
- peer learns additional peers after entrypoint dial;
- peer fetches offers, performs Get, and performs Subscribe.

Each scenario should include expected served-domain-set result, offer usability
result, Get or Subscribe behavior, lifecycle state, status fields, and stable
failure codes.

## Later / Product-Scope Work

Keep these parked unless product scope pulls them forward:

- DHT-style discovery.
- Relay milestone expansion beyond `RFC-0017` and `RFC-0018`.
- Future offer kinds beyond the `RFC-0031` minimum set, such as `pose_stream`,
  `pose_log_range`, `time_transform`, `detection_stream`, `map_fragment`, and
  `spatial_query`.
- Chunking, replay, resume, reliable history, large object transfer, and map
  queries.
- Non-normative migration notes mapping current SDK protocol paths such as
  `/auki/resources/0.0.1`, `/auki/registries/0.0.1`, and
  `/auki/stream/0.1.0` to the final v1 contract.

## Recently Solved

- Minimal handshake schema is specified in `RFC-0019`.
- Discovery no longer blocks configured/private peer-to-peer connectivity.
- To Fill sections have been cleaned of normative protocol requirements.
- Time and clock semantics are owned by `RFC-0035`.
- Stale owner references and stale RFC slots have been repaired.
- Size-limit semantics are normalized across `RFC-0028`, `RFC-0029`, and
  `RFC-0030`.
- Future offer kinds are non-normative planning context.
- Current SDK protocol paths have been removed from the normative spec.
- Glossary alignment was expanded for current recurring terms.
