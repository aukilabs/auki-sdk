# Peer-To-Peer Cluster Protocol Specs

Status: draft normative baseline.

Last updated: 2026-05-21.

Related RFC backlog:
[`cluster-lifecycle-backlog.md`](cluster-lifecycle-backlog.md).

Related glossary:
[`glossary.md`](glossary.md).

## Scope Of This Version

This document specifies the first minimal version of the peer-to-peer cluster
protocol. Its scope is bootstrapping: peers identify each other, declare served
domains when they expose domain-scoped data, configure or optionally discover
reachable peers, authorize connections, and exchange spatial data through
simple peer-to-peer relationships.

The goal is not centralized runtime control. The goal is a small protocol
foundation that lets peers form clusters and exchange spatial data directly.

This baseline intentionally uses a small, explicit protocol surface. Recurring
rules should be defined once in their owner RFC and referenced elsewhere
instead of restated.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", "MAY",
and "OPTIONAL" are to be interpreted as described in RFC 2119.

Terminology used by this document is defined in the related glossary.

Sections marked "To Fill" are placeholders for unfinished RFC work. They are
not normative until their status changes.

This document distinguishes specified v1 text from v1 To Fill sections.
Specified v1 text is the current v1 baseline. V1 To Fill sections are in v1
scope, but their concrete interoperability rules are not defined until the
placeholder section is filled.

## Protocol Structure

This document is ordered from protocol foundations to runtime behavior:

- protocol foundations: authority, identity, time, wire conventions, and
  failure codes;
- peer and domain model;
- discovery and reachability;
- connection lifecycle;
- spatial data exchange;
- compatibility and observability.

## Protocol Foundations

### RFC-0001: Authority Boundaries

#### Requirement

Protocol authority is built from:

- the transport-authenticated libp2p peer id;
- a wallet-signed peer binding;
- verified domain declarations and delegations;
- local peer authorization, domain access policy, and offer policy.

The following are never authority proofs by themselves:

- Discovery records, peer graph hints, advertised addresses, and relays;
- offers, registry references, labels, metadata, diagnostics, and status
  snapshots;
- timestamps, freshness hints, liveness checks, and clock-sync results;
- external registries, blockchain records, NFTs, or tokenomics systems unless a
  later RFC defines that external-binding authority model.

Time and clock semantics are defined in `RFC-0035`.

Domain ownership, delegation, and accepted served-domain membership prove only
the authority they state. They do not prove payload correctness, canonical
truth, completeness, trustworthiness, or dialability.

Unless a later RFC explicitly requires it, v1 authority validation MUST NOT
require Discovery access, blockchain access, registry access, online revocation
lookup, or any other online lookup.

### RFC-0002: V1 JSON Wire Conventions

#### Requirement

V1 signed authority objects and protocol messages are JSON objects.

Binary values in v1 JSON are encoded as base64url without padding.

UTC timestamps use RFC3339 strings with a `Z` suffix.

The v1 `wallet_signature_scheme` value `ed25519` means:

- raw 32-byte Ed25519 public keys;
- raw 64-byte Ed25519 signatures;
- Ed25519 verification over the defined signed bytes.

Unless a field definition overrides them, common v1 encodings are:

- wallet public-key fields: base64url without padding over raw 32-byte Ed25519
  public keys;
- `signature`: base64url without padding over a raw 64-byte Ed25519 signature;
- `domain_id`: base64url without padding over a raw 32-byte domain id;
- `nonce`: base64url without padding over a raw 16-byte nonce;
- peer id fields: standard libp2p PeerId text representations.

Unknown fields MAY be present unless a specific object forbids them.
Receivers MUST ignore unknown fields after validation unless a later RFC
defines them.

Fields named `label`, `display_name`, `metadata`, `message`, `details`, and
diagnostic objects are for operators and applications. They MUST NOT change
identity, authority, delegation, reachability, or policy decisions.
Implementations MUST NOT require another implementation to parse those fields
for protocol behavior unless a later RFC defines a structured field.

### RFC-0003: Signed JSON Object Conventions

#### Requirement

For v1 peer bindings, domain declarations, and domain delegations, the signed
bytes are the RFC 8785 JSON Canonicalization Scheme output for the whole object
with only the `signature` field removed.

The `type` field is part of the signed bytes and is the domain separator.

Receivers MUST include unknown fields in the canonical signed bytes before
signature verification.

Implementations MUST NOT normalize peer ids, timestamps, base64url spelling,
field values, or array order, and MUST NOT drop unknown fields before
canonicalizing signed bytes.

### RFC-0004: Peer Identity And Wallet Binding

#### Requirement

A peer id MUST be bound to a wallet identity by a wallet-signed peer binding.

Each peer MUST present a wallet identity through a verified peer binding.

A wallet MAY bind one or more peer ids.

The v1 peer binding schema is defined in `RFC-0005`.

The wallet authority key and the libp2p peer key are separate protocol roles.
The libp2p peer key authenticates the transport connection. The wallet
authority key signs the peer binding that authorizes that runtime peer id.

The wallet/libp2p role separation is enforced by the verification rules in
`RFC-0005`.

Deployments SHOULD use distinct key material for wallet authority and libp2p
peer identity. A deployment that intentionally reuses key material loses that
key-compromise separation, but the protocol still treats the two roles
separately.

A peer binding MAY include a metadata label.

A peer binding MAY be reused across sessions.

Freshness, refresh, and exact failure mapping are defined by `RFC-0005`.

#### Verification

V1 peer binding verification is defined in `RFC-0005`.

#### Consequences

A peer binding proves only that the wallet recognizes the connected libp2p
peer id as one of its runtime peers.

A peer binding MUST be interpreted with the authority boundaries in `RFC-0001`.

### RFC-0005: Peer Binding Schema

#### Requirement

A v1 peer binding is a JSON object.

A v1 peer binding MUST include:

- `type`: string, exactly `auki.peer_binding.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `wallet_public_key`: wallet public key;
- `peer_id`: peer id string;
- `issued_at`: timestamp;
- `signature`: Ed25519 signature.

A v1 peer binding MAY include:

- `label`: string.

#### Signed Bytes

Peer binding signed bytes use the signed-object conventions in `RFC-0003`.

#### Peer Id Encoding

The `peer_id` field MUST be parsed as a libp2p PeerId. Receivers MUST support
the standard libp2p PeerId text forms required by libp2p, including legacy
base58btc multihash form and CID/multibase form.

Peer id comparison MUST use parsed libp2p PeerId equality, not string equality.

Auki v1 does not define custom libp2p peer id derivation, custom libp2p key
types, or custom Identify-protocol authority rules. Implementations MUST rely
on their libp2p stack for libp2p peer id parsing and connection identity.

#### Verification

To verify a v1 peer binding, a receiver MUST:

1. Decode the JSON object and verify all required fields are present and
   well-formed.
2. Verify that `type` and `wallet_signature_scheme` are supported.
3. Decode `wallet_public_key` and `signature`.
4. Recompute the canonical signed bytes.
5. Verify `signature` against `wallet_public_key` over the canonical signed
   bytes.
6. Parse `peer_id` as a libp2p PeerId.
7. Verify that the parsed `peer_id` equals the transport-authenticated remote
   libp2p peer id for the connection.
8. Apply local freshness policy to `issued_at`.

The transport-authenticated remote libp2p peer id is the authority for the
connected peer id. A peer id carried in a peer binding is a signed claim that
MUST match the transport-authenticated peer id; it MUST NOT override it.

The peer binding signature is a wallet authority signature. It is not the
libp2p secure-channel signature, and it MUST NOT be verified against the
remote libp2p public key unless that same key is explicitly the declared wallet
public key.

#### Freshness

The `issued_at` timestamp is required in v1 peer bindings.

Receivers MAY enforce a maximum accepted binding age from `issued_at`. The
recommended default maximum accepted binding age is 1 hour.

When a receiver enforces a maximum accepted binding age, it MUST reject a
binding older than that age with `identity.binding_too_old`.

Receivers SHOULD reject bindings whose `issued_at` is in the future beyond
local clock-skew tolerance with `identity.binding_from_future`. The recommended
default future tolerance is 5 minutes.

Refreshing a peer binding requires a new wallet authority signature over a
binding for the same wallet public key and peer id with a newer `issued_at`.
Possession of the libp2p peer private key alone is not sufficient to refresh a
peer binding.

#### Failure Mapping

A receiver SHOULD fail malformed peer bindings with
`identity.invalid_peer_binding`. This includes missing required fields,
unsupported `type`, unsupported wallet signature scheme, malformed base64url,
wrong public-key or signature length for the declared scheme, malformed
`issued_at`, or an unparsable `peer_id`.

A receiver SHOULD fail a binding whose signature does not verify with
`identity.invalid_signature`.

A receiver SHOULD fail a binding whose parsed peer id does not match the
transport-authenticated remote libp2p peer id with `identity.peer_id_mismatch`.

### RFC-0006: Domain Identity And Ownership

#### Requirement

A domain MUST have a stable domain id that can be verified without Discovery,
blockchain access, or any online registry.

A domain id MUST be derived from the domain owner wallet's public key and a
nonce:

domain_id = hash(domain_owner_wallet_public_key, nonce)

The concrete v1 hash input, hash function, and domain id encoding are defined
in `RFC-0007`.

The nonce MUST be unique for domains created by the same domain owner wallet.

The domain owner wallet authorizes a domain id through a domain declaration.
The v1 domain declaration schema is defined in `RFC-0007` and binds:

- domain id;
- domain owner wallet public key;
- nonce.

The domain owner wallet MAY authorize runtime peers to advertise or serve under
that domain.

Later RFCs can define additional domain-scoped actions and authorization rules.

#### Runtime Authority

A peer MAY serve a domain directly when the peer controls the domain owner
wallet.

A peer MAY serve a domain on behalf of the domain owner wallet when it presents
a valid delegation signed by the domain owner wallet.

A valid delegation proves only the delegated authority it states.

#### External Bindings

External registries, blockchain records, NFTs, or tokenomics systems MAY bind
to a domain id.

External bindings are optional for v1 peer-to-peer use.

The native domain id model in this version is wallet-rooted. It fits local,
private, non-transferable, and explicitly delegated domains.

Transferable NFT-backed domains are a future external-binding authority model.
In that model, an external registry, NFT, or token record may identify the
current controller wallet for a domain id. The concrete ownership-transfer,
controller-resolution, revocation, and chain-finality rules are out of scope
for this baseline.

#### Consequences

The same wallet-rooted domain id model supports local, LAN-only, offline, and
externally referenced domains. Transferable token-backed domains require a later
external-binding authority model.

Domain ownership and external bindings are subject to `RFC-0001`.

### RFC-0007: Domain Declaration Schema

#### Requirement

A v1 domain declaration is a JSON object.

A v1 domain declaration MUST include:

- `type`: string, exactly `auki.domain_declaration.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `domain_id`: v1 domain id;
- `domain_owner_public_key`: domain owner wallet public key;
- `nonce`: domain nonce;
- `signature`: Ed25519 signature.

