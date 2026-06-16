# Decentralized Peer Discovery RFC Draft

Status: companion protocol/RFC reference; not implemented.

Base branch evidence: `develop` at `1682c38` still ships the legacy HTTP
Discovery/ClusterManager runtime. PR #159 (`matt/docs` at `e1547b91`) is used as
a design seed for authority boundaries and networking RFC structure, not as
merged truth.

Baseline relationship: `baseline.md` now owns the normative discovery candidate
requirements in RFC-0014 through RFC-0016.5. This companion keeps design
rationale, example candidate JSON, libp2p source mapping notes, and open
implementation decisions. If this document conflicts with `baseline.md`,
`baseline.md` is the normative source for the draft implementable baseline.

Source links:

- PR #159 design seed: <https://github.com/aukilabs/auki-sdk/pull/159>
- libp2p Kademlia DHT overview: <https://libp2p.io/docs/kademlia-dht>
- libp2p mDNS overview: <https://libp2p.io/docs/mdns>
- libp2p Identify package description: <https://www.npmjs.com/package/@libp2p/identify>
- libp2p AutoNAT overview: <https://libp2p.io/docs/autonat>
- libp2p Circuit Relay v2 spec: <https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md>
- libp2p hole punching overview: <https://docs.libp2p.io/concepts/nat/hole-punching/>

## 1. Scope And Non-Authority Boundary

This draft defines decentralized peer discovery as a multi-source candidate
pipeline over libp2p-compatible entrypoints. It does not define a centralized
Auki Discovery service and does not make discovery authoritative.

Baseline-aligned requirements, restated from `baseline.md` RFC-0014 through RFC-0016.5:

- Discovery candidates MUST be treated only as candidate dial targets and coarse
  non-authoritative hints.
- Discovery candidates MUST NOT grant cluster membership, trust, domain
  authority, offer authority, policy acceptance, payload correctness, or
  transport success.
- Candidate source, freshness, labels, relay metadata, domain hints, capability
  hints, and data-type hints MUST NOT override identity, domain, offer, or local
  policy validation.
- Authority remains validated after transport connection using the existing
  boundary: transport-authenticated libp2p peer id, wallet-signed peer binding,
  domain declaration/delegation validation, offer policy, and local application
  policy.
- A peer MAY learn candidates from many sources at once and MUST merge them
  through the same validation, cache, dial-policy, retry, and authority boundary
  rules.

Valid candidate entrypoints include:

- configured peers and manual multiaddrs;
- invitation, deep-link, or QR multiaddrs;
- LAN mDNS records;
- persisted peer cache entries;
- Kad-DHT peer routing results;
- DHT provider mappings;
- rendezvous mappings;
- app-bundled bootstrap or relay peers;
- Circuit Relay v2 reservation addresses;
- connected-peer advertisements.

Current-runtime note: `develop` still includes a centralized HTTP Discovery
client and ClusterManager path. This draft treats that runtime as legacy/current
implementation evidence or as one possible optional rendezvous input. It is not
required infrastructure for the target architecture.

## 2. Candidate Advertisement Object

The draft object is named `AukiPeerCandidateV1`.

Open decision: this section is JSON-shaped so validators and vectors can be
implemented mechanically. It is not yet a decision that the final wire encoding
must be normative JSON/JCS; protocol crate work must resolve canonical encoding
before runtime/API implementation.

### 2.1 Object Shape

```json
{
  "type": "auki.peer_candidate.v1",
  "peer_id": "12D3KooWExamplePeerIdText",
  "addrs": [
    "/ip4/203.0.113.10/udp/4001/quic-v1/p2p/12D3KooWExamplePeerIdText"
  ],
  "source": "dht_peer_routing",
  "domain_hints": [
    {
      "domain_id": "u7XlF2wX5Rz8t8vK3YE4dTcLh4PEFIvlY3NWmZRDvbs",
      "display_label": "demo-floor",
      "advertise_authority": {
        "type": "draft-open-decision",
        "material": "base64url-without-padding-if-required-later"
      }
    }
  ],
  "capability_hints": ["offer_catalog", "relay_reservation"],
  "data_type_hints": ["spatial.pose", "sensor.camera.rgb"],
  "observed_at": "2026-06-16T03:30:00Z",
  "expires_at": "2026-06-16T03:40:00Z",
  "ttl_ms": 600000,
  "relay": {
    "kind": "circuit_relay_v2",
    "relay_peer_id": "12D3KooWExampleRelayPeerIdText",
    "reservation_expires_at": "2026-06-16T03:45:00Z",
    "expensive": true
  },
  "reachability": {
    "observed": "public",
    "confidence": "hint"
  }
}
```

