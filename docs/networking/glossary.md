# Networking Glossary

Status: draft terminology baseline.

Last updated: 2026-05-21.

This glossary defines terms used by the networking RFCs. The goal is stable
protocol wording: each term should name one concept, and each concept should
use the same term throughout the specs.

This glossary is descriptive. Normative requirements live in the RFC documents.

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

A namespace and authority boundary for spatial state.

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

### Served Domain Set

The domains a remote peer is accepted to serve within one peer relationship.

The served domain set is produced by validating declared domains. It is not
every domain the remote peer knows about.

### Display Label

A human-readable label for a domain, peer, or offer.

### Domain Ownership

Authority over a domain namespace.

### Nonce

A value chosen when creating a domain id.

The nonce is combined with the domain owner wallet public key when deriving a
domain id.

### Runtime Authority

Authority for a runtime peer to advertise, serve, or update data under a
domain.

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

## Exchange Terms

### Spatial Data

Data describing spatial or temporal state, including frames, clocks, sensors,
streams, logs, maps, transforms, and resources.

### Offer

A connected peer's declaration of one named and typed data item it is willing
to serve.

### Offer Id

The identifier used to request a specific offer from the producing peer.

### Offer Catalog

The protocol surface a peer uses to list current offers.

### Offer Policy

Local or application policy deciding whether to load, display, Get, or
Subscribe to a specific offer.

### Spatial Message Envelope

Common metadata carried with spatial data messages so a consumer can interpret
the payload.

### Get

A one-shot fetch of an offered data item.

### Subscribe

A request to receive ongoing updates from an offered data item.
