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
- issued_at timestamp;
- signature by the wallet authority key.

The wallet authority key and the libp2p peer key are separate protocol roles.
The libp2p peer key authenticates the transport connection. The wallet
authority key signs the peer binding that authorizes that runtime peer id.

A peer binding signature MUST be verified against the wallet public key, not
against the libp2p peer public key derived from the connection. A libp2p peer
signature MUST NOT be accepted as a substitute for a wallet-signed peer binding.

Deployments SHOULD use distinct key material for wallet authority and libp2p
peer identity. A deployment that intentionally reuses key material loses that
key-compromise separation, but the protocol still treats the two roles
separately.

A peer binding MAY include a metadata label.

A peer binding MAY be reused across sessions.

A receiver MAY enforce a maximum accepted binding age from `issued_at`.

A receiver that enforces a maximum accepted binding age MUST reject a peer
binding whose `issued_at` is older than that limit.

The recommended default maximum accepted binding age is 1 hour.

Peers that rely on fresh peer bindings SHOULD refresh them before half of the
maximum accepted binding age has elapsed.

A peer refreshes a peer binding by producing a new wallet-signed peer binding
for the same wallet public key and peer id with a newer `issued_at`.

Refreshing a peer binding does not change domain ownership, delegation, served
domain validation, or offer authority by itself.

A receiver MAY accept older peer bindings according to local policy.

#### Verification

When a peer presents a peer binding, the receiver MUST verify that:

- the peer binding is well-formed;
- the signature verifies against the wallet public key;
- the connected libp2p peer id matches the bound peer id;
- `issued_at` satisfies local freshness policy.

#### Consequences

A peer binding proves only that the wallet recognizes the connected libp2p
peer id as one of its runtime peers.

A peer binding MUST NOT be treated as proof of domain ownership, runtime
authority, data correctness, or dialability at any advertised address.

### RFC-0002: Peer Binding Schema

#### Requirement

A v1 peer binding is a JSON object.

A v1 peer binding MUST include:

- `type`: string, exactly `auki.peer_binding.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `wallet_public_key`: base64url without padding, containing the raw 32-byte
  Ed25519 wallet public key;
- `peer_id`: string, containing a standard libp2p PeerId text representation;
- `issued_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `signature`: base64url without padding, containing the raw 64-byte Ed25519
  signature.

The `wallet_signature_scheme` field determines the wallet public-key encoding,
signature encoding, signature length, and verification algorithm.

A v1 peer binding MAY include:

- `label`: string, for operator metadata only.

The `label` field has no identity, authority, delegation, reachability, or
policy semantics.

#### Signed Bytes

The signed bytes are the RFC 8785 JSON Canonicalization Scheme output for the
whole peer binding object with only the `signature` field removed.

The `type` field is part of the signed bytes and is the domain separator for
v1 peer bindings.

Unknown fields MAY be present. A receiver MUST include unknown fields in the
canonical signed bytes before signature verification, and MUST ignore unknown
fields after verification unless a later RFC defines them.

An implementation MUST NOT normalize the `peer_id` string, reformat
`issued_at`, drop unknown fields, or change base64url spelling before
canonicalizing the signed bytes. The signature verifies the JSON values as
presented, minus only the `signature` field.

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

### RFC-0003: Domain Identity And Ownership

#### Requirement

A domain MUST have a stable domain id that can be verified without Discovery,
blockchain access, or any online registry.

A domain id MUST be derived from the domain owner wallet's public key and a
nonce:

domain_id = hash(domain_owner_wallet_public_key, nonce)

The concrete v1 hash input, hash function, and domain id encoding are defined
in `RFC-0004`.

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

Discovery may help locate peers that claim to serve a domain, but Discovery
does not create domain ownership or prove runtime authority.

### RFC-0004: Domain Declaration Schema

#### Requirement

A v1 domain declaration is a JSON object.

A v1 domain declaration MUST include:

