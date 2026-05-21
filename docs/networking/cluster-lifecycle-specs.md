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
  scoped to that domain;
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
timestamps, scope order, drop unknown fields, or change base64url spelling
before canonicalizing the signed bytes. The signature verifies the JSON values
as presented, minus only the `signature` field.

#### Verification

To verify a v1 domain delegation for a claimed action, a receiver MUST:

1. Decode the JSON object and verify all required fields are present and
   well-formed.
2. Verify that `type` and `wallet_signature_scheme` are supported.
3. Decode `domain_id`, `domain_owner_public_key`,
   `delegate_wallet_public_key`, and `signature`.
4. Parse `delegate_peer_id` as a libp2p PeerId.
5. Verify that `scopes` contains only v1 scopes, is non-empty, has no
   duplicates, and is sorted lexicographically.
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
8. Accept only offer catalog entries whose domain is in the accepted served
   domain set.

Peer authorization is defined in `RFC-0016`. In the authority-chain validation
path, peer authorization runs after peer binding verification and before served
domains are accepted.

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
- `offer.domain_not_served`
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
- liveness or status initialization data, when supported.

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

A peer that exposes a domain-scoped offer catalog MUST declare the domains whose
offers may appear in that catalog.

The receiver MUST NOT treat offers from that catalog as usable unless each
offer's domain is in the accepted served domain set.

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

Non-experimental deployments SHOULD use `whitelisted-only` or `app-policy`.

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
- one offer belongs to exactly one domain in v1;
- multi-domain offers are future work;
- how an offer references delegation or served-domain validation;
- behavior when the served domain becomes rejected, expired, or removed;
- how consumers distinguish producer-declared metadata from verified authority.

An offer is usable only when its single domain id is in the accepted served
domain set for that peer relationship.

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