### 2.2 Field Requirements

| Field | Requirement |
| --- | --- |
| `type` | REQUIRED string. MUST be exactly `auki.peer_candidate.v1`. |
| `peer_id` | REQUIRED string. MUST parse as a libp2p PeerId text representation. |
| `addrs` | REQUIRED array of multiaddr strings. MAY be empty only when the candidate is hint-only and not directly eligible for dialing. Each dialable address SHOULD contain or be bound to the same `peer_id`; mismatches MUST be rejected or treated as non-dialable hints. |
| `source` | REQUIRED string enum. MUST be one of the source values in section 2.3. |
| `domain_hints` | OPTIONAL array. Each entry names a possibly served domain. Hints are not authority; `serve` authority is still validated during handshake before any accepted served-domain set is created. |
| `capability_hints` | OPTIONAL array of coarse strings. Hints are not offers and MUST NOT replace offer-catalog fetch. |
| `data_type_hints` | OPTIONAL array of coarse strings. Hints are not offers, payload profiles, or policy acceptance. |
| `observed_at` | REQUIRED RFC3339 UTC timestamp for when the candidate was learned or refreshed locally. |
| `expires_at` | OPTIONAL RFC3339 UTC timestamp after which the candidate is not eligible for new dial attempts. |
| `ttl_ms` | OPTIONAL non-negative safe integer. If both `expires_at` and `ttl_ms` are present, validators MUST compute an expiry from `observed_at + ttl_ms` and use the earlier of that value and `expires_at`. |
| `relay` | OPTIONAL object describing relay/reachability metadata. It MUST NOT alter authority. |
| `reachability` | OPTIONAL object for coarse reachability observations. It MUST NOT override local dial policy. |

### 2.3 Source Enum

`source` MUST be one of:

- `configured`
- `invitation`
- `mdns`
- `peer_cache`
- `dht_peer_routing`
- `dht_provider`
- `bootstrap`
- `rendezvous`
- `relay_reservation`
- `connected_peer_advertisement`

Unsupported source values MUST be rejected with
`discovery.source_unsupported`.

### 2.4 Domain Hint Shape

A domain hint object MAY contain:

| Field | Requirement |
| --- | --- |
| `domain_id` | REQUIRED string. MUST parse as the current domain-id encoding, currently base64url without padding over the domain-id bytes used by the authority RFCs. |
| `display_label` | OPTIONAL human-readable string. MUST NOT affect authority, policy, or cache identity. |
| `advertise_authority` | OPTIONAL draft field for advertisement/delegation material. Open decision: humans/protocol owners must decide whether advertise delegation material is required inside candidate/domain hints or only validated at publication/handshake boundaries. |

Even if `advertise_authority` is present, a candidate MUST NOT be accepted as
serving a domain until the post-connection handshake validates serve authority
for that domain.

### 2.5 Cache Key And Replacement

Implementations MUST derive cache identity from at least:

```text
(peer_id, normalized_multiaddr_or_hint_only_marker, source, domain_id_or_none)
```

Where a candidate has multiple addresses, validators MAY store one candidate per
normalized address or one peer/source entry with an address set. Either choice
MUST produce deterministic duplicate detection.

Replacement rules:

- A newer candidate for the same cache key replaces an older candidate when its
  `observed_at` is later and validation succeeds.
- A candidate with a later `expires_at` but an older `observed_at` MUST NOT
  silently replace a newer observation unless local policy explicitly allows
  source-priority replacement.
- Replacement MUST NOT reset backoff for unrelated addresses or sources.
- Cache replacement MAY emit `discovery.cache_replaced` as status/diagnostic
  information; it is not an error.

### 2.6 Size And Count Limits

Implementations MUST define local limits before expensive parsing or signature
work. Recommended draft defaults:

| Limit | Suggested default |
| --- | ---: |
| Candidate JSON body | 16 KiB |
| `addrs` entries | 16 |
| `domain_hints` entries | 16 |
| `capability_hints` entries | 32 |
| `data_type_hints` entries | 32 |
| String field length | 512 bytes unless a field-specific encoding requires otherwise |
| `relay`/`reachability` serialized size | 2 KiB each |
| JSON nesting depth | 16 |