- `type`: string, exactly `auki.domain_declaration.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `domain_id`: base64url without padding, containing the raw 32-byte v1 domain
  id;
- `domain_owner_public_key`: base64url without padding, containing the raw
  32-byte Ed25519 domain owner wallet public key;
- `nonce`: base64url without padding, containing the raw 16-byte domain nonce;
- `signature`: base64url without padding, containing the raw 64-byte Ed25519
  signature.

The `wallet_signature_scheme` field determines the wallet public-key encoding,
signature encoding, signature length, and verification algorithm.

A v1 domain declaration MAY include:

- `label`: string, for operator metadata only.

The `label` field has no ownership, authority, delegation, reachability, or
policy semantics.

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

The signed bytes are the RFC 8785 JSON Canonicalization Scheme output for the
whole domain declaration object with only the `signature` field removed.

The `type` field is part of the signed bytes and is the domain separator for
v1 domain declarations.

Unknown fields MAY be present. A receiver MUST include unknown fields in the
canonical signed bytes before signature verification, and MUST ignore unknown
fields after verification unless a later RFC defines them.

An implementation MUST NOT normalize `domain_id`, `domain_owner_public_key`,
`nonce`, drop unknown fields, or change base64url spelling before
canonicalizing the signed bytes. The signature verifies the JSON values as
presented, minus only the `signature` field.

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

Domain declaration verification MUST NOT require Discovery, blockchain access,
registry access, online revocation lookup, or any other online lookup.

#### Failure Mapping

A receiver SHOULD fail malformed domain declarations with
`domain.invalid_declaration`. This includes missing required fields,
unsupported `type`, unsupported wallet signature scheme, malformed base64url,
wrong field length for the declared scheme, malformed nonce, or an invalid
signature.

A receiver SHOULD fail a declaration whose recomputed domain id does not match
the declared `domain_id` with `domain.id_mismatch`.

### RFC-0005: Domain Delegation Schema

#### Requirement

A v1 domain delegation is a JSON object.

A v1 domain delegation MUST include:

- `type`: string, exactly `auki.domain_delegation.v1`;
- `wallet_signature_scheme`: string, exactly `ed25519`;
- `domain_id`: base64url without padding, containing the raw 32-byte domain
  id being delegated;
- `domain_owner_public_key`: base64url without padding, containing the raw
  32-byte Ed25519 domain owner wallet public key;
- `delegate_wallet_public_key`: base64url without padding, containing the raw
  32-byte Ed25519 wallet public key from the delegate peer binding;
- `delegate_peer_id`: string, containing the delegate's standard libp2p PeerId
  text representation;
- `scopes`: non-empty array of strings;
- `valid_from`: UTC RFC3339 timestamp string with a `Z` suffix;
- `expires_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `signature`: base64url without padding, containing the raw 64-byte Ed25519
  signature by the domain owner wallet.

The `wallet_signature_scheme` field determines the wallet public-key encoding,
signature encoding, signature length, and verification algorithm.

A v1 domain delegation MAY include:

- `label`: string, for operator metadata only.

The `label` field has no ownership, authority, reachability, or policy
semantics.

The v1 delegation scopes are exactly:

- `advertise`: the peer may announce the domain through Discovery,
  peer-discovery metadata, or equivalent reachability surfaces;
- `serve`: the peer may declare the domain during handshake and serve offers
  or spatial data scoped to that domain;
- `update`: the peer may publish or apply domain-scoped updates where a later
  RFC defines an update path.

The `scopes` array MUST contain only v1 delegation scopes and MUST NOT contain
duplicates.

Before signing, producers MUST sort `scopes` in alphabetical string order.
Receivers MUST verify the signature against the `scopes` array exactly as
presented.

The `expires_at` timestamp MUST be later than `valid_from`.

#### Signed Bytes

The signed bytes are the RFC 8785 JSON Canonicalization Scheme output for the
whole domain delegation object with only the `signature` field removed.

The `type` field is part of the signed bytes and is the domain separator for
v1 domain delegations.

Unknown fields MAY be present. A receiver MUST include unknown fields in the
canonical signed bytes before signature verification, and MUST ignore unknown
fields after verification unless a later RFC defines them.

An implementation MUST NOT normalize `domain_id`, `delegate_peer_id`,
timestamps, `scopes` array order, drop unknown fields, or change base64url
spelling before canonicalizing the signed bytes. The signature verifies the
JSON values as presented, minus only the `signature` field.

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

Domain delegation verification MUST NOT require Discovery, blockchain access,
registry access, online revocation lookup, or any other online lookup.

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

### RFC-0006: Authority Chain Validation

#### Requirement

After a transport connection is established, each peer MUST validate the remote
peer's authority chain before treating any remote offer as usable.

Authority-chain validation MUST run in this order:

1. Verify that the remote peer binding is well-formed.
2. Verify that the peer binding signature is valid for the bound wallet public
   key.
3. Verify that the transport-authenticated libp2p peer id matches the peer id
   in the remote peer binding.
4. Apply local freshness policy for `issued_at`, including maximum accepted
   binding age and future-timestamp tolerance.
5. Run peer authorization for the verified peer identity.
6. Validate each declared domain independently.
7. Compute the accepted served domain set for the peer relationship.

Peer authorization is defined in `RFC-0016`. In the authority-chain validation
path, peer authorization runs after peer binding verification and before served
domains are accepted.

Offer loading happens after authority-chain validation. When offers are loaded,
the receiver MUST accept only offer catalog entries whose `domain_id` is in the
accepted served domain set for that peer relationship.

Domain validation MUST verify the domain declaration for each declared domain.
The receiver MUST recompute the domain id from the declared domain owner wallet
public key and nonce, verify the declaration signature, and reject the declared
domain if the recomputed domain id does not match.

A declared domain is directly accepted when the verified peer wallet is the
domain owner wallet.

A declared domain is accepted through delegation when the peer presents a valid
delegation from the domain owner wallet that authorizes the verified peer
identity to serve that domain. A receiver MUST reject an expired, malformed, or
wrong-domain delegation.