A v1 domain declaration MAY include:

- `label`: string.

#### Domain Id Derivation

The v1 domain id is the SHA-256 digest of the RFC 8785 JSON
Canonicalization Scheme output for this JSON object:

```json
{
  "type": "auki.domain_id.v1",
  "wallet_signature_scheme": "ed25519",
  "domain_owner_public_key": "<domain_owner_public_key>",
  "nonce": "<nonce>"
}
```

The `domain_owner_public_key` and `nonce` values in the hash input MUST be the
same base64url strings carried in the domain declaration.

The encoded `domain_id` field is base64url without padding over the raw
32-byte SHA-256 digest.

The domain id hash input does not include `label`, `signature`, or unknown
declaration fields.

#### Signed Bytes

Domain declaration signed bytes use the signed-object conventions in
`RFC-0003`.

#### Verification

To verify a v1 domain declaration, a receiver MUST:

1. Decode the JSON object and verify all required fields are present and
   well-formed.
2. Verify that `type` and `wallet_signature_scheme` are supported.
3. Decode `domain_id`, `domain_owner_public_key`, `nonce`, and `signature`.
4. Recompute the v1 domain id from `domain_owner_public_key` and `nonce`.
5. Verify that the recomputed domain id equals `domain_id`.
6. Recompute the canonical signed bytes.
7. Verify `signature` against `domain_owner_public_key` over the canonical
   signed bytes.

Domain declaration verification follows the online-lookup rule in `RFC-0001`.

#### Failure Mapping

A receiver SHOULD fail malformed domain declarations with
`domain.invalid_declaration`. This includes missing required fields,
unsupported `type`, unsupported wallet signature scheme, malformed base64url,
wrong field length for the declared scheme, malformed nonce, or an invalid
signature.

A receiver SHOULD fail a declaration whose recomputed domain id does not match
the declared `domain_id` with `domain.id_mismatch`.

### RFC-0008: Domain Delegation Schema

#### Requirement

A v1 domain delegation is a JSON object.

A v1 domain delegation MUST include:

