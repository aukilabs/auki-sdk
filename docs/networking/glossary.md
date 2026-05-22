# Networking Glossary

Status: draft terminology baseline.

Last updated: 2026-05-22.

This glossary defines terms used by the networking RFCs. The goal is stable
protocol wording: each term should name one concept, and each concept should
use the same term throughout the specs.

This glossary is descriptive. Normative requirements live in the RFC documents.

## Protocol Foundation Terms

### Authority Boundaries

The separation between protocol authority proofs and operational metadata.

Authority boundaries describe which inputs can affect identity, domain
authority, delegation, reachability, policy, payload interpretation, and
diagnostics.

### JSON Wire Conventions

The shared JSON encoding conventions used by v1 protocol messages and signed
authority objects.

### Signed JSON Object

A JSON object whose signature is verified over a defined canonical byte form.

Signed JSON objects are used for peer bindings, domain declarations, and domain
delegations.

### Failure Code

A stable string such as `category.reason` that identifies a protocol or
diagnostic failure.

### Failure Record

A diagnostic object that records a failure code, failure time, scope, optional
peer/domain/offer/path identifiers, and optional diagnostic details.

### Handshake

The symmetric exchange peers perform after transport connection and before
offer loading or domain-scoped data exchange.

### Offer Catalog Fetch Path

The handshake-advertised path a connected peer can use to request the remote
peer's v1 offer catalog.

## Actor And Identity Terms

### Wallet

A cryptographic identity used for ownership, signing, delegation, and policy.

### Peer

A runtime network endpoint participating in peer-to-peer communication.

A peer is identified by a peer id and may be reachable through one or more
advertised addresses.

### Peer Id

The runtime network identity of a peer.

In this RFC set, a peer id is a libp2p peer id.

### Peer Binding

A wallet-signed statement that binds a wallet public key to a libp2p peer id.

### Peer Binding Freshness

The receiver's local decision about whether a peer binding is recent enough for
local policy.

### Peer Authorization

The peer-level allow or deny decision for one peer relationship after peer
binding verification.

### Peer Relationship

The per-remote-peer relationship tracked after discovery, configuration,
dialing, connection, authorization, offer loading, or data exchange.

### Peer Graph

The set of peers known locally, connected locally, or usable as candidate dial
targets.

A peer graph is operational state. It is not authoritative membership.

## Domain Terms

### Domain

A namespace and authority boundary for domain-scoped data.

### Domain Id

The stable identifier of a domain.

### Domain Owner Wallet

The wallet that controls a domain namespace.

### Domain Declaration

A signed statement by the domain owner wallet declaring a domain id.

### Declared Domain

A domain a peer claims to serve during handshake.

A declared domain is backed by a domain declaration and, when required, a
delegation.

### Authority Chain Validation

The validation path that checks peer identity, peer authorization, domain
declarations, delegations when needed, and local domain policy before accepting
served domains.

### Accepted Served Domain Set

The domains a remote peer is accepted to serve within one peer relationship.

The accepted served domain set is produced by validating declared domains. It
is also called the served domain set in spec prose. It is not every domain the
remote peer knows about.

### Display Label

A human-readable label for a domain, peer, or offer.

### Domain Ownership

Authority over a domain namespace.

### Nonce

A value chosen when creating a domain id.

The nonce is combined with the domain owner wallet public key when deriving a
domain id.

### Runtime Authority

Authority for a runtime peer to advertise, serve, or perform a defined
domain-scoped action under a domain.

### Delegation

An authorization, signed by the domain owner wallet, that allows a peer to act
for a domain within a specific scope.

### Domain Access Policy

Local or application policy deciding whether to consume or use an otherwise
valid remote domain.

Domain access policy is not proof of domain ownership or runtime authority.

### External Binding

A reference from an external registry, blockchain record, NFT, or tokenomics
system to a domain id.

## Discovery And Connectivity Terms

### Discovery

Optional rendezvous and presence infrastructure.

Discovery helps peers find entrypoints.

### Discovery Record

A non-authoritative advertisement published through Discovery or an equivalent
index.

A Discovery record may include a domain id, peer id, advertised addresses,
freshness metadata, entrypoint hints, and coarse data-type hints.

### Data-Type Hint

Coarse, non-authoritative Discovery metadata about data types that may be
available behind an entrypoint.

### Entrypoint

A peer or address used to start dialing into a peer graph.

### Listen Address

The local address where a network runtime binds.

### Advertised Address

An address another peer should use to dial a peer.

Advertised addresses are operational reachability hints. They are not identity
or authority.

### Relay

A transport path used when peers cannot connect directly.

### Cluster

A peer connectivity or session graph used for operational coordination.

## Time Terms

### Timestamp

A time value carried in protocol metadata or diagnostics.

### Generated At

Producer wall-clock metadata reporting when an object, snapshot, message, or
status record was generated.

### Timestamp Ns

Producer or domain event time expressed in nanoseconds in a referenced clock.

### Clock Reference

A reference that identifies the clock used to interpret a timestamp.

### Clock Registry Reference

A registry reference whose `registry` value is `clock`.

### Freshness Hint

Metadata that helps a receiver decide whether something is recent enough for
local policy.

### Local Receive Time

Receiver-local diagnostic time for when data or protocol state was observed.

### Clock-Sync Result

Diagnostic state estimating timing relationship between peers or clocks.

### Clock Sync

Diagnostic clock-comparison behavior used to estimate timing relationships.

## Exchange Terms

### Domain-Scoped Data

Domain-scoped data advertised and exchanged through Offer, Get, and Subscribe.
Its concrete meaning is defined by an offer kind, application implementation,
deployment profile, or later RFC.

### Offer

A connected peer's declaration of one named and typed data item it is willing
to serve.

### Offer Id

The identifier used to request a specific offer from the producing peer.

### Offer Catalog

The protocol surface a peer uses to list current offers.

### Offer Kind

An open string identifying which application-defined, profile-defined, or
later-RFC-defined semantics apply to an offer.

### Offer Status

The producer-reported availability state of an offer in an offer catalog.

Status API offer status objects are diagnostics about loaded offers and include
this availability state.

### Access Mode

An offer-advertised path by which the offer can be used, such as Get or
Subscribe.

### Offer Usability

The receiver's local decision that an offer can be used on a requested path
after authority, policy, kind, access-mode, payload, status, and freshness
checks.

### Payload Descriptor

Offer metadata describing the expected payload family, encoding, schema, or
media type.

### Registry Reference

A content-addressed reference to a registry entry used by an offer or message.
The required registry references depend on the offer kind or application
profile.

### Offer Policy

Local or application policy deciding whether to load, display, Get, or
Subscribe to a specific offer.

### Spatial Message Envelope

Common metadata carried with data messages returned by Get or Subscribe.

### Spatial Message Payload

The payload object inside a spatial message envelope.

### Sequence Gap

A diagnostic observation that a Subscribe stream's sequence numbers skipped one
or more expected values.

### Get

A one-shot fetch of an offered data item.

### Subscribe

A request to receive ongoing updates from an offered data item.

## Status And Observability Terms

### Status Snapshot

A best-effort diagnostic view of local peer state, local domains, remote peer
relationships, active or recent paths, and recent failures.

### Local Peer Status

Diagnostic state about the local peer's identity, peer binding, authorization
mode, listen addresses, and advertised addresses.

### Remote Peer Status

Diagnostic state about one remote peer relationship.

### Path Status

Diagnostic state about one active or recently completed Get or Subscribe path.