A receiver MUST reject a delegation that does not authorize the claimed action.
The v1 delegation scopes are `advertise`, `serve`, and `update`.

For served-domain validation, the claimed action is `serve`. For Discovery,
peer-discovery metadata, or equivalent reachability advertisement, the claimed
action is `advertise`. The `update` scope is checked only by protocols that
define domain-scoped update behavior.

Domain authority validation answers whether the remote peer may serve under a
domain. It does not decide whether the local application wants to consume that
domain or any offer from it.

After domain authority validates, domain access policy MAY still reject the
domain with `policy.domain_rejected`.

Validating one declared domain MUST NOT cause another declared domain from the
same peer to be accepted. Each declared domain needs its own valid authority
chain.

The v1 authority validation path MUST NOT require online revocation lookup,
blockchain access, registry access, or Discovery access. Maximum accepted
binding age is the baseline mechanism for aging out peer bindings. Delegation
expiry and replacement are the baseline mechanisms for aging out delegations.

#### Failure Codes

Lifecycle, authority, offer-loading, Get, and Subscribe diagnostics SHOULD use
stable string failure codes in `category.reason` form.

Baseline failure codes:

- `protocol.unsupported_version`
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

#### Consequences

Authority-chain validation proves only that a connected peer is authorized to
serve the accepted domains it declared. It does not prove that the peer's data
is correct, canonical, complete, or trusted.

Invalid identity material is a peer-level failure. Invalid domain authority is
a domain-level failure unless peer authorization or local policy chooses to
reject the whole peer relationship.

## Peer And Domain Model

### RFC-0007: Serving Peers Declare Domains

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

### RFC-0008: Served Domain Set

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

#### Offer Interaction

The served domain set is the authority filter for remote offers.

An offer whose domain is in the accepted served domain set MAY be loaded,
displayed, requested by Get, or requested by Subscribe, subject to local
domain access policy and offer policy.

An offer whose domain is not in the accepted served domain set MUST NOT be
treated as usable. Implementations SHOULD reject or ignore such offers with
`offer.domain_not_served`.

If offer loading fails for an accepted served domain, the peer relationship MAY
remain ready while reporting `offer.load_failed` for that offer-loading path.

#### Dynamic Updates (To Fill)

Future protocol work should define how a peer adds, removes, refreshes, or
replaces served domains during an active peer relationship.

Any dynamic served-domain update MUST rerun the same authority-chain validation
rules used during the initial handshake.

Until a dynamic update protocol is defined, implementations SHOULD treat served
domain changes as requiring a reconnect or fresh handshake.

#### Diagnostics

Diagnostics SHOULD report:

- each declared domain id;
- whether the declared domain was accepted or rejected;
- the failure code for each rejected domain;
- whether the peer relationship has an empty served domain set;
- which loaded offers are scoped to each accepted served domain.

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

A peer that advertises a domain on behalf of another wallet MUST have a valid
delegation with `advertise` scope. A peer that controls the domain owner wallet
MAY advertise that domain directly.

Receiving an advertisement MUST NOT by itself cause the receiver to accept the
advertised peer as a server for that domain. Served-domain acceptance still
requires peer-to-peer authority validation with `serve` authority.

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

### RFC-0015: Peer Handshake

#### Requirement

After dialing and establishing a transport connection, peers MUST run a
symmetric handshake before loading offers or exchanging spatial data.

The handshake is symmetric because either peer may be a producer, consumer, or
both. Each side MUST be able to present identity, supported protocol versions,
authorization material, and any domains it claims to serve.

Each handshake side MUST include:

- supported lifecycle protocol versions;
- peer binding;
- peer authorization material, when required by local policy;
- declared domains, domain declarations, and delegations, when the peer claims
  to serve domains;
- offer-catalog fetch path, when the peer exposes offers;
- liveness or non-authoritative diagnostic initialization data, when supported.

The transport-authenticated libp2p peer id is the source of truth for the
remote peer id. Any peer id carried inside handshake material is a claim that
MUST be checked against the transport-authenticated peer id. It MUST NOT
override the transport-authenticated peer id.

The libp2p Identify protocol MAY provide peer metadata such as public keys,
agent versions, and listen addresses. Identify metadata MUST NOT override the
transport-authenticated remote peer id and MUST NOT satisfy the wallet-signed
peer binding requirement.

Each side MUST choose the highest mutually supported lifecycle protocol version.
If no compatible lifecycle protocol version exists, the peer relationship MUST
fail with `protocol.unsupported_version`.

A peer that only consumes remote offers MAY send no declared domains and no
offer-catalog fetch path.

A peer that exposes a domain-scoped offer catalog MUST declare the domains it
may use in offers from that catalog.

The receiver MUST NOT treat offers from that catalog as usable unless each
offer's domain is in the receiver's accepted served domain set for that peer
relationship.

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

The connection MUST NOT load remote offers before peer identity is verified,
peer authorization succeeds, and the served domain set is computed.

The connection MAY become ready with an empty remote served domain set. In that
case, the remote peer is connected and authorized but exposes no usable remote
offers for that relationship.