- `type`: string, exactly `auki.domain_delegation.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `domain_id`: delegated domain id;
- `domain_owner_public_key`: domain owner wallet public key;
- `delegate_wallet_public_key`: wallet public key from the delegate peer
  binding;
- `delegate_peer_id`: delegate peer id string;
- `scopes`: non-empty array of strings;
- `valid_from`: timestamp;
- `expires_at`: timestamp;
- `signature`: Ed25519 signature by the domain owner wallet.

A v1 domain delegation MAY include:

- `label`: string.

The v1 delegation scopes are exactly:

- `advertise`: the peer may announce the domain through Discovery,
  peer-discovery metadata, or equivalent reachability surfaces;
- `serve`: the peer may declare the domain during handshake and serve offers
  or spatial data scoped to that domain.

The `scopes` array MUST contain only v1 delegation scopes and MUST NOT contain
duplicates.

Before signing, producers MUST sort `scopes` in alphabetical string order.
Receivers MUST verify the signature against the `scopes` array exactly as
presented.

The `expires_at` timestamp MUST be later than `valid_from`.

#### Signed Bytes

Domain delegation signed bytes use the signed-object conventions in
`RFC-0003`.

#### Verification

To verify a v1 domain delegation for a claimed action, a receiver MUST:

1. Decode the JSON object and verify all required fields are present and
   well-formed.
2. Verify that `type` and `wallet_signature_scheme` are supported.
3. Decode `domain_id`, `domain_owner_public_key`,
   `delegate_wallet_public_key`, and `signature`.
4. Parse `delegate_peer_id` as a libp2p PeerId.
5. Verify that `scopes` contains only v1 scopes, is non-empty, has no
   duplicates, and is in alphabetical string order.
6. Verify that `valid_from` and `expires_at` form a valid time window.
7. Recompute the canonical signed bytes.
8. Verify `signature` against `domain_owner_public_key` over the canonical
   signed bytes.
9. Verify that `domain_id` equals the declared domain id being validated.
10. Verify that `domain_owner_public_key` equals the owner public key from the
    verified domain declaration.
11. Verify that `delegate_wallet_public_key` equals the wallet public key from
    the verified remote peer binding.
12. Verify that `delegate_peer_id` equals the transport-authenticated remote
    libp2p peer id.
13. Verify that the claimed action is included in `scopes`.
14. Verify that the current time is within the delegation validity window.

Domain delegation verification follows the online-lookup rule in `RFC-0001`.

#### Presentation

A peer that claims to serve a domain on behalf of a domain owner wallet MUST
present the domain declaration and a matching delegation during handshake.

A peer whose verified wallet public key is the domain owner public key MAY
serve the domain directly without a delegation.

One delegation authorizes exactly one `domain_id`, one
`delegate_wallet_public_key`, and one `delegate_peer_id`.

#### Expiry And Replacement

A delegation is valid only within its `valid_from` and `expires_at` window.

The baseline v1 revocation and rotation mechanisms are expiry and replacement.
Replacing a delegation means issuing a new domain owner signature over a new
delegation object.

V1 has no online revocation requirement.

#### Failure Mapping

A receiver SHOULD fail malformed, wrong-domain, wrong-owner, wrong-peer,
wrong-wallet, wrong-scope, not-yet-valid, or invalid-signature delegations with
`domain.invalid_delegation`.

A receiver SHOULD fail an expired delegation with
`domain.expired_delegation`.

A receiver SHOULD fail a missing required delegation with
`domain.missing_delegation`.

### RFC-0009: Authority Chain Validation

#### Requirement

After a transport connection is established, each peer MUST validate the remote
peer's authority chain before treating any remote offer as usable.

Authority-chain validation MUST run in this order:

1. Verify the remote peer binding using `RFC-0005`.
2. Run peer authorization for the verified peer identity.
3. Validate each declared domain independently using `RFC-0007` and, when
   delegation is required, `RFC-0008`.
4. Compute the accepted served domain set for the peer relationship.

Peer authorization is defined in `RFC-0020`. In the authority-chain validation
path, peer authorization runs after peer binding verification and before served
domains are accepted.

Offer loading happens after authority-chain validation and follows the offer
usability rules in `RFC-0026`.

For each declared domain, domain declaration verification uses `RFC-0007`.

A declared domain is directly accepted when the verified peer wallet is the
domain owner wallet.

A declared domain is accepted through delegation when the peer presents a valid
delegation from the domain owner wallet that authorizes the verified peer
identity to serve that domain. Delegation validation, expiry, claimed-action
checks, and v1 delegation scopes are defined in `RFC-0008`.

For served-domain validation, the claimed action is `serve`. For Discovery,
peer-discovery metadata, or equivalent reachability advertisement, the claimed
action is `advertise`.

Domain authority validation answers whether the remote peer may serve under a
domain. Local domain access policy MAY still reject the domain with
`policy.domain_rejected`, as defined in `RFC-0020`.

Validating one declared domain MUST NOT cause another declared domain from the
same peer to be accepted. Each declared domain needs its own valid authority
chain.

The v1 authority validation path follows the online-lookup rule in `RFC-0001`.
Peer-binding freshness is defined in `RFC-0005`. Delegation expiry and
replacement are defined in `RFC-0008`.

#### Consequences

Authority-chain validation is interpreted under `RFC-0001`.

Invalid identity material is a peer-level failure. Invalid domain authority is
a domain-level failure unless peer authorization or local policy chooses to
reject the whole peer relationship.

### RFC-0010: Failure Code Registry

#### Requirement

Lifecycle, authority, offer-loading, Get, Subscribe, and message diagnostics
SHOULD use stable string failure codes in `category.reason` form.

Baseline failure codes:

- `protocol.unsupported_version`
- `handshake.invalid_message`
- `handshake.missing_required_material`
- `identity.missing_peer_binding`
- `identity.invalid_peer_binding`
- `identity.peer_id_mismatch`
- `identity.invalid_signature`
- `identity.binding_too_old`
- `identity.binding_from_future`
- `domain.invalid_declaration`
- `domain.id_mismatch`
- `domain.missing_delegation`
- `domain.invalid_delegation`
- `domain.expired_delegation`
- `authorization.peer_rejected`
- `policy.domain_rejected`
- `offer.unknown_offer`
- `offer.domain_not_served`
- `offer.unsupported_kind`
- `offer.unsupported_access_mode`
- `offer.unsupported_payload_type`
- `offer.invalid_catalog_request`
- `offer.invalid_catalog_response`
- `offer.invalid_offer`
- `offer.catalog_unavailable`
- `offer.temporarily_unavailable`
- `offer.stale`
- `message.invalid_envelope`
- `message.invalid_payload`
- `message.payload_too_large`
- `message.sequence_gap`
- `get.invalid_request`
- `subscribe.invalid_request`
- `offer.load_failed`
- `transport.failed`

### RFC-0035: Time And Clock Semantics

#### Requirement

V1 time fields are metadata and diagnostics. Timestamps, freshness hints,
liveness checks, and clock-sync results MUST NOT be treated as authority
proofs, payload-correctness proofs, or timestamp-truth proofs.

The `generated_at` field is producer wall-clock metadata. It reports when the
producer generated an object, snapshot, message, or status record. It MUST NOT
replace `timestamp_ns` for domain data timing.

The `timestamp_ns` field is producer or domain event time in nanoseconds. It is
measured in the clock identified by the message `clock` field or by an
inherited clock registry reference for the stream, response, or offer.

The `clock` field, when present, is a registry-reference object whose
`registry` is `clock`. Clock registry references use the registry-reference
shape in `RFC-0024`.

If `timestamp_ns` is present and no clock can be resolved, the receiver SHOULD
treat the timestamp as uninterpretable rather than assuming local wall-clock
time.

Freshness hints such as `updated_at`, `expires_at`, `ttl`, and `last_seen_at`
MAY guide local policy. They do not prove authority, payload correctness, or
dialability.

Local receive time is receiver diagnostic state. It is distinct from
`generated_at` and `timestamp_ns`.

Clock-sync results are optional diagnostics in the specified v1 protocol. The
specified v1 protocol does not require a concrete clock-sync protocol for
Offer, Get, or Subscribe. Clock-sync results MAY inform local diagnostics or
local policy. A local policy MAY decline a path when clock-sync state is absent
or unhealthy, but that policy decision is not a protocol authority proof.

Concrete NTP or clock-sync message flow is v1 To Fill work.

## Peer And Domain Model

### RFC-0011: Serving Peers Declare Domains

#### Requirement

A peer that only consumes remote offers MAY participate without declaring a
local domain.

A peer that serves offers, publishes spatial data, or asks a remote peer to
accept it as serving a domain MUST declare that domain and MUST prove that it
controls the domain owner wallet or has a valid delegation.

A local domain is the authority boundary for spatial state served by the peer,
including frames, clocks, sensors, streams, logs, maps, transforms, offers, and
resources.

A peer MAY own or maintain a local domain without making that domain
discoverable or exposing offers from it.

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

A cluster is subject to `RFC-0001`.

#### Consequences

A peer can consume another peer's spatial data through a direct peer
relationship without declaring its own local domain. The peers do not need to
merge their domains or share a common runtime authority.

Failure of one peer SHOULD affect that peer's served domains and peer
relationships only; it SHOULD NOT invalidate unrelated domains.

### RFC-0012: Served Domain Set

#### Requirement

Each peer relationship MUST track the set of remote domains the local peer has
accepted the remote peer to serve.

The served domain set is computed from the remote peer's declared domains after
peer binding validation, peer authorization, domain declaration validation, and
delegation validation.

A peer relationship MAY have an empty served domain set when the remote peer is
only consuming local offers or when none of its declared domains are accepted.
An empty served domain set MUST NOT by itself close the connection or mark the
peer relationship as degraded.

Partial domain acceptance is allowed. If a remote peer declares multiple domains
and only some validate, the receiver SHOULD accept the valid domains, reject the
invalid domains, and keep diagnostics for each rejected domain.

The served domain set is scoped to one peer relationship. Accepting a domain for
one remote peer MUST NOT imply that another peer is accepted to serve the same
domain.

The specified v1 protocol does not define in-place changes to the served domain
set during an active peer relationship. A peer that wants the remote peer to
accept a changed served-domain set MUST use a reconnect or fresh handshake.
The changed served-domain set MUST NOT be treated as accepted until the same
authority-chain validation used during the initial handshake has succeeded.

#### Offer Interaction

The served domain set is the authority filter used by `RFC-0026`. Offers
outside the set fail the served-domain part of offer usability.

If offer loading fails for an accepted served domain, the peer relationship MAY
remain ready while reporting `offer.load_failed` for that offer-loading path.

#### Dynamic Updates (To Fill)

Classification: v1 To Fill. This section is in v1 scope. Its concrete
interoperability rules are unset until filled.

Future protocol work can define how a peer adds, removes, refreshes, or
replaces served domains during an active peer relationship.

That work needs to describe:

- update message shape;
- validation trigger;
- authority-chain reuse;
- stale offer and subscription handling;
- failure mapping.

#### Diagnostics

Diagnostics SHOULD report:

- each declared domain id;
- whether the declared domain was accepted or rejected;
- the failure code for each rejected domain;
- whether the peer relationship has an empty served domain set;
- which loaded offers are scoped to each accepted served domain.

### RFC-0013: Private And Discoverable Peers

#### Requirement

The specified v1 protocol MUST support private or configured peer-to-peer
connectivity without Discovery.

A private or configured peer does not need to register presence in Discovery
and can still:

- dial another peer through explicit configuration, invitation, direct address
  exchange, or another configured entrypoint;
- be dialed through explicit configuration;
- participate in authorized peer-to-peer exchange once connected.

A discoverable peer registers presence through Discovery or an equivalent
index.

Discoverable-peer interoperability is v1 To Fill until `RFC-0015` defines a
concrete Discovery record shape. Discovery data-type hints are v1 To Fill
until `RFC-0016` defines the hint vocabulary.

Until those RFCs are filled, implementations MAY support discoverable peers
through implementation-defined Discovery records, but specified v1
peer-to-peer interoperability MUST NOT depend on those records.

#### Consequences

A Discovery query MUST NOT be used to prove that a private peer does not exist.

## Discovery And Reachability

### RFC-0014: Discovery Is Optional Entrypoint Rendezvous

#### Requirement

A peer MUST NOT be required to register with Discovery merely to use the
currently specified v1 peer-to-peer lifecycle or to connect to another peer.

A peer MAY register with Discovery when it wants to be discoverable by other
peers.

A peer that does not register with Discovery MAY still connect to other peers
through manual configuration, invitation, direct address exchange, or another
discovery mechanism.

The currently specified v1 peer-to-peer lifecycle does not require a concrete
Discovery record schema.

#### Discovery Authority

Discovery MUST be treated as rendezvous/presence infrastructure unless a later
RFC explicitly expands its authority.

Discovery is subject to `RFC-0001`.

#### Discovery Records

When implemented, a Discovery record SHOULD answer:

- what domain is being advertised;
- how a peer can dial it;
- coarse, non-authoritative metadata about data types that may be available;
- how fresh the advertisement is.

Until `RFC-0015` and `RFC-0016` are filled, Discovery record shapes and
data-type hints are implementation-defined and MUST NOT be required for the
currently specified v1 peer-to-peer lifecycle.

A Discovery record MUST NOT be treated as an authoritative offer catalog.

A peer that advertises a domain on behalf of another wallet MUST have a valid
delegation with `advertise` scope. A peer that controls the domain owner wallet
MAY advertise that domain directly.

Receiving an advertisement MUST NOT by itself cause the receiver to accept the
advertised peer as a server for that domain. Served-domain acceptance still
requires peer-to-peer authority validation with `serve` authority.

Discovery records MAY advertise peer-graph entrypoints, but not authoritative
membership. Discovery SHOULD attach freshness metadata and expire records that
are not refreshed. Stale or expired Discovery data MUST NOT invalidate existing
peer-to-peer connections by itself.

#### Consequences

Existing peer relationships SHOULD continue when Discovery is temporarily
unavailable, assuming the underlying peer-to-peer transport remains healthy.

Implementations SHOULD distinguish "Discovery presence degraded" from "peer
relationship degraded" in status and diagnostics.

### RFC-0015: Discovery Record Shape (To Fill)

Classification: v1 To Fill. This section is in v1 scope. Its concrete
interoperability rules are unset until filled.

Define the concrete Discovery advertisement:

- domain id and optional display label;
- peer id and dialable advertised addresses;
- freshness fields such as `ttl`, `expires_at`, or `last_seen_at`;
- coarse, non-authoritative data-type hints;
- refresh, update, remove, and expiry behavior.

The record shape needs to preserve entrypoint advertisement semantics and avoid
becoming an authoritative offer catalog.

### RFC-0016: Discovery Data-Type Hints (To Fill)

Classification: v1 To Fill. This section is in v1 scope. Its concrete
interoperability rules are unset until filled.

Define the coarse data-type hints allowed in Discovery records:

- vocabulary for baseline hints;
- how hints differ from offers;
- whether hints are free-form, registered, or both;
- freshness behavior for hints;
- client handling for missing, stale, or unsupported hints.

### RFC-0017: Listen Addresses And Advertised Addresses Are Different

#### Requirement

Implementations MUST distinguish listen addresses from advertised addresses.

- A listen address is where the local network runtime binds.
- An advertised address is what another peer should dial.

Implementations MUST NOT automatically advertise non-dialable bind addresses
as cross-host dial addresses.

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

Diagnostics SHOULD report the final advertised address set and identify whether
each address was auto-detected, operator-supplied, or relay-mediated.

### RFC-0018: Relay Is Connectivity, Not Authority

#### Requirement

Relay support MAY be used to establish peer-to-peer connectivity when direct
dialing fails or is unavailable.

Relay support MUST NOT change identity, authority, policy, or spatial-data
semantics.

#### Consequences

A relay-mediated connection MUST be treated as a transport path to the same
remote peer id, not as a different authority model.

Discovery MAY advertise relay-mediated multiaddrs when direct addresses are not
sufficient.

## Connection Lifecycle

### RFC-0019: Peer Handshake

#### Requirement

After dialing and establishing a transport connection, peers MUST run a
symmetric handshake before loading offers or exchanging spatial data.

The handshake is symmetric because either peer may be a producer, consumer, or
both. Each side MUST be able to present identity, supported protocol versions,
authorization material, and any domains it claims to serve.

The v1 lifecycle handshake protocol ID is
`/auki/cluster-lifecycle/0.0.1`.

The v1 lifecycle version string is `auki.cluster_lifecycle.v1`.

The v1 offer-catalog protocol ID is `/auki/offer-catalog/0.0.1`.

Each handshake side MUST send one v1 handshake message and MUST validate the
remote v1 handshake message before loading offers or exchanging spatial data.

Remote peer id handling follows `RFC-0005`: handshake material MUST match the
transport-authenticated libp2p peer id and MUST NOT override it.

Each side MUST choose the highest mutually supported lifecycle protocol version.
If no compatible lifecycle protocol version exists, the peer relationship MUST
fail with `protocol.unsupported_version`.

A peer that only consumes remote offers MAY send an empty `declared_domains`
array and omit the offer-catalog fetch path.

A peer that exposes a domain-scoped offer catalog MUST declare the domains it
may use in offers from that catalog. Offer use follows `RFC-0026`.

#### Handshake Message

A v1 handshake message is a JSON object.

A v1 handshake message MUST include:

- `type`: string, exactly `auki.peer_handshake.v1`;
- `supported_lifecycle_versions`: non-empty array of lifecycle version
  strings;
- `peer_binding`: peer binding object as defined in `RFC-0005`;
- `declared_domains`: array of declared-domain objects.

A v1 handshake message MAY include:

- `authorization_material`: array of authorization-material objects;
- `offer_catalog`: offer-catalog fetch-path object;
- `diagnostics`: JSON object;
- `metadata`: JSON object.

The `supported_lifecycle_versions` array MUST contain
`auki.cluster_lifecycle.v1` for v1. It MUST NOT contain duplicates.

The `declared_domains` array MAY be empty.

#### Declared-Domain Object

A v1 declared-domain object MUST include:

- `domain_id`: domain id string;
- `domain_declaration`: domain declaration object as defined in `RFC-0007`.

A v1 declared-domain object MAY include:

- `delegation`: domain delegation object as defined in `RFC-0008`;
- `metadata`: JSON object.

The `domain_id` field MUST match the `domain_id` in `domain_declaration`.
Authority-chain validation determines whether `delegation` is required, as
defined in `RFC-0009`.

#### Authorization Material

A v1 authorization-material object MUST include:

- `type`: open string naming the authorization material.

A v1 authorization-material object MAY include:

- `id`: string;
- `value`: JSON value;
- `expires_at`: timestamp;
- `metadata`: JSON object.

The baseline protocol defines no required authorization-material types.
Authorization material is input to local peer authorization policy in
`RFC-0020`. It does not replace the authority-chain validation in `RFC-0009`.

#### Offer-Catalog Fetch Path

A v1 offer-catalog fetch-path object MUST include:

- `type`: string, exactly `auki.offer_catalog_path.v1`;
- `protocol_id`: string, exactly `/auki/offer-catalog/0.0.1`;
- `catalog_version`: string, exactly `auki.offer_catalog.v1`.

A v1 offer-catalog fetch-path object MAY include:

- `metadata`: JSON object.

Presence of `offer_catalog` means the peer exposes the v1 offer-catalog path.
Absence of `offer_catalog` means the peer exposes no v1 offer-catalog
path in this peer relationship.

#### Handshake Result

For each remote peer relationship, the handshake MUST produce:

- selected lifecycle protocol version;
- verified peer id and wallet public key;
- peer authorization result;
- validation result for each declared domain;
- accepted served domain set;
- offer-catalog fetch path, if the remote peer exposes one;
- initial lifecycle state;
- stable failure codes for any rejected identity, domain, authorization, or
  offer-loading step.

The connection MUST NOT load remote offers before the authority-chain
validation path in `RFC-0009` has completed.

The connection MAY become ready with an empty remote served domain set. In that
case, the remote peer is connected and authorized but exposes no usable remote
offers for that relationship.

#### Failure Mapping

A receiver SHOULD fail a malformed v1 handshake message with
`handshake.invalid_message`. This includes invalid JSON, unsupported `type`, a
missing or malformed `supported_lifecycle_versions` field, a malformed
`declared_domains` field, a malformed `authorization_material` field, or a
malformed `offer_catalog` field.

A receiver SHOULD fail a handshake that omits `peer_binding` with
`identity.missing_peer_binding`.

A receiver SHOULD fail missing authorization material required by local policy
with `handshake.missing_required_material` or `authorization.peer_rejected`.

A receiver SHOULD fail a declared-domain object whose `domain_id` does not
match its domain declaration with `domain.id_mismatch`.

A receiver SHOULD fail a declared-domain object that requires but omits a
delegation with `domain.missing_delegation`.

A receiver SHOULD fail unsupported lifecycle versions with
`protocol.unsupported_version`.

#### Lifecycle Examples

In the happy path, Park dials Robot, negotiates a lifecycle version, verifies
Robot's peer binding, authorizes Robot, validates Robot's declared domains,
computes Robot's served domain set, fetches Robot's offer catalog, and then
uses Get or Subscribe only for offers that pass `RFC-0026`.

If Robot's peer binding claims a different peer id than the
transport-authenticated libp2p peer id, Park rejects the peer relationship with
`identity.peer_id_mismatch` and stops before domain validation or offer loading.

If Robot declares domains `A`, `B`, and `C`, Park MAY accept valid domains
`A` and `B`, reject expired domain `C` with `domain.expired_delegation`, keep
the peer relationship, and treat offers scoped to `C` as unusable.

### RFC-0020: Authorization Model

#### Requirement

Peer authorization is the peer-level allow or deny decision for one peer
relationship after peer binding verification.

The baseline peer authorization modes are:

- `all`: accept any peer with a valid peer binding;
- `whitelisted-only`: accept only configured peer ids or wallet public keys;
- `app-policy`: defer the allow or deny decision to application policy.

The default peer authorization mode for this experimental baseline is `all`.

Deployments that need tighter peer admission SHOULD use `whitelisted-only` or
`app-policy`.

Peer authorization MUST NOT depend solely on Discovery presence.

#### Domain And Offer Policy

Domain authority validation decides whether a remote peer may serve under a
domain.

Domain access policy decides whether the local application wants to consume or
use an otherwise valid remote domain.

Offer policy decides whether the local application wants to load, display, Get,
or Subscribe to a specific offer.

Domain access policy and offer policy MAY be application-defined. They MUST NOT
replace domain authority validation.

Invite tokens, signed challenges, and per-offer policy hooks are optional
hardening layers for future RFC work.

### RFC-0021: Peer Connectivity State Is Tracked Per Remote Peer

#### Requirement

A peer SHOULD track connectivity and readiness state independently for each
remote peer.

Failure of one peer relationship MUST NOT force unrelated peer relationships to
restart or become invalid.

#### Candidate State Model

The following states are descriptive names, but implementations SHOULD expose
equivalent diagnostic information:

- `unknown`: the peer relationship has no known discovery, configuration, or
  connection state;
- `discovered`: the peer was learned through Discovery or an equivalent index;
- `configured`: the peer was learned through explicit configuration,
  invitation, direct address exchange, or another non-Discovery mechanism;
- `dialing`: the local peer is attempting to establish a transport connection;
- `connected`: the transport connection exists, but handshake and peer
  authorization are not complete;
- `authorized`: peer identity is verified and peer authorization has succeeded;
- `loading offers`: the peer relationship is loading remote offers for accepted
  served domains;
- `ready`: handshake and peer authorization are complete, and any required
  initial offer-loading attempt has completed;
- `degraded`: the peer relationship remains usable but has a recoverable
  problem, such as Discovery freshness loss, offer-loading failure, or a
  rejected domain;
- `lost`: the transport connection or peer relationship is no longer available.

A peer relationship MAY reach `ready` with an empty remote served domain set.
In that case, the relationship is ready for peer-level interaction but exposes
no usable remote offers.

A rejected declared domain MUST NOT by itself force `degraded` when other
declared domains are accepted or when an empty served domain set is allowed by
local policy. Implementations SHOULD expose the rejected-domain diagnostics
without changing unrelated peer relationships.

#### Consequences

A peer losing connectivity to one remote peer SHOULD NOT drop unrelated peer
connections.

A peer exiting SHOULD make that peer unavailable to other peers. It SHOULD NOT
by itself invalidate unrelated peer relationships or domains.

### RFC-0022: Peer Graph Hints (To Fill)

Classification: v1 To Fill. This section is in v1 scope. Its concrete
interoperability rules are unset until filled.

Define how a peer shares additional peer candidates after connection:

- whether learned peers are dialed automatically or surfaced as candidates;
- what metadata can be shared;
- whether a peer may hide known peers;
- how the exchange avoids becoming authoritative membership;
- whether DHT-style peer discovery is in scope for this baseline.

The intended baseline is to treat learned peers as non-authoritative candidate
dial targets or offer sources.

## Spatial Data Exchange

### RFC-0023: Peers Exchange Spatial Data With Offer / Get / Subscribe

#### Requirement

Each peer SHOULD maintain local spatial state for the domains it serves.

After configuration or optional discovery and authorization, peers SHOULD
exchange spatial data peer-to-peer.

A peer MAY choose not to expose spatial data, or MAY expose only a subset of
its spatial data according to local policy.

The minimum baseline exchange shape is:

- `Offer`: a peer advertises named and typed spatial data it can share now.
- `Get`: a peer fetches an offered data item once.
- `Subscribe`: a peer receives ongoing updates from an offer.

A peer that intends to consume spatial data SHOULD fetch offers from remote
peers only after the offer usability rules in `RFC-0026` can be evaluated.

Discovery may help find dial targets, but it follows the authority boundaries
in `RFC-0001` and MUST NOT be required to exchange spatial data.

#### Offers

An offer is a connected peer's declaration of one named and typed data item it
is willing to serve.

Offer ids are scoped to the producing peer's served domain. They identify data
the producer exposes from that domain, not global network objects.

The concrete offer object is defined in `RFC-0024`.

An offer is a reference to data exposed from a domain. It follows the authority
boundaries in `RFC-0001`.

#### Get

`Get` fetches an offered data item once.

Get is for finite responses. In v1, `RFC-0029` narrows Get to descriptors,
registry entries, transform edges, and small snapshots. Future RFCs MAY extend
Get to log ranges, map fragments, or other finite spatial-data representations.

Get failure mapping is defined in `RFC-0029`.

#### Subscribe

`Subscribe` receives live updates from an offered data item. Examples include
a camera stream, point-cloud stream, pose stream, audio stream, or future live
map updates.

Subscribe failure mapping is defined in `RFC-0030`.

#### Consequences

Implementations SHOULD support a peer learning what another peer can share by
name or type before opening a stream or fetching data.

### RFC-0024: Offer Catalog

#### Requirement

An offer catalog is a peer-to-peer snapshot of the offers a connected peer is
willing to expose to the requester at the time of the request.

The offer catalog is runtime metadata under `RFC-0001`. It is not a signed
authority object. Catalog entries become usable only through `RFC-0026`.

A peer that exposes one or more domain-scoped offers MUST declare an
offer-catalog fetch path during handshake.

A peer that consumes remote offers SHOULD fetch the remote offer catalog only
after the authority-chain validation path in `RFC-0009` has completed.

A peer that exposes no offers MAY return an empty catalog.

#### Request

A v1 offer-catalog request is a JSON object.

A v1 offer-catalog request MUST include:

- `type`: string, exactly `auki.offer_catalog_request.v1`.

A v1 offer-catalog request MAY include:

- `domain_ids`: array of domain id strings;
- `kinds`: array of offer-kind strings;
- `include_inline_registry_entries`: boolean.

If `domain_ids` is omitted or empty, the responder SHOULD consider all domains
it is willing to serve to the requester.

A requester SHOULD set `domain_ids` to the domains it accepted in the remote
peer's served domain set when it wants the responder to return only offers the
requester may treat as usable.

If `kinds` is omitted or empty, the responder SHOULD consider all offer kinds
it is willing to advertise to the requester.

If `include_inline_registry_entries` is omitted, it defaults to false.

The `kinds` values are open strings. Unknown or unsupported kind filters SHOULD
produce an empty matching result, not a protocol failure.

If `include_inline_registry_entries` is true, the responder MAY attach
canonical registry JSON to matching registry references when it has the exact
entry locally. This is an optimization only. Consumers MUST still verify the
returned canonical JSON against the referenced hash before using it.

#### Response

A v1 offer-catalog response is a JSON object.

A v1 offer-catalog response MUST include:

- `type`: string, exactly `auki.offer_catalog_response.v1`;
- `offers`: array of offer objects.

A v1 offer-catalog response MAY include:

- `generated_at`: timestamp;
- `diagnostics`: array of diagnostic objects.

The `offers` array MAY be empty. An empty array means the responder understood
the request but has no matching offers currently visible to the requester.

The `generated_at` timestamp follows `RFC-0035`.

#### Diagnostics

A v1 offer-catalog diagnostic uses the v1 error object defined in `RFC-0027`.

#### Offer Object

A v1 offer is a JSON object.

A v1 offer MUST include:

- `offer_id`: string;
- `domain_id`: domain id string;
- `kind`: offer-kind string;
- `status`: string;
- `access_modes`: non-empty array of access-mode strings;
- `payload`: payload descriptor object;
- `registry_refs`: array of registry-reference objects.

A v1 offer MAY include:

- `display_name`: string;
- `updated_at`: timestamp;
- `expires_at`: timestamp;
- `metadata`: JSON object.

The `offer_id` is scoped to the tuple `(producing peer id, domain_id)`.
Consumers that cache remote offers MUST identify an offer by
`(peer_id, domain_id, offer_id)`, not by `offer_id` alone.

For a given catalog response, the tuple `(domain_id, offer_id)` MUST be unique.

Producers SHOULD keep `offer_id` stable across catalog refreshes for the same
logical data source. Producers SHOULD issue a new `offer_id` when reusing the
old id would hide an incompatible payload, registry, or access-mode change.

The `kind` field is an open string. The v1 minimum known kinds are defined in
`RFC-0031`. Consumers MUST ignore unknown kinds unless local application code
explicitly supports them.

The v1 `status` values are:

- `available`: the producer currently believes the offer can be used;
- `temporarily_unavailable`: the offer is known but not currently usable.

An offer with `temporarily_unavailable` MAY remain in the catalog so consumers
can keep stable UI state or retry later. Get and Subscribe requests for such
an offer MAY fail with `offer.temporarily_unavailable`.

An offer that should no longer be discoverable SHOULD be removed from later
catalog snapshots rather than advertised with a permanent unavailable status.

The `updated_at` and `expires_at` fields are freshness hints under `RFC-0035`.
A consumer MAY enforce `expires_at` by local policy. A consumer that enforces
`expires_at` MUST NOT start a new Get or Subscribe attempt for an expired offer
and SHOULD report `offer.stale`.

The v1 `access_modes` values are:

- `get`;
- `subscribe`.

The `access_modes` array MUST contain at least one value and MUST NOT contain
duplicates. An offer MUST NOT advertise an access mode the producer is not
willing to serve for that offer.

The `registry_refs` array MAY be empty when the offer kind does not require
registry context. Offer kinds that require registry context define the required
roles in their own RFC sections.

#### Payload Descriptor

A v1 payload descriptor is a JSON object.

A v1 payload descriptor MUST include:

- `type`: open string naming the payload or schema family.

A v1 payload descriptor MAY include:

- `encoding`: open string;
- `schema_version`: string;
- `media_type`: string.

The payload descriptor helps a consumer decide whether it can decode Get or
Subscribe payloads. The Subscribe accept start result and the Get response
envelope may further commit to exact payload details.

The payload descriptor describes the expected payload family. It MUST NOT carry
the payload bytes or structured payload value; those belong in the spatial
message envelope's payload object defined in `RFC-0027`.

#### Registry References

A v1 registry reference is a JSON object.

A v1 registry reference MUST include:

- `registry`: string;
- `role`: string;
- `id`: string;
- `hash`: string.

A v1 registry reference MAY include:

- `canonical_json`: string.

The registry-reference shape is reused by offer catalogs and spatial message
envelopes.

The v1 known `registry` values are:

- `sensor`;
- `clock`;
- `frame`;
- `detector`.

The `registry` and `role` fields are open strings. The `registry` field names
the registry namespace. The `role` field names why this offer references that
entry, such as `sensor`, `clock`, `frame`, `from_frame`, `to_frame`, or
`detector`. A generic registry-entry offer MAY use `entry` as the role.

Registry references are content-addressed. A consumer that receives
`canonical_json` MUST hash the canonical JSON bytes and verify that the result
matches `hash` before using the entry. A consumer that needs an entry not
inlined in the catalog SHOULD fetch it through a `registry_entry` offer using
Get.

Spatial offers SHOULD include frame registry references needed to interpret
positions, rotations, or point coordinates. Temporal offers SHOULD include
clock registry references needed to interpret timestamps when the producer
knows them at catalog time. Clock registry-reference semantics are defined in
`RFC-0035`.

For live subscriptions, the Subscribe accept start result MAY refine or confirm
registry references for that subscription. The accepted registry context is
authoritative for the lifetime of that subscription.

#### Snapshot And Updates

Each offer-catalog response is a complete snapshot for the request filters the
responder accepted.

If a later complete snapshot for the same peer, domain filter, and kind filter
omits a previously advertised offer, the consumer SHOULD treat that offer as
withdrawn for new Get and Subscribe attempts.

Withdrawing an offer from a later catalog snapshot does not by itself terminate
an existing subscription. Existing subscriptions follow the Subscribe protocol's
end and error semantics.

If a later snapshot includes the same `(domain_id, offer_id)` with changed
payload, registry references, access modes, or status, the consumer SHOULD
treat the offer as updated and re-check local compatibility before new Get or
Subscribe attempts.

#### Failure Mapping

A responder SHOULD fail malformed catalog requests with
`offer.invalid_catalog_request`.

Offer catalog responders and receivers MUST apply `RFC-0026`. Domain-scope
failures use `offer.domain_not_served`.

A receiver SHOULD fail malformed catalog responses with
`offer.invalid_catalog_response`.

A receiver SHOULD ignore individual malformed offers with `offer.invalid_offer`
when the rest of the catalog is usable.

A responder SHOULD use `offer.catalog_unavailable` when it cannot produce a
catalog because of a local recoverable problem.

### RFC-0025: Offer Domain Scope And Authority

#### Requirement

Each v1 offer MUST include exactly one `domain_id`.

One v1 offer belongs to exactly one domain. Multi-domain offers are future
work.

The `domain_id` field in an offer is a producer-declared scope. Receivers MUST
apply the offer usability rules in `RFC-0026`.

A v1 offer SHOULD NOT carry its own domain declaration or delegation unless a
later RFC defines embedded authority proofs. Offer authority is derived from the
peer relationship's accepted served domain set.

#### Domain State Changes

When a domain is rejected, expires under local policy, or is removed from the
accepted served domain set by a future dynamic update protocol, cached offers
scoped to that domain MUST become unusable for new Get and Subscribe attempts.

Existing subscriptions for that domain SHOULD be ended or treated as no longer
authorized once the implementation observes the domain is no longer accepted.
When an explicit Subscribe end message is sent for this case, it SHOULD use
the `not_authorized` reason.

#### Producer Claims And Receiver Authority

The producer controls offer metadata such as `offer_id`, `kind`, `payload`,
`registry_refs`, `status`, and `metadata`.

The receiver controls whether that offer is usable by applying the offer
usability rules in `RFC-0026`.

Offer metadata and payloads follow the authority boundaries in `RFC-0001`.

### RFC-0026: Offer Usability

#### Requirement

A remote offer is usable only when all of the following are true for the
producing peer relationship:

- peer identity is verified;
- peer authorization has succeeded;
- the offer's `domain_id` is in the accepted served domain set;
- local domain access policy and offer policy allow it;
- the offer's kind, access mode, payload descriptor, status, and freshness are
  compatible with the requested path.

Catalog loading, Get, Subscribe, spatial-message, and status rules that refer
to offer usability use this RFC unless a later RFC explicitly changes it.

A producer MUST NOT intentionally expose an offer for a domain it did not
declare to the requester during handshake.

#### Failure Mapping

An offer that fails the served-domain part of the usability check MUST be
rejected or ignored with `offer.domain_not_served`.

An offer that fails kind, access-mode, payload, availability, or freshness
checks SHOULD use the corresponding offer failure code from `RFC-0010`.

### RFC-0027: Spatial Message Envelope

#### Requirement

Get and Subscribe use a shared envelope shape for successful data responses or
messages and shared error objects for failures.

The envelope is a semantic object. This RFC does not require a specific
transport framing, binary encoding, compression scheme, or libp2p protocol id.
The Get and Subscribe RFCs define the concrete request, response, and stream
message shapes for each path.

A v1 spatial message envelope is a JSON object.

A v1 spatial message envelope MUST include:

- `type`: string, exactly `auki.spatial_message.v1`;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `payload`: payload object.

A v1 spatial message envelope MAY include:

- `sequence`: non-negative integer;
- `timestamp_ns`: integer;
- `clock`: registry-reference object;
- `registry_refs`: array of registry-reference objects;
- `generated_at`: timestamp;
- `metadata`: JSON object.

The tuple `(domain_id, offer_id)` identifies the offer that produced the
message within the producing peer relationship. Consumers already know the
producing peer id from the transport-authenticated peer relationship. A message
envelope SHOULD NOT include a `peer_id` field unless a later RFC defines a
diagnostic or forwarding use case for it.

#### Domain And Offer Binding

A receiver MUST reject or ignore a message envelope that fails the
offer usability rules in `RFC-0026` or whose `(domain_id, offer_id)` does
not match the Get request or accepted Subscribe stream.

A message envelope MUST NOT carry its own domain declaration, delegation, or
authority proof in v1.

#### Payload Object

A v1 payload object is a JSON object.

A v1 payload object MUST include:

- `type`: open string naming the payload or schema family.

A v1 payload object MAY include:

- `encoding`: open string;
- `schema_version`: string;
- `media_type`: string;
- `bytes`: base64url without padding;
- `json`: JSON value.

The `payload.type`, `payload.encoding`, `payload.schema_version`, and
`payload.media_type` fields SHOULD be compatible with the offer's payload
descriptor, with the Get response envelope, and with metadata from a Subscribe
accept start result.

The `bytes` field is for opaque binary payloads. The `json` field is for
structured payloads. A v1 payload object SHOULD NOT include both `bytes` and
`json` unless a later RFC defines a specific dual representation.

If `bytes` is present, it uses the binary-value encoding from `RFC-0002`.

Receivers MUST reject malformed payload objects with `message.invalid_payload`.

#### Registry References

The `clock` field, when present, is a registry-reference object whose
`registry` is `clock`. Clock semantics are defined in `RFC-0035`.

The `registry_refs` field, when present, uses the same registry-reference shape
defined in `RFC-0024`.

Message-level registry references refine or confirm the registry context for
that message. For Subscribe, they MUST NOT contradict the registry references
committed by the Subscribe accept start result unless the accepted offer kind
explicitly allows per-message registry changes. For Get, they SHOULD be
compatible with the requested offer's registry references and payload
descriptor.

Spatial messages SHOULD include or inherit the frame registry references needed
to interpret positions, rotations, point coordinates, or other frame-scoped
data.

Temporal messages SHOULD include or inherit the clock registry reference needed
to interpret `timestamp_ns`.

#### Sequence And Freshness

The `sequence` field, when present, is scoped to one Subscribe stream or to the
single successful Get response. Get v1 does not use `sequence` for
continuation.

When `sequence` is present on a Subscribe stream, the producer SHOULD start at
0 or 1 and increase it by 1 for each data message on that stream.

Receivers MAY use `sequence` gaps as a diagnostic signal. A sequence gap does
not by itself prove data tampering or producer fault, because Subscribe v1
allows local drop and coalescing policies under backpressure.

The `timestamp_ns`, unresolved-clock, and `generated_at` semantics follow
`RFC-0035`.

#### Error Object

Get, Subscribe, offer-catalog diagnostics, and future spatial protocols SHOULD
use a common error object.

A v1 error object is a JSON object.

A v1 error object MUST include:

- `code`: stable string failure code.

A v1 error object MAY include:

- `message`: string;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `kind`: offer-kind string;
- `retryable`: boolean;
- `details`: JSON object.

The `message` and `details` fields follow the diagnostic-field rule in
`RFC-0002`.

The `retryable` field is advisory. Receivers MAY apply local retry policy even
when it is absent.

#### Failure Mapping

A receiver SHOULD fail malformed envelopes with `message.invalid_envelope`.

A receiver SHOULD fail malformed payloads with `message.invalid_payload`.

A receiver SHOULD fail oversized payloads or envelopes with
`message.payload_too_large`. Concrete size units and enforcement points are
defined in `RFC-0028`.

A receiver MAY report observed sequence gaps with `message.sequence_gap`.

A receiver SHOULD use the existing offer failure-code family when the envelope
or request targets an unknown, unauthorized, unsupported, unavailable, or stale
offer.

### RFC-0028: Get And Subscribe Common Path Rules

#### Requirement

Get and Subscribe requests target one offer within the producing peer
relationship.

Each request MUST include:

- `domain_id`: domain id string;
- `offer_id`: offer id string.

Each request MAY include:

- `params`: JSON object;
- `accepted_payload_types`: array of payload type strings;
- a path-specific positive integer size limit whose field name and unit are
  defined by the path RFC.

The tuple `(domain_id, offer_id)` identifies the offer within the producing
peer relationship. The producing peer id is known from the
transport-authenticated peer relationship and SHOULD NOT be repeated in the
request.

The `params` object is offer-kind-specific. Receivers MUST ignore unknown
`params` fields unless the offer kind defines them as required.

The `accepted_payload_types` array lets the requester narrow the payload types
it is willing to receive. If omitted or empty, the requester accepts any
payload type advertised by the offer and supported by local policy.

Size limits are path-specific. This RFC owns the common enforcement rule. Each
path RFC owns its size-limit field name and unit.

The v1 size units are:

- raw payload bytes: the size of the carried payload content before response
  or message envelope overhead. For a payload object with `bytes`, this is the
  decoded byte length. For a payload object with `json`, this is the UTF-8 JSON
  byte length of the serialized `payload.json` value used in the message. If
  both `bytes` and `json` are validly present, the raw payload size is the sum
  of both carried representations unless the offer-kind RFC defines a narrower
  rule.
- serialized envelope bytes: the byte length of the UTF-8 JSON serialization
  of the spatial message envelope produced by the path before transport
  framing, compression, or encryption. This size includes envelope fields and
  the payload object. It is not a canonicalization rule.

Get defines `max_payload_bytes` over raw payload bytes. Subscribe defines
`max_message_bytes` over serialized envelope bytes.

Implementations MUST define local maximum sizes for data they produce or
accept on each supported path, even when the requester omits a size field.

When a requester supplies a path-specific size limit, the responder MUST honor
the lower of the requester's limit and the responder's local production limit
for that path's size unit.

If local policy defines additional raw-payload or serialized-envelope
production or acceptance limits, the implementation MUST enforce those limits
too.

If a Get response, Subscribe data message, or received envelope exceeds an
applicable requester or local size limit, the failing side SHOULD report
`message.payload_too_large` when a structured error, reject, or end message can
be sent. If the path cannot return structured failure because transport or
framing failed first, `transport.failed` MAY be used.

Get and Subscribe failure mapping SHOULD reuse the offer failure-code family:

- `offer.unknown_offer` when the requested `(domain_id, offer_id)` is not known;
- `offer.domain_not_served` when the offer fails the served-domain part of the
  offer usability check;
- `offer.unsupported_kind` when the offer kind is known but unsupported by the
  responder's implementation for that path;
- `offer.unsupported_access_mode` when the offer does not support the requested
  path;
- `offer.unsupported_payload_type` when the responder cannot produce any
  payload type accepted by the requester;
- `offer.temporarily_unavailable` when the offer is known but unavailable;
- `offer.stale` when local freshness policy rejects the offer.

Structured message failures SHOULD use the message failure-code family from
`RFC-0027`. Transport or framing failures SHOULD use `transport.failed` when a
structured response, reject, or end message cannot be returned.

### RFC-0029: Get

#### Requirement

Get is a one-shot request-response protocol for fetching a finite representation
of one offer.

Get v1 is intentionally narrow. It is for descriptors, registry entries,
transform edges, and small snapshots. Get v1 does not define log-range fetches,
chunked responses, streaming responses, map queries, or large object transfer.

Get MUST NOT be usable for an offer unless the offer advertises `get` in
`access_modes`.

#### Request

A v1 Get request is a JSON object.

A v1 Get request MUST include:

- `type`: string, exactly `auki.get_request.v1`;
- the common request fields defined in `RFC-0028`.

A v1 Get request MAY additionally include:

- `max_payload_bytes`: positive integer.

The `max_payload_bytes` field is the Get path-specific requester size limit
defined in `RFC-0028`. It limits the raw payload bytes of a successful
response's `message.payload`. It does not limit the full Get response object or
the spatial message envelope.

The responder MUST NOT send a successful Get response whose raw payload bytes
exceed the applicable limit computed in `RFC-0028`.

#### Response

A v1 Get response is a JSON object.

A v1 Get response MUST include:

- `type`: string, exactly `auki.get_response.v1`.

A successful v1 Get response MUST include:

- `message`: spatial message envelope object.

A failed v1 Get response MUST include:

- `error`: error object as defined in `RFC-0027`.

A v1 Get response MUST include exactly one of `message` or `error`.

The `message` object MUST follow the spatial message envelope shape defined in
`RFC-0027`.

The `message.domain_id` and `message.offer_id` MUST match the request
`domain_id` and `offer_id`.

The response MUST be a complete response for the request. Get v1 has no
continuation token, chunk list, or streaming body.

#### Snapshot Semantics

A Get response represents the producer's best available snapshot at response
time.

The producer SHOULD set `message.generated_at` when it can report when the
snapshot was generated. The field follows `RFC-0035`.

The producer SHOULD set `message.timestamp_ns` and an applicable clock
reference when the returned data represents domain data observed at a specific
producer clock time. These fields follow `RFC-0035`.

The producer SHOULD include or inherit registry references needed to interpret
the returned payload.

Get does not create a subscription and does not reserve future availability.
A later Get for the same offer MAY return different data or fail if the offer
status changes.

#### Size Limits

Get v1 is for small finite responses.

Get uses raw payload bytes as its path-specific size unit. Serialized response
or envelope local limits may also apply under `RFC-0028`.

Get v1 MUST NOT split a response into multiple chunks.

#### First Use Cases

The first Get use cases are:

- `registry_entry`: return the exact registry entry identified by the offer's
  registry reference;
- `transform_edge`: return one direct transform edge;
- descriptor or small snapshot offers explicitly advertised with `get`.

Log ranges, map fragments, and large binary artifacts are future work unless a
later RFC defines chunking or a separate transfer protocol.

#### Failure Mapping

A responder SHOULD fail malformed Get requests with `get.invalid_request`.

Get SHOULD use the common offer-path failure mapping defined in
`RFC-0028`.

A requester SHOULD use the message failure-code family from `RFC-0027` when a
Get response includes `message` but the envelope, payload, or payload size is
invalid.

### RFC-0030: Subscribe

#### Requirement

Subscribe is a request-accept-message-end protocol for receiving live updates
from one offer.

Subscribe v1 is intentionally narrow. It is for live updates from an already
advertised offer. It does not define historical replay, log-range fetches,
exactly-once delivery, reliable delivery, resumable streams, stream forwarding,
map queries, or large object transfer.

Subscribe MUST NOT be usable for an offer unless the offer advertises
`subscribe` in `access_modes`.

#### Request

A v1 Subscribe request is a JSON object.

A v1 Subscribe request MUST include:

- `type`: string, exactly `auki.subscribe_request.v1`;
- the common request fields defined in `RFC-0028`.

A v1 Subscribe request MAY additionally include:

- `max_message_bytes`: positive integer.

The `max_message_bytes` field is the Subscribe path-specific requester size
limit defined in `RFC-0028`. It limits the serialized spatial message envelope
bytes for each data message.

The producer MUST NOT send a data message whose serialized spatial message
envelope bytes exceed the applicable limit computed in `RFC-0028`.

#### Start Result

A producer MUST send exactly one start result before sending data messages.

A successful v1 start result is a JSON object.

A successful v1 start result MUST include:

- `type`: string, exactly `auki.subscribe_accept.v1`;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `payload`: payload descriptor object.

A successful v1 start result MAY include:

- `registry_refs`: array of registry-reference objects;
- `initial_sequence`: non-negative integer;
- `generated_at`: timestamp;
- `metadata`: JSON object.

The `domain_id` and `offer_id` fields MUST match the request.

The `payload` descriptor commits the payload family the producer will send for
this subscription. It SHOULD be compatible with the offer's payload descriptor
and with the requester's `accepted_payload_types`.

The `registry_refs` array commits the registry context for the subscription.
When present, data messages on the subscription MUST NOT contradict it unless
the accepted offer kind explicitly allows per-message registry changes.

The `initial_sequence` field, when present, declares the first expected
`message.sequence` value for the subscription.

The `generated_at` field follows `RFC-0035`.

A failed v1 start result is a JSON object.

A failed v1 start result MUST include:

- `type`: string, exactly `auki.subscribe_reject.v1`;
- `error`: error object as defined in `RFC-0027`.

If the request's `domain_id` and `offer_id` can be parsed, the `error` object
SHOULD include them.

The producer MUST NOT send data messages after a failed start result.

#### Data Messages

After a successful start result, each data message MUST follow the spatial
message envelope shape defined in `RFC-0027`.

Each data message MUST have `domain_id` and `offer_id` matching the accepted
subscription.

The data message payload fields SHOULD be compatible with the accepted
subscription payload descriptor.

Sequence behavior follows `RFC-0027`, using `initial_sequence` when present.

#### End Message

A v1 subscription MAY end by transport close, stream close, or an explicit end
message.

A requester MAY cancel a subscription by closing the stream or sending an end
message with reason `cancelled`.

A v1 end message is a JSON object.

A v1 end message MUST include:

- `type`: string, exactly `auki.subscribe_end.v1`;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `reason`: string.

A v1 end message MAY include:

- `error`: error object as defined in `RFC-0027`;
- `retryable`: boolean;
- `details`: JSON object.

The v1 end reason values are:

- `complete`: the producer intentionally completed the subscription;
- `cancelled`: the requester cancelled the subscription;
- `offer_withdrawn`: the offer is no longer available for this subscription;
- `not_authorized`: the subscription is no longer authorized;
- `producer_shutdown`: the producer is shutting down or leaving;
- `error`: the subscription ended because of an error described by `error`.

An end message does not prove that all prior data was delivered. It only
reports the producer's or requester's local end reason.

#### Backpressure And Delivery

Subscribe v1 does not guarantee delivery, ordering across subscriptions, or
exactly-once processing.

Subscribe uses serialized spatial message envelope bytes as its path-specific
size unit. Subscribe message size handling follows `RFC-0028`.

When a producer or receiver is overloaded, it MAY drop data messages, apply
local coalescing, or end the subscription. If dropping creates an observable
`sequence` gap, the receiver MAY report `message.sequence_gap`.

Offer kinds that need reliable history, replay, resume, or chunking MUST define
that behavior in a later RFC.

#### Reconnect

After transport loss or subscription end, a requester MAY open a new Subscribe
request for the same `(domain_id, offer_id)` if the offer is still usable.

Subscribe v1 has no resume token and no replay cursor. A new subscription is a
new live stream, not a continuation of the old stream.

Before reconnecting, the requester SHOULD re-check that the peer relationship
still exists and `RFC-0026` still permits Subscribe.

#### Failure Mapping

A responder SHOULD fail malformed Subscribe requests with
`subscribe.invalid_request`.

Subscribe SHOULD use the common offer-path failure mapping defined in
`RFC-0028`.

A receiver SHOULD use the message failure-code family from `RFC-0027` when a
Subscribe data message includes a spatial message envelope but the envelope,
payload, message size, or observed sequence behavior is invalid.

### RFC-0031: Minimum Offer Kinds

#### Requirement

The v1 minimum offer-kind set is:

- `sensor_stream`;
- `transform_edge`;
- `registry_entry`.

Offer kinds are open strings. Implementations MAY advertise additional kinds,
and consumers MUST ignore unknown kinds unless local application code supports
them.

The minimum set is intentionally small. It covers the current baseline of live
sensor data, exact metadata lookup, and direct frame transforms without
requiring maps, generic spatial queries, payments, bookings, or application
marketplace semantics.

#### `sensor_stream`

A `sensor_stream` offer represents live data produced by one sensor-like data
source.

A `sensor_stream` offer MUST include:

- `subscribe` in `access_modes`;
- a `sensor` registry reference;
- a payload descriptor that lets consumers choose a decoder.

A `sensor_stream` offer SHOULD include:

- a `clock` registry reference when the producer knows the stream clock at
  catalog time;
- a `frame` registry reference for spatial sensors when the producer knows the
  sensor frame at catalog time.

A Subscribe implementation for `sensor_stream` MUST commit to the exact sensor,
clock, and frame references for that subscription when those references are
required to interpret messages.

Examples of sensor stream payloads include camera frames, point-cloud frames,
audio frames, and joint-encoder frames. The offer kind does not imply a single
payload format; the payload descriptor and registry references define
interpretation.

#### `transform_edge`

A `transform_edge` offer represents a direct transform between two frame
registry entries.

A `transform_edge` offer MUST include:

- `get` in `access_modes`;
- a `from_frame` registry reference;
- a `to_frame` registry reference;
- a payload descriptor for a spatial transform payload.

A `transform_edge` offer MAY include `subscribe` in `access_modes` when the
transform is expected to change over time.

Static or rigid transforms SHOULD be represented as `transform_edge` offers
before introducing a pose-stream or pose-log-range offer kind.

#### `registry_entry`

A `registry_entry` offer represents one exact content-addressed registry entry
that the producer is willing to serve.

A `registry_entry` offer MUST include:

- `get` in `access_modes`;
- exactly one `registry_refs` entry identifying the registry entry.

A `registry_entry` offer MUST NOT include `subscribe` in `access_modes` in v1.

Registry entries are immutable by hash. A changed registry entry is a different
offer target and SHOULD use a different `offer_id` or a changed registry
reference in a later catalog snapshot.

#### Future Kinds

The following are expected future offer kinds, but are not part of the v1
minimum set:

- `pose_stream`;
- `pose_log_range`;
- `time_transform`;
- `detection_stream`;
- `map_fragment`;
- `spatial_query`.

This list is planning context, not a v1 interoperability requirement. Future
kind definitions can reference the existing owner RFCs for baseline offer
behavior:

- domain scope is owned by `RFC-0025`;
- unknown-kind handling is owned by `RFC-0024` and this RFC;
- offer usability is owned by `RFC-0026`.

## Compatibility And Observability

### RFC-0032: Protocol Versions Are Compatibility Contracts

#### Requirement

A protocol ID, such as `/auki/example/0.0.1`, identifies a wire contract
between implementations. Once a protocol version is used by deployed peers,
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

### RFC-0033: Observability Must Explain State Transitions

#### Requirement

Implementations MUST make core lifecycle state explainable without noisy
per-frame logs.

Diagnostics SHOULD answer:

- local peer, Discovery, and advertised-address state;
- local domains and their authority material;
- remote peers, how they were learned, and their lifecycle state;
- accepted domains, rejected domains, loaded offers, and active paths;
- why identity, authority, policy, offer, Get, Subscribe, or message validation
  failed;
- Discovery degradation independently from peer relationship degradation.

#### Consequences

Heartbeat-frame logs, stream-frame logs, and repeated dial retry logs SHOULD be
rate-limited or omitted by default.

State transitions and failures SHOULD be logged once with enough context to
debug the lifecycle.

`RFC-0034` defines the concrete diagnostic status snapshot shape for
implementations that expose status as structured data.

### RFC-0034: Status And Observability API

#### Requirement

Implementations MUST expose a status surface that explains the local peer's
current lifecycle state, peer relationships, served domains, offer loading, and
active or recently completed spatial-data paths.

The status surface is diagnostic state under `RFC-0001`. Protocol validation
and the authority rules in `RFC-0001` remain the source of authority.

The status surface MAY be exposed through an in-process API, a local debug
endpoint, logs, a CLI, or another implementation-defined surface. When exposed
as JSON, it SHOULD use the v1 status snapshot shape defined here.

Status output MUST NOT include private keys, wallet seed material,
authorization secrets, invite secrets, or bearer tokens. Implementations MAY
redact addresses, labels, or metadata according to local privacy policy.

#### Status Snapshot

A v1 status snapshot is a JSON object.

A v1 status snapshot MUST include:

- `type`: string, exactly `auki.status_snapshot.v1`;
- `generated_at`: timestamp;
- `local_peer`: local peer status object;
- `local_domains`: array of local domain status objects;
- `remote_peers`: array of remote peer status objects;
- `active_paths`: array of active or recently completed path status objects;
- `last_failures`: array of failure record objects.

A v1 status snapshot MAY include:

- `discovery`: Discovery status object;
- `metadata`: JSON object.

The snapshot is best-effort diagnostic state at `generated_at`. It is not a
transactional view of the network. The `generated_at` field follows `RFC-0035`.

Arrays in the status snapshot MAY be empty.

#### Local Peer Status

A local peer status object SHOULD include:

- `peer_id`: local libp2p peer id string;
- `wallet_public_key`: base64url wallet public key string when available;
- `peer_binding_issued_at`: timestamp;
- `peer_binding_age_ms`: non-negative integer when known;
- `peer_binding_fresh`: boolean under local freshness policy;
- `authorization_mode`: string;
- `listen_addresses`: array of listen multiaddr strings;
- `advertised_addresses`: array of advertised multiaddr strings.

The `peer_binding_fresh` field is a local diagnostic decision. A remote peer
MUST still verify the presented peer binding itself.

#### Local Domain Status

A local domain status object SHOULD include:

- `domain_id`: domain id string;
- `role`: string, one of `owner`, `delegate`, or `managed`;
- `declaration_present`: boolean;
- `declaration_valid`: boolean or null when not evaluated;
- `delegation_present`: boolean;
- `delegation_valid`: boolean or null when not required or not evaluated;
- `delegation_scopes`: array of delegation-scope strings;
- `delegation_expires_at`: timestamp or null;
- `advertised`: boolean;
- `serving_offers`: boolean;
- `last_failure`: failure record object or null.

This object reports the local peer's view of domains it owns, manages, serves,
or intends to advertise. Remote peers still validate domains through
`RFC-0009`.

The `role` field is diagnostic. The `managed` role means the local peer tracks
the domain locally without claiming owner or delegate authority in that status
entry.

#### Discovery Status

A Discovery status object SHOULD include:

- `enabled`: boolean;
- `discoverable`: boolean;
- `advertised_domains`: array of domain id strings;
- `advertised_addresses`: array of multiaddr strings;
- `last_refresh_at`: timestamp or null;
- `expires_at`: timestamp or null;
- `degraded`: boolean;
- `last_failure`: failure record object or null.

Discovery degradation MUST be reported independently from peer relationship
degradation.

#### Remote Peer Status

A remote peer status object SHOULD include:

- `peer_id`: remote libp2p peer id string;
- `learned_from`: string, such as `discovery`, `configured`, `invitation`, or
  `peer_graph_hint`;
- `dialable`: boolean or null when unknown;
- `connected`: boolean;
- `lifecycle_state`: string compatible with the state model in `RFC-0021`;
- `selected_protocol_version`: string or null;
- `authorized`: boolean or null when authorization has not completed;
- `verified_wallet_public_key`: base64url wallet public key string or null;
- `accepted_served_domains`: array of domain id strings;
- `rejected_domains`: array of rejected domain status objects;
- `offer_catalog_status`: offer-catalog status object;
- `loaded_offers`: array of offer status objects;
- `last_failure`: failure record object or null.

A rejected domain status object SHOULD include:

- `domain_id`: domain id string;
- `code`: stable failure code string;
- `message`: diagnostic string, optional.

An offer-catalog status object SHOULD include:

- `path_available`: boolean;
- `last_fetch_at`: timestamp or null;
- `last_success_at`: timestamp or null;
- `last_failure`: failure record object or null.

#### Offer Status

An offer status object SHOULD include:

- `peer_id`: producing peer id string;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `kind`: offer-kind string;
- `status`: offer status string;
- `access_modes`: array of access-mode strings;
- `payload_type`: payload type string when known;
- `registry_refs`: array of registry-reference summary objects;
- `usable`: boolean;
- `unusable_reason`: stable failure code string or null;
- `updated_at`: timestamp or null;
- `expires_at`: timestamp or null;
- `last_failure`: failure record object or null.

The `usable` field is local policy and compatibility state under the offer
usability rules in `RFC-0026`.

Registry-reference summary objects SHOULD include the `registry`, `role`, `id`,
and `hash` fields from the registry-reference shape defined in `RFC-0024`.

#### Path Status

A path status object SHOULD include:

- `path_id`: implementation-defined local id string;
- `path_type`: string, one of `get` or `subscribe`;
- `peer_id`: producing peer id string;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `state`: string;
- `started_at`: timestamp;
- `last_message_at`: timestamp or null;
- `payload_type`: payload type string when known;
- `last_sequence`: non-negative integer or null;
- `sequence_gap_count`: non-negative integer;
- `last_envelope_failure`: failure record object or null;
- `last_payload_failure`: failure record object or null;
- `last_failure`: failure record object or null.

For `get` paths, `state` SHOULD distinguish at least `requested`, `succeeded`,
and `failed`.

For `subscribe` paths, `state` SHOULD distinguish at least `starting`, `active`,
`ending`, `ended`, and `failed`.

Completed Get paths and ended subscriptions MAY remain in `active_paths` for a
bounded diagnostic history according to local policy.

#### Failure Records

A failure record object SHOULD include:

- `code`: stable failure code string;
- `at`: timestamp;
- `scope`: string, such as `peer`, `domain`, `offer_catalog`, `offer`, `get`,
  `subscribe`, `message`, or `discovery`;
- `peer_id`: peer id string, optional;
- `domain_id`: domain id string, optional;
- `offer_id`: offer id string, optional;
- `path_id`: path id string, optional;
- `retryable`: boolean, optional;
- `message`: diagnostic string, optional;
- `details`: JSON object, optional.

The `code` field SHOULD use the stable failure codes defined in `RFC-0010` or
by the RFC that owns the failing path.

The `message` and `details` fields follow the diagnostic-field rule in
`RFC-0002`.

#### Update Semantics

Implementations SHOULD update the status surface when:

- peer binding validation succeeds or fails;
- peer authorization succeeds or fails;
- a domain is accepted or rejected;
- Discovery advertisement refresh succeeds, expires, or fails;
- a peer relationship changes lifecycle state;
- offer catalog loading succeeds or fails;
- an offer becomes loaded, updated, stale, withdrawn, usable, or unusable;
- a Get request starts, succeeds, or fails;
- a Subscribe path starts, becomes active, observes a sequence gap, ends, or
  fails;
- a spatial message envelope or payload is rejected.

Repeated high-frequency message events SHOULD be aggregated. Implementations
SHOULD expose counters and last failure records instead of unbounded per-frame
logs by default.

Implementations SHOULD bound retained `last_failures` and completed path
history according to local policy.
