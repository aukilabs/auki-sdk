# Glossary

This page explains the terms most likely to be confused when composing local
recording with authenticated networking. The repository's
[`GLOSSARY.md`](https://github.com/aukilabs/auki-sdk/blob/develop/GLOSSARY.md)
remains the compact reference.

## Peer ID

The libp2p identity of a network participant. Native applications normally
persist the underlying `auki_sdk::Identity` and reuse the same Peer ID after a
restart. The Web `0.1` facade intentionally creates a fresh in-memory identity
for each peer start.

A Peer ID identifies a cryptographic peer. It does not grant permissions and
does not tell another peer where to dial.

## `auki_session::Peer`

The long-lived, network-free owner of local registries and recording metadata.
It creates `Session` timelines. It does not authenticate, book a relay, listen
for network traffic, or discover peers.

This type and `auki_sdk::AukiPeer` have different ownership roles even when an
application associates them with the same robot.

## `auki_sdk::AukiPeer`

The authenticated networking runtime for one Peer ID in one DDS Domain. It
owns renewable authority, transport, relay booking, routes, protocol
registration, lifecycle fencing, and shutdown.

An `AukiPeer` serves no product protocol by default. Applications explicitly
mount the endpoints they support.

## DDS Domain

The authority boundary selected during authentication. A signed credential
proves that a Peer ID may participate in that Domain.

A Domain is not a process owner, peer roster, discovery mechanism, or route.
The removed Rust `Domain` runtime is historical; current networking uses
`AukiPeer`.

## `PreparedPeer`

Validated authority material for starting an `AukiPeer`: selected Domain,
authorized Peer ID, credential, verification keys, expiration, and renewal
provider. `auki-auth` creates it for trusted User and App hosts. Robot and
Compute integrations may supply equivalent authority through
`AukiPeer::start_external`.

## `AuthenticatedPeer`

The remote identity and verified claims attached to an admitted protocol
stream. Protocol providers receive it so they can enforce product policy.

Authentication answers “who is this peer and which Domain admitted it?” It
does not answer “may it drive this robot?”

## Route

An untrusted location hint used to dial an expected Peer ID. Native routes are
normally TCP; browser routes are WSS. The authenticated transport still proves
the remote Peer ID and Domain before application data flows.

## Relay

A reachable intermediary that accepts circuit reservations. Relay allocation
makes a peer dialable from more network environments. It does not discover
remote peers or authorize application operations.

Native `AukiPeer` uses relay-backed reachability by default and may explicitly
choose direct-only operation. Each relay-provider slot yields one required
TCP/WSS route pair under one reservation; native/Python reserve over TCP and
browsers reserve over WSS. Additional slots are provider redundancy.

## Discovery

How an application learns another peer's Peer ID, supported protocols, and
routes. Automatic discovery and route publication are not part of `0.1`.
Applications currently use configuration, a product control plane, or an
explicit peer-information exchange.

Discovery data is never authority.

## Protocol ID

The immutable name of one wire contract and conversation, for example
`/example/echo/1.0.0`. Changing incompatible framing, schemas, bounds, ordering,
or observable semantics requires a new ID. `/auki/...` is reserved for
SDK-owned protocols.

## Endpoint and Client

An `auki-protocols` `Endpoint` owns an explicitly mounted inbound protocol and
its cleanup barrier. Its cloneable `Client` performs outbound operations. A
client-only peer need not mount the corresponding endpoint.

Keep endpoints alive while serving, close them before `peer.shutdown()`, and
put product-specific protocols in one product-owned Rust crate so native and
Web use the same implementation.

## Catalog

The live snapshot of resources a peer is currently willing to serve. Catalog
v3 is the active general endpoint. Sensor, pose, time-transform, and detection
entries retain their Catalog v2 JSON shape inside the v3 response; v3 also adds
message-channel rows. Catalog v2 remains a compatibility wire schema. Map Logs
use Catalog v4.

A catalog row is metadata, not a guarantee that a resource remains available
forever and not permission to access it.

## Session

One local recording timeline created by `auki_session::Peer::start_session()`.
It has a fresh session ID, clocks, and log handles. A session can exist without
any `AukiPeer`.

## Registry reference

An immutable `(peer_id, id, hash)` reference to exact registry content. The hash
binds consumers to one canonical version rather than whichever value happens
to share a label later.

## Source peer and writer peer

Every log distinguishes canonical origin from the peer storing this copy:

- `source_peer_id` is the enduring owner of the data product.
- `writer_peer_id` is the peer that wrote these manifest and segment bytes.

They are equal for an origin log and differ for a materialized copy. See
[Concept: peer-owned logs](Concept-Peer-Owned-Logs).

## See also

- [The Five Questions](The-Five-Questions)
- [Crate map](Crate-Map)
- [Auki P2P](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/README.md)
- [Protocol authoring](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/authoring-protocols.md)

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Crate map →](Crate-Map)