#### Happy Path Example

1. Park dials Robot.
2. Park and Robot exchange supported lifecycle protocol versions.
3. Robot presents a peer binding.
4. Robot declares a served domain and presents the domain declaration.
5. Robot presents a delegation if Robot's verified wallet is not the domain
   owner wallet.
6. Park verifies Robot's peer binding against Robot's transport-authenticated
   libp2p peer id.
7. Park authorizes Robot under peer authorization mode `all`.
8. Park validates Robot's declared domain authority.
9. Park accepts the domain into Robot's served domain set.
10. Park fetches Robot's offer catalog.
11. Park may Get or Subscribe to offers scoped to the accepted served domain.

#### Authority Examples

Direct owner:

1. Robot's verified peer binding wallet public key equals the domain owner
   public key in the verified domain declaration.
2. Park accepts Robot as directly authorized to serve the domain.
3. Robot does not need to present a delegation for that domain.

Delegated server:

1. Robot's verified peer binding wallet public key differs from the domain
   owner public key.
2. Robot presents a delegation signed by the domain owner wallet.
3. Park verifies that the delegation binds the domain id, domain owner public
   key, Robot's wallet public key, Robot's transport-authenticated libp2p peer
   id, a valid time window, and the `serve` scope.
4. Park accepts the domain into Robot's served domain set.

#### Failure Path Examples

Identity failure:

1. Park dials Robot and establishes a libp2p transport connection.
2. Robot presents a peer binding for a different libp2p peer id than the
   transport-authenticated peer id.
3. Park rejects the peer relationship with `identity.peer_id_mismatch`.
4. Park MUST NOT validate Robot's declared domains or load Robot's offers.

Partial domain acceptance:

1. Robot declares domains `A`, `B`, and `C`.
2. Park validates `A` directly because Robot's verified wallet is the domain
   owner wallet.
3. Park validates `B` through a valid delegation.
4. Park rejects `C` because the delegation is expired, reporting
   `domain.expired_delegation`.
5. Park keeps the peer relationship and records Robot's served domain set as
   `{A, B}`.
6. Park MUST NOT treat offers scoped to `C` as usable.

Policy and offer rejection:

1. Robot declares domain `A` and proves valid domain authority.
2. Park's domain access policy rejects `A` with `policy.domain_rejected`.
3. Park keeps the peer relationship if peer authorization succeeded.
4. Park does not add `A` to Robot's served domain set.
5. If Robot's offer catalog includes an offer scoped to `A`, Park rejects or
   ignores that offer with `offer.domain_not_served`.

### RFC-0016: Authorization Model

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

### RFC-0017: Peer Connectivity State Is Tracked Per Remote Peer

#### Requirement

A peer SHOULD track connectivity and readiness state independently for each
remote peer.

Failure of one peer relationship MUST NOT force unrelated peer relationships to
restart or become invalid.

#### Candidate State Model

The following states are non-normative names, but the SDK SHOULD expose
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
peers after connecting, authorizing, and computing the remote peer's accepted
served domain set.

Discovery MUST NOT be required as the transport for spatial data exchange,
and MUST NOT be treated as the authoritative offer registry.

#### Offers

An offer is a connected peer's declaration of one named and typed data item it
is willing to serve.

Offer ids are scoped to the producing peer's served domain. They identify data
the producer exposes from that domain, not global network objects.

An offer SHOULD provide enough information for a consumer to decide whether it
can use the data and whether to fetch it once or subscribe to it:

- offer id and domain id;
- display name, when useful;
- offer kind;
- payload or schema type and version;
- supported access mode: Get, Subscribe, or both;
- registry references for spatial and temporal interpretation, when relevant;
- freshness or availability status.

An offer MUST NOT by itself be treated as proof of authority, correctness, or
trustworthiness. It is a reference to data exposed from a domain.

#### Get

`Get` fetches an offered data item once.

Get is for finite responses. In v1, `RFC-0023` narrows Get to descriptors,
registry entries, transform edges, and small snapshots. Future RFCs MAY extend
Get to log ranges, map fragments, or other finite spatial-data representations.

A failed `Get` SHOULD explain whether the offer was unknown, unauthorized,
stale, unavailable, unsupported, or failed at the transport/protocol layer.

#### Subscribe

`Subscribe` receives live updates from an offered data item. Examples include
a camera stream, point-cloud stream, pose stream, audio stream, or future live
map updates.

A subscription failure SHOULD explain whether the offer was unknown,
unauthorized, stale, unavailable, unsupported, or failed at the
transport/protocol layer.

#### Current Implementation Mapping

Current `/auki/resources/0.0.1` resource rows, `/auki/registries/0.0.1`
content-addressed registry fetches, and `/auki/stream/0.1.0` typed streams are
implementation examples.

This RFC does not require those protocol names or exact wire shapes to be the
final Offer / Get / Subscribe contract.

#### Consequences

The SDK SHOULD support a peer learning what another peer can share by name or
type before opening a stream or fetching data.