## 3. Validation And Rejection Rules

Validators MUST reject a candidate before it becomes queued/eligible when any of
these conditions hold:

- `type` is missing or not exactly `auki.peer_candidate.v1`;
- `peer_id` is missing or malformed, including malformed multibase/multihash PeerId text;
- `addrs` is missing, not an array, contains malformed multiaddr strings, or is
  empty without being explicitly treated as hint-only;
- a multiaddr contains a different terminal `/p2p/<peer_id>` than the candidate
  `peer_id`, unless local policy stores the address as a non-dialable hint;
- `source` is missing or not in the source enum;
- `observed_at`, `expires_at`, or `ttl_ms` is malformed, outside supported time
  bounds, has an invalid timezone, or implies an already expired candidate;
- a domain hint has malformed `domain_id`, malformed base64url, excessive label
  length, malformed `advertise_authority`, invalid advertise/delegation material,
  or excessive nested material;
- a binary/string field that claims base64url uses padding, alternate alphabet,
  non-canonical spelling, or wrong decoded length;
- size, count, or nesting limits are exceeded;
- the candidate is expired at validation time;
- local dial policy disallows every dialable address;
- the candidate source attempts to override authority or policy.

Dial policy MUST be applied before a candidate becomes eligible for automatic
new dial attempts. Unless explicitly configured, implementations SHOULD reject
or quarantine addresses that are:

- loopback;
- link-local;
- private/local network when the source is not local/configured;
- local-service-only names;
- relay paths marked or inferred as expensive;
- transports disabled by local policy;
- addresses whose peer id binding conflicts with `peer_id`.

Dial-policy rejection uses `discovery.dial_policy_rejected`. A candidate with
multiple addresses MAY remain eligible if at least one address passes policy;
rejected addresses SHOULD be retained only as diagnostics or quarantined hints.

Candidate validation MUST NOT validate final domain serving authority, offer
catalog correctness, trust, or membership. Those failures are owned by the
transport, identity, domain, and offer RFCs after connection.

## 4. Candidate State Machine

Implementations SHOULD model candidate processing with these states:

```text
learned
  -> validated
  -> queued
  -> eligible
  -> dial_attempt
  -> transport_connected
  -> identity_peer_binding_validated
  -> domain_declaration_delegation_validated
  -> accepted_served_domain_set
  -> offer_catalog_fetch
```

Failure and terminal transitions:

```text
learned -> rejected
validated|queued|eligible -> stale
queued|eligible -> evicted
dial_attempt -> dial_failed -> backoff -> queued|stale|evicted
transport_connected -> identity_failed
identity_peer_binding_validated -> domain_validation_failed
accepted_served_domain_set -> offer_catalog_fetch_failed
any non-terminal state -> expired -> stale|evicted
```

State semantics:

- `learned`: raw input from any source. No dial or authority decision exists.
- `validated`: object shape, source, freshness, size/count, and address syntax
  passed.
- `queued`: candidate is stored for scheduling but not yet dialable due to
  backoff, source priority, capacity, network state, or policy.
- `eligible`: at least one address is fresh, allowed by dial policy, and not in
  backoff.
- `dial_attempt`: one or more addresses are being dialed under per-peer,
  per-address, and per-source backoff controls.
- `transport_connected`: libp2p transport connected and peer id should be
  transport-authenticated by libp2p.
- `identity_peer_binding_validated`: wallet-signed peer binding validates the
  peer id and satisfies freshness/local authorization policy.
- `domain_declaration_delegation_validated`: declared domains and required
  delegations validate under the domain authority RFCs.
- `accepted_served_domain_set`: local policy has accepted the domains this peer
  may serve in this peer relationship.
- `offer_catalog_fetch`: the peer's offer catalog is requested only after the
  preceding authority/policy gates needed by the relevant offer RFCs.

A candidate that reaches `transport_connected` but fails identity, domain, or
offer validation MUST NOT be reclassified as authoritative discovery evidence.
It MAY update retry/backoff diagnostics for that peer/address/source.

## 5. Retry, Cache, Expiry, And Persistence

### 5.1 Backoff

