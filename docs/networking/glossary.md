# Networking Glossary

Status: draft terminology baseline.

Last updated: 2026-05-20.

This glossary defines terms used by the networking RFCs. The goal is stable
protocol wording: each term should name one concept, and each concept should
use the same term throughout the specs.

## Actor And Identity Terms

### Participant

An actor using the SDK networking protocol.

A participant has a wallet identity, maintains a domain, may operate one or
more peers, and may publish, consume, relay, compute, observe, or diagnose
according to local policy.

### Wallet

A cryptographic identity used for ownership, signing, delegation, and policy.

### Peer

A runtime network endpoint participating in peer-to-peer communication.

A peer is identified by a peer id and may be reachable through one or more
advertised addresses.

### Peer Id

The runtime network identity of a peer.

In this RFC set, a peer id is a libp2p peer id and is bound to a wallet
identity by a peer binding.

### Peer Binding

A wallet-signed statement that binds a wallet public key to a libp2p peer id.

A peer binding proves only that the wallet recognizes the peer id as one of
its runtime peers.

### Peer Relationship

The per-remote-peer relationship tracked by a participant after discovery,
configuration, dialing, connection, authorization, offer loading, or data
exchange.

### Peer Graph

The set of peers a participant knows about, is connected to, or can use as
candidate dial targets.

A peer graph is operational state. It is not authoritative membership.

## Domain Terms

### Domain

A participant-maintained namespace and authority boundary for spatial state.

A domain groups local frames, clocks, sensors, streams, logs, maps, transforms,
offers, and resources under one domain id.

### Domain Id

The stable identifier of a domain.

In this RFC set:

```text
domain_id = hash(domain_owner_wallet_public_key, nonce)
```

### Domain Owner Wallet

The wallet that controls a domain namespace.

### Domain Declaration

A signed statement by the domain owner wallet that binds:

- domain id;
- domain owner wallet public key;
- nonce.

The domain declaration proves that the domain owner wallet declared the domain
id and controls the namespace. It does not prove data correctness.

### Display Label

A human-readable label for a domain, peer, offer, or participant.

A display label is convenience metadata. It is not identity and is not proof of
ownership or authority.

### Domain Ownership

Authority over a domain namespace.

Domain ownership includes authority to authorize runtime peers to advertise,
serve, or update data under that domain.

Domain ownership does not prove that associated spatial data is correct,
canonical, complete, or trusted.

### Nonce

A value chosen when creating a domain id.

The nonce is combined with the domain owner wallet public key when deriving the
domain id. It must be unique for domains created by the same domain owner
wallet.

### Runtime Authority

Authority for a runtime peer to advertise, serve, or update data under a
domain.

Runtime authority comes from controlling the domain owner wallet or presenting
a valid delegation signed by the domain owner wallet.

### Delegation

An authorization, signed by the domain owner wallet, that allows a peer to act
for a domain within a specific scope.

The concrete delegation schema is future RFC work.

### External Binding

A reference from an external registry, blockchain record, NFT, or tokenomics
system to a domain id.

External bindings may support persistence or economics, but they are not
required for peer-to-peer domain id verification.

## Discovery And Connectivity Terms

### Discovery

Optional rendezvous and presence infrastructure.

Discovery helps participants find entrypoints. It is not authoritative for
domain ownership, runtime authority, membership, data ownership, or data
correctness.

### Discovery Record

A non-authoritative advertisement published through Discovery or an equivalent
index.

A Discovery record may include a domain id, peer id, advertised addresses,
freshness metadata, entrypoint hints, and coarse data-kind hints.

### Entrypoint

A peer or address a participant can use to start dialing into a peer graph.

### Listen Address

The local address where a network runtime binds.

### Advertised Address

An address another participant should use to dial a peer.

Advertised addresses are operational reachability hints. They are not identity
or authority.

### Relay

A transport path used when peers cannot connect directly.

A relay changes connectivity only. It does not change peer id, domain
ownership, runtime authority, or data semantics.

### Cluster

A peer connectivity or session graph used for operational coordination.

A cluster is not authoritative for domain ownership, data ownership, publish
authorization, or consume authorization unless a later RFC explicitly gives it
that authority.

### Manager

A legacy or future coordination role.

Manager is not a baseline authority role in this RFC set. If current
implementation code still uses the term, it should be treated as compatibility
vocabulary until a later RFC defines a shared-domain overlay.

## Exchange Terms

### Spatial Data

Data describing spatial or temporal state, including frames, clocks, sensors,
streams, logs, maps, transforms, and resources.

### Offer

A connected peer's declaration of one named and typed data item it is willing
to serve.

An offer is not proof of authority, correctness, or trustworthiness.

### Offer Id

The identifier used to request a specific offer from the producing peer.

Offer ids are scoped to the producing participant's domain unless a later
offer-catalog RFC explicitly defines a narrower scope.

### Offer Catalog

The protocol surface a peer uses to list current offers.

The concrete request and response shape is future RFC work.

### Get

A one-shot fetch of an offered data item.

### Subscribe

A request to receive ongoing updates from an offered data item.