### RFC-0020: Offer Catalog

#### Requirement

An offer catalog is a peer-to-peer snapshot of the offers a connected peer is
willing to expose to the requester at the time of the request.

The offer catalog is runtime metadata. It is not a signed authority object.
Offer authority is derived from the peer relationship's accepted served domain
set, as defined in `RFC-0021`.

A peer that exposes one or more domain-scoped offers MUST declare an
offer-catalog fetch path during handshake.

A peer that consumes remote offers SHOULD fetch the remote offer catalog only
after peer identity is verified, peer authorization succeeds, and the remote
served domain set has been computed.

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

- `generated_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `diagnostics`: array of diagnostic objects.

The `offers` array MAY be empty. An empty array means the responder understood
the request but has no matching offers currently visible to the requester.

The `generated_at` timestamp is producer metadata. It can help diagnostics and
freshness decisions, but it MUST NOT be treated as authority proof.

#### Diagnostics

A v1 offer-catalog diagnostic uses the v1 error object defined in `RFC-0022`.

A v1 offer-catalog diagnostic MUST include:

- `code`: string.

A v1 offer-catalog diagnostic MAY include:

- `message`: string;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `kind`: offer-kind string;
- `retryable`: boolean;
- `details`: JSON object.

Diagnostics are explanatory. They MUST NOT authorize an offer or override the
accepted served domain set.

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
- `updated_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `expires_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `metadata`: JSON object.

The `offer_id` is scoped to the tuple `(producing peer id, domain_id)`.
Consumers that cache remote offers MUST identify an offer by
`(peer_id, domain_id, offer_id)`, not by `offer_id` alone.

For a given catalog response, the tuple `(domain_id, offer_id)` MUST be unique.

Producers SHOULD keep `offer_id` stable across catalog refreshes for the same
logical data source. Producers SHOULD issue a new `offer_id` when reusing the
old id would hide an incompatible payload, registry, or access-mode change.

The `kind` field is an open string. The v1 minimum known kinds are defined in
`RFC-0025`. Consumers MUST ignore unknown kinds unless local application code
explicitly supports them.

The v1 `status` values are:

- `available`: the producer currently believes the offer can be used;
- `temporarily_unavailable`: the offer is known but not currently usable.

An offer with `temporarily_unavailable` MAY remain in the catalog so consumers
can keep stable UI state or retry later. Get and Subscribe requests for such
an offer MAY fail with `offer.temporarily_unavailable`.

An offer that should no longer be discoverable SHOULD be removed from later
catalog snapshots rather than advertised with a permanent unavailable status.

The `updated_at` and `expires_at` fields are freshness hints. A consumer MAY
enforce `expires_at` by local policy. A consumer that enforces `expires_at`
MUST NOT start a new Get or Subscribe attempt for an expired offer and SHOULD
report `offer.stale`.

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
message envelope's payload object defined in `RFC-0022`.

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
knows them at catalog time.

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

A responder MUST NOT intentionally return an offer for a domain it did not
declare to the requester during handshake.

A receiver MUST ignore or reject any returned offer whose `domain_id` is not in
the receiver's accepted served domain set for that peer relationship, using
`offer.domain_not_served`.

A receiver SHOULD fail malformed catalog responses with
`offer.invalid_catalog_response`.

A receiver SHOULD ignore individual malformed offers with `offer.invalid_offer`
when the rest of the catalog is usable.

A responder SHOULD use `offer.catalog_unavailable` when it cannot produce a
catalog because of a local recoverable problem.

### RFC-0021: Offer Domain Scope And Authority

#### Requirement

Each v1 offer MUST include exactly one `domain_id`.

One v1 offer belongs to exactly one domain. Multi-domain offers are future
work.

The `domain_id` field in an offer is a producer-declared scope. It is not proof
that the producer is authorized to serve that domain.

A receiver MUST treat an offer as usable only when the offer's `domain_id` is
in the receiver's accepted served domain set for that peer relationship.

An offer whose `domain_id` is not in the accepted served domain set MUST be
ignored or rejected with `offer.domain_not_served`.

A v1 offer SHOULD NOT carry its own domain declaration or delegation unless a
later RFC defines embedded authority proofs. Offer authority is derived from the
peer relationship's accepted served domain set.

#### Domain State Changes

If a domain is rejected during handshake, offers scoped to that domain MUST NOT
be treated as usable.

If a domain later becomes invalid, expires under local policy, or is removed
from the accepted served domain set by a future dynamic update protocol, all
cached offers scoped to that domain MUST become unusable for new Get and
Subscribe attempts.

Existing subscriptions for that domain SHOULD be ended or treated as no longer
authorized once the implementation observes the domain is no longer accepted.
When an explicit Subscribe end message is sent for this case, it SHOULD use
the `not_authorized` reason.

#### Producer Claims And Receiver Authority

The producer controls offer metadata such as `offer_id`, `kind`, `payload`,
`registry_refs`, `status`, and `metadata`.

The receiver controls whether that offer is usable in the local peer
relationship by applying:

- peer authorization;
- domain authority validation;
- domain access policy;
- offer policy;
- payload and kind compatibility checks.

Accepting an offer's domain scope does not imply that the receiver trusts the
offer payload, registry contents, spatial correctness, data completeness, or
application semantics.

### RFC-0022: Spatial Message Envelope

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
- `generated_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `metadata`: JSON object.