Implementations MUST apply bounded backoff to automatic dial attempts. Backoff
SHOULD be scoped by:

```text
(peer_id, normalized_addr, source)
```

Recommended behavior:

- syntax/validation rejection: no retry until a refreshed candidate is learned;
- dial failure: exponential backoff with jitter per peer/address/source;
- identity/domain authority failure: stronger backoff per peer id, independent
  of address, because the peer itself failed authority validation;
- offer-catalog failure: leave discovery backoff unchanged unless the failure is
  transport/path-related;
- manual/configured sources MAY have shorter or user-visible retry controls.

Remote `retryable`, labels, or diagnostics are untrusted hints and MUST NOT
drive uncontrolled retry loops.

### 5.2 Expiry

Expiry is independent of live connections:

- Expired candidates MUST NOT be used for new automatic dial attempts.
- Expiring a candidate MUST NOT invalidate an existing transport connection,
  accepted peer relationship, accepted served-domain set, active subscription,
  or offer relationship by itself.
- Existing relationships remain governed by identity, domain, offer, session,
  authority freshness, and local policy deadlines.
- A refreshed candidate MAY extend future dial eligibility but MUST NOT extend
  authority deadlines for existing relationships.

### 5.3 Peer Cache Persistence

Persistent peer caches SHOULD store only bounded candidate metadata needed for
future bootstrapping:

- `peer_id`;
- normalized allowed addresses;
- source and source priority;
- observed/expiry timestamps;
- coarse non-authoritative hints;
- last failure/status code and capped counters;
- no secrets, bearer tokens, private invitation contents, or raw debug logs.

Caches MUST drop expired candidates on load or mark them stale before use. Cache
load MUST re-run current validators and dial policy because policy and network
context may have changed since persistence.

### 5.4 Refresh, Update, And Remove

- Refresh: a new observation for the same cache key updates freshness and MAY
  replace addresses/hints according to replacement policy.
- Update: a candidate with changed addresses or hints MUST be validated like a
  new candidate. Authority-sensitive changes remain hints only.
- Remove: a source MAY withdraw a candidate. Removal stops future automatic
  dials from that source but MUST NOT tear down existing relationships unless a
  separate authority/policy rule says so.
- Stale: candidates whose expiry elapsed MAY remain in diagnostics briefly but
  MUST be excluded from eligibility.

## 6. Discovery Failure And Status Codes

Discovery/candidate code additions use stable `category.reason` strings:

| Code | Meaning |
| --- | --- |
| `discovery.invalid_candidate` | Candidate object shape, field encoding, multiaddr, PeerId, domain id, base64, time, size, count, or authority-boundary validation failed. |
| `discovery.expired_candidate` | Candidate was expired at validation, queue, cache-load, or dial-scheduling time. |
| `discovery.dial_policy_rejected` | Candidate address was syntactically valid but rejected by local dial policy. |
| `discovery.source_unsupported` | Candidate source was not in the supported source enum or disabled by local policy. |
| `discovery.cache_replaced` | Candidate cache entry was replaced by a fresher valid candidate. Diagnostic/status event, not necessarily an error. |
| `discovery.no_eligible_candidate` | Scheduler found no fresh, policy-allowed, non-backoff candidate for a requested peer/domain/query. |

Ownership boundary:

- transport failures stay in the transport/libp2p failure space;
- peer id or wallet-signed peer-binding failures stay in identity/handshake RFCs;
- domain declaration/delegation failures stay in domain authority RFCs;
- offer-catalog/Get/Subscribe failures stay in offer/data-exchange RFCs.

Discovery code MUST NOT be used to hide or reclassify an authority failure as a
mere candidate failure.

## 7. Interop Test And Vector Requirements

A later protocol implementation MUST include docs-backed vectors for at least:

1. valid JSON candidate with one public direct address;
2. valid hint-only candidate with empty `addrs` and non-authoritative
   `domain_hints`;
3. valid relay candidate containing Circuit Relay v2 metadata;
4. invalid `type`;
5. malformed `peer_id`;
6. malformed multiaddr;
7. address terminal `/p2p` peer id mismatch;
8. malformed timestamp or invalid timezone;
9. unsupported `source`;
10. malformed `domain_id` and malformed base64url in `advertise_authority`;
11. invalid advertise/delegation material shape when `advertise_authority` is
    present;