The tuple `(domain_id, offer_id)` identifies the offer that produced the
message within the producing peer relationship. Consumers already know the
producing peer id from the transport-authenticated peer relationship. A message
envelope SHOULD NOT include a `peer_id` field unless a later RFC defines a
diagnostic or forwarding use case for it.

#### Domain And Offer Binding

A receiver MUST reject or ignore a message envelope whose `domain_id` is not in
the accepted served domain set for the producing peer relationship.

A receiver MUST reject or ignore a message envelope whose `(domain_id,
offer_id)` does not match the Get request or accepted Subscribe stream the
receiver opened.

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

If `bytes` is present, it MUST be base64url without padding over the raw
payload bytes.

Receivers MUST reject malformed payload objects with `message.invalid_payload`.

#### Registry References

The `clock` field, when present, is a registry-reference object whose
`registry` is `clock`.

The `registry_refs` field, when present, uses the same registry-reference shape
defined in `RFC-0020`.

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

The `timestamp_ns` field is measured in the clock identified by `clock` or by
the inherited clock registry reference for the stream or response.

If `timestamp_ns` is present and no clock can be resolved, the receiver SHOULD
treat the timestamp as uninterpretable rather than assuming local wall-clock
time.

The `generated_at` field is producer metadata and uses UTC wall-clock time. It
MAY be used for freshness diagnostics, but it MUST NOT replace `timestamp_ns`
for domain data timing.

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

The `message` field is for diagnostics only. Implementations MUST NOT branch
protocol behavior on `message`.

The `retryable` field is advisory. Receivers MAY apply local retry policy even
when it is absent.

#### Failure Mapping

A receiver SHOULD fail malformed envelopes with `message.invalid_envelope`.

A receiver SHOULD fail malformed payloads with `message.invalid_payload`.

A receiver SHOULD fail oversized payloads with `message.payload_too_large`.
Concrete size limits are defined by the Get and Subscribe RFCs.

A receiver MAY report observed sequence gaps with `message.sequence_gap`.

A receiver SHOULD use the existing offer failure-code family when the envelope
or request targets an unknown, unauthorized, unsupported, unavailable, or stale
offer.

### RFC-0023: Get

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
- `domain_id`: domain id string;
- `offer_id`: offer id string.

A v1 Get request MAY include:

- `params`: JSON object;
- `accepted_payload_types`: array of payload type strings;
- `max_payload_bytes`: positive integer.

The tuple `(domain_id, offer_id)` identifies the offer within the producing
peer relationship. The producing peer id is known from the
transport-authenticated peer relationship and SHOULD NOT be repeated in the
request.

The `params` object is offer-kind-specific. Receivers MUST ignore unknown
`params` fields unless the offer kind defines them as required.

The `accepted_payload_types` array lets the requester narrow the payload types
it is willing to receive. If omitted or empty, the requester accepts any
payload type advertised by the offer and supported by local policy.

The `max_payload_bytes` field lets the requester declare the largest payload it
is willing to accept. The responder MUST NOT send a payload whose raw payload
size is larger than `max_payload_bytes`.

#### Response

A v1 Get response is a JSON object.

A v1 Get response MUST include:

- `type`: string, exactly `auki.get_response.v1`.

A successful v1 Get response MUST include:

- `message`: spatial message envelope object.

A failed v1 Get response MUST include:

- `error`: error object as defined in `RFC-0022`.

A v1 Get response MUST include exactly one of `message` or `error`.

The `message` object MUST follow the spatial message envelope shape defined in
`RFC-0022`.

The `message.domain_id` and `message.offer_id` MUST match the request
`domain_id` and `offer_id`.

The response MUST be a complete response for the request. Get v1 has no
continuation token, chunk list, or streaming body.

#### Snapshot Semantics

A Get response represents the producer's best available snapshot at response
time.

The producer SHOULD set `message.generated_at` when it can report when the
snapshot was generated.

The producer SHOULD set `message.timestamp_ns` and an applicable clock
reference when the returned data represents domain data observed at a specific
producer clock time.

The producer SHOULD include or inherit registry references needed to interpret
the returned payload.

Get does not create a subscription and does not reserve future availability.
A later Get for the same offer MAY return different data or fail if the offer
status changes.

#### Size Limits

Get v1 is for small finite responses.

Implementations MUST define a maximum accepted Get response size.

If the requester supplies `max_payload_bytes`, the responder MUST honor the
lower of the requester's limit and the responder's local limit.

If the response would exceed the applicable limit, the responder SHOULD fail
with `message.payload_too_large`.

Get v1 MUST NOT split a response into multiple chunks.

#### First Use Cases

The first Get use cases are:

- `registry_entry`: return the exact registry entry identified by the offer's
  registry reference;
- `transform_edge`: return one direct transform edge;
- descriptor or small snapshot offers explicitly advertised with `get`.

A `registry_entry` Get response SHOULD return a payload object with a structured
`json` value containing the canonical registry JSON or an equivalent envelope
defined by the registry-entry protocol.

A `transform_edge` Get response SHOULD return a payload object with structured
`json` containing the transform and SHOULD include registry references with
roles `from_frame` and `to_frame`.

Log ranges, map fragments, and large binary artifacts are future work unless a
later RFC defines chunking or a separate transfer protocol.

#### Failure Mapping

A responder SHOULD fail malformed Get requests with `get.invalid_request`.

Get failure mapping SHOULD reuse the offer failure-code family:

- `offer.unknown_offer` when the requested `(domain_id, offer_id)` is not known;
- `offer.domain_not_served` when the domain is not in the accepted served domain
  set;
- `offer.unsupported_kind` when the offer kind is known but unsupported by the
  responder's Get implementation;
- `offer.unsupported_access_mode` when the offer does not support Get;
- `offer.unsupported_payload_type` when the responder cannot produce any payload
  type accepted by the requester;
- `offer.temporarily_unavailable` when the offer is known but unavailable;
- `offer.stale` when local freshness policy rejects the offer.

A responder SHOULD use `message.payload_too_large` when the response would
exceed the applicable size limit.

A requester SHOULD use the message failure-code family when a Get response
includes `message` but the envelope, payload, or payload size is invalid:

- `message.invalid_envelope`;
- `message.invalid_payload`;
- `message.payload_too_large`.

A transport or framing failure SHOULD be reported as `transport.failed` when a
structured Get response cannot be returned.

### RFC-0024: Subscribe

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
- `domain_id`: domain id string;
- `offer_id`: offer id string.

A v1 Subscribe request MAY include:

- `params`: JSON object;
- `accepted_payload_types`: array of payload type strings;
- `max_message_bytes`: positive integer.

The tuple `(domain_id, offer_id)` identifies the offer within the producing
peer relationship. The producing peer id is known from the
transport-authenticated peer relationship and SHOULD NOT be repeated in the
request.

The `params` object is offer-kind-specific. Receivers MUST ignore unknown
`params` fields unless the offer kind defines them as required.

The `accepted_payload_types` array lets the requester narrow the payload types
it is willing to receive. If omitted or empty, the requester accepts any
payload type advertised by the offer and supported by local policy.

The `max_message_bytes` field lets the requester declare the largest data
message it is willing to accept on the subscription. The producer MUST NOT send
a serialized spatial message envelope larger than `max_message_bytes`.

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
- `generated_at`: UTC RFC3339 timestamp string with a `Z` suffix;
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

A failed v1 start result is a JSON object.

A failed v1 start result MUST include:

- `type`: string, exactly `auki.subscribe_reject.v1`;
- `error`: error object as defined in `RFC-0022`.

If the request's `domain_id` and `offer_id` can be parsed, the `error` object
SHOULD include them.

The producer MUST NOT send data messages after a failed start result.

#### Data Messages

After a successful start result, each data message MUST follow the spatial
message envelope shape defined in `RFC-0022`.

Each data message MUST have `domain_id` and `offer_id` matching the accepted
subscription.

The data message payload fields SHOULD be compatible with the accepted
subscription payload descriptor.

When `sequence` is present, the producer SHOULD start at `initial_sequence` if
one was accepted, or at 0 or 1 otherwise. The producer SHOULD increase
`sequence` by 1 for each data message on the subscription.

Receivers MAY use sequence gaps as diagnostics. A sequence gap does not by
itself require closing the subscription.

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

- `error`: error object as defined in `RFC-0022`;
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

Implementations MUST define a maximum accepted Subscribe message size.

If the requester supplies `max_message_bytes`, the producer MUST honor the
lower of the requester's limit and the producer's local serialized-message
limit.

If a data message would exceed the applicable limit, the producer SHOULD either
skip the message and record local diagnostics or end the subscription with an
error using `message.payload_too_large`.

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

Before reconnecting, the requester SHOULD re-check that the peer relationship,
served domain set, offer status, offer access modes, and payload compatibility
still permit Subscribe.

#### Failure Mapping

A responder SHOULD fail malformed Subscribe requests with
`subscribe.invalid_request`.

Subscribe failure mapping SHOULD reuse the offer failure-code family:

- `offer.unknown_offer` when the requested `(domain_id, offer_id)` is not known;
- `offer.domain_not_served` when the domain is not in the accepted served domain
  set;
- `offer.unsupported_kind` when the offer kind is known but unsupported by the
  responder's Subscribe implementation;
- `offer.unsupported_access_mode` when the offer does not support Subscribe;
- `offer.unsupported_payload_type` when the responder cannot produce any
  payload type accepted by the requester;
- `offer.temporarily_unavailable` when the offer is known but unavailable;
- `offer.stale` when local freshness policy rejects the offer.