12. expired `expires_at`;
13. `ttl_ms` expiry earlier than `expires_at`;
14. size/count limit excess;
15. duplicate candidate with older `observed_at` rejected or retained below the
    newer cache entry;
16. duplicate candidate with newer `observed_at` replacing the old entry and
    emitting `discovery.cache_replaced`;
17. dial-policy rejection for loopback/link-local/private/local-service address;
18. dial-policy rejection for expensive relay unless configured;
19. source mapping matrix covering every source enum value;
20. authority boundary case: candidate with domain hint does not create an
    accepted served-domain set before handshake validation;
21. authority boundary case: candidate with capability/data-type hints does not
    create an offer, membership, trust, or policy acceptance;
22. stale candidate does not invalidate an already accepted live relationship.

The vector suite SHOULD include both object-level fixtures and state-machine
transition tests so later implementers do not invent object shape, replacement,
expiry, or failure behavior.

## 8. libp2p Mechanism Mapping

Auki maps libp2p mechanisms into candidate inputs. They are not Auki authority.

| libp2p mechanism | Auki candidate source mapping | Notes |
| --- | --- | --- |
| Configured/bootstrap peers | `configured` or `bootstrap` | App/user/deployment-provided multiaddrs. Eligible only after validation and dial policy. |
| Invitation/deep-link/QR | `invitation` | User-carried bootstrap material. It may be private/sensitive and should not be persisted raw. |
| mDNS | `mdns` | LAN-local peer discovery through multicast DNS records; normally local-network only. |
| Kad-DHT peer routing | `dht_peer_routing` | Peer routing results provide peer ids and addresses. They do not prove Auki authority. |
| DHT provider records | `dht_provider` | Provider lookups can point to peers associated with content/domain/rendezvous keys. They are only candidate hints. |
| Rendezvous | `rendezvous` | Optional rendezvous service/protocol can map interests to peers. A centralized rendezvous server is not required Auki infrastructure and grants no authority. |
| Identify / identify-push | `connected_peer_advertisement` | Connected peers can advertise observed/listen addresses and supported protocols. Treat as hints subject to policy. |
| AutoNAT | reachability metadata | Helps decide whether a peer appears public/private. It does not override dial policy or authority. |
| Circuit Relay v2 reservation | `relay_reservation` | Reservation addresses can make a peer reachable through a relay. Relay is connectivity, not authority. |
| DCUtR / hole punching | reachability metadata or transport path | A connection-upgrade mechanism after candidates/relay paths exist; not an authority source. |
| Peer exchange | `connected_peer_advertisement` or future source | Peers may share more candidates after connection. Shared candidates are still non-authoritative. |

## 9. Centralized Discovery-Service Framing Avoidance

Spec, API, docs, and UI wording SHOULD avoid capital-D `Discovery` as a required
Auki service for this target architecture. Prefer:

- peer candidate pipeline;
- candidate sources;
- rendezvous input;
- bootstrap source;
- peer advertisement;
- reachability hint.

If legacy/current runtime is mentioned, it MUST be framed as current
`auki-network` HTTP Discovery/ClusterManager behavior or as an optional
rendezvous-style input, not as the target architecture or authority layer.

## 10. Protocol Baseline Vs SDK Helper Boundary

`baseline.md` RFC-0014 through RFC-0016.5 define the baseline candidate semantics
when discovery is used: object shape, source enum, validation, rejection, dial
policy, cache/freshness, lifecycle, retry/backoff, failure codes, interop
vectors, and mechanism mapping boundaries.

Before later SDK/runtime/helper/app/demo/example work, these items remain outside
this protocol companion and need their own scoped cards or decisions:

1. whether `advertise` delegation material is required inside candidate domain
   hints or only validated at publication/handshake boundaries;
2. whether the JSON-shaped example here becomes the final canonical wire
   encoding, including JSON/JCS vs another wire form;
3. public SDK helper API compatibility, method names, storage backend,
   browser/native platform wiring, UI/devtools surfaces, and demo flows;
4. any authority/security semantic change beyond non-authoritative candidates;
5. concrete protocol crate validators and vector fixture placement.

SDK helpers MAY make the candidate pipeline easier to use, but they MUST consume
or implement the protocol semantics from `baseline.md`; they must not define a
parallel discovery authority model.

No later implementation should define discovery candidates as membership,
trust, domain authority, offer authority, or policy acceptance.