A producer SHOULD use `message.payload_too_large` when a data message would
exceed the applicable size limit.

A receiver SHOULD use the message failure-code family when a Subscribe data
message includes a spatial message envelope but the envelope, payload, message
size, or observed sequence behavior is invalid:

- `message.invalid_envelope`;
- `message.invalid_payload`;
- `message.payload_too_large`;
- `message.sequence_gap`.

A transport or framing failure SHOULD be reported as `transport.failed` when a
structured Subscribe reject or end message cannot be returned.

Subscribe is the likely replacement or evolution path for `/auki/stream/0.1.0`.

### RFC-0025: Minimum Offer Kinds

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

Future kinds MUST preserve the same baseline rules: one v1 offer belongs to
exactly one domain, unknown kinds are ignored by default, and authority is
derived from the peer relationship's accepted served domain set rather than
from the offer object itself.

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
- why any peer binding, domain declaration, or delegation was rejected;
- why any offer catalog, Get, Subscribe, or spatial message envelope failed;
- whether a Subscribe stream has observed sequence gaps;
- why a peer became degraded or lost;
- whether Discovery is degraded independently from peer connectivity.

#### Consequences

Heartbeat-frame logs, stream-frame logs, and repeated dial retry logs SHOULD be
rate-limited or omitted by default.

State transitions and failures SHOULD be logged once with enough context to
debug the lifecycle.

`RFC-0028` defines the concrete diagnostic status snapshot shape for
implementations that expose status as structured data.

### RFC-0028: Status And Observability API

#### Requirement

Implementations MUST expose a status surface that explains the local peer's
current lifecycle state, peer relationships, served domains, offer loading, and
active or recently completed spatial-data paths.

The status surface is diagnostic only. It MUST NOT be used as proof of peer
identity, domain authority, offer authority, payload correctness, or data
trustworthiness. Protocol validation remains the source of authority.

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
- `generated_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `local_peer`: local peer status object;
- `local_domains`: array of local domain status objects;
- `remote_peers`: array of remote peer status objects;
- `active_paths`: array of active or recently completed path status objects;
- `last_failures`: array of failure record objects.

A v1 status snapshot MAY include:

- `discovery`: Discovery status object;
- `metadata`: JSON object.

The snapshot is best-effort diagnostic state at `generated_at`. It is not a
transactional view of the network.

Arrays in the status snapshot MAY be empty.

#### Local Peer Status

A local peer status object SHOULD include:

- `peer_id`: local libp2p peer id string;
- `wallet_public_key`: base64url wallet public key string when available;
- `peer_binding_issued_at`: UTC RFC3339 timestamp string with a `Z` suffix;
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
- `delegation_expires_at`: UTC RFC3339 timestamp string with a `Z` suffix,
  or null;
- `advertised`: boolean;
- `serving_offers`: boolean;
- `last_failure`: failure record object or null.

This object reports the local peer's view of domains it owns, manages, serves,
or intends to advertise. It does not prove that a remote peer will accept those
domains.

The `role` field is diagnostic. The `managed` role means the local peer tracks
the domain locally without claiming owner or delegate authority in that status
entry.

#### Discovery Status

A Discovery status object SHOULD include:

- `enabled`: boolean;
- `discoverable`: boolean;
- `advertised_domains`: array of domain id strings;
- `advertised_addresses`: array of multiaddr strings;
- `last_refresh_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
- `expires_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
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
- `lifecycle_state`: string compatible with the state model in `RFC-0017`;
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
- `last_fetch_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
- `last_success_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
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
- `updated_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
- `expires_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
- `last_failure`: failure record object or null.

The `usable` field is local policy and compatibility state. It MUST NOT be
treated as proof that the offer is correct, complete, or trustworthy.

Registry-reference summary objects SHOULD include the `registry`, `role`, `id`,
and `hash` fields from the registry-reference shape defined in `RFC-0020`.

#### Path Status

A path status object SHOULD include:

- `path_id`: implementation-defined local id string;
- `path_type`: string, one of `get` or `subscribe`;
- `peer_id`: producing peer id string;
- `domain_id`: domain id string;
- `offer_id`: offer id string;
- `state`: string;
- `started_at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `last_message_at`: UTC RFC3339 timestamp string with a `Z` suffix, or null;
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
- `at`: UTC RFC3339 timestamp string with a `Z` suffix;
- `scope`: string, such as `peer`, `domain`, `offer_catalog`, `offer`, `get`,
  `subscribe`, `message`, or `discovery`;
- `peer_id`: peer id string, optional;
- `domain_id`: domain id string, optional;
- `offer_id`: offer id string, optional;
- `path_id`: path id string, optional;
- `retryable`: boolean, optional;
- `message`: diagnostic string, optional;
- `details`: JSON object, optional.

The `code` field SHOULD use the stable failure codes defined in `RFC-0006` or
by the RFC that owns the failing path.

The `message` and `details` fields are diagnostic only. Implementations MUST
NOT require another implementation to parse them for protocol behavior.

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
