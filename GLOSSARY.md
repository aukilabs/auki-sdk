# Glossary

Terms used by the current Auki SDK and its real-world-web protocol.

## Domain

A DDS-owned identifier and authority boundary for one physical space. A Domain
groups data that is asserted to describe the same place. It is not a Rust
runtime, a leader, a peer roster, a scenegraph, or a coordinate frame.

## Domain ID

The canonical DDS UUID carried by credentials and checked during mutual peer
authentication. Domain ID, Scenegraph ID, and Session ID answer different
questions:

| Identifier | Question |
| --- | --- |
| Domain ID | Which physical space and authority boundary? |
| Scenegraph ID | Which structured spatial representation of that space? |
| Session ID | Which recording timeline? |

## Domain owner

The entity that controls Domain policy in DDS and may designate a canonical
Map. Runtime access is represented by signed authority, not an SDK-local
Manager role.

## AukiPeer

The high-level authenticated networking runtime in `auki-sdk`. One `AukiPeer`
owns one Peer ID, authority for one Domain, reachability, protocol
registrations, fencing, and ordered shutdown. Native Rust additionally exposes
authenticated peer observations.

## PreparedPeer

Validated input for starting `AukiPeer`: selected Domain, authorized Peer ID,
verification keys, signed credential, expiry information, and renewal
mechanism. `auki-auth` prepares this for User or trusted native App
credentials.

## Peer ID

The libp2p identity bound by Noise and carried in `/p2p/<peer-id>` multiaddrs.
Native hosts normally persist the Ed25519 identity with
`auki_p2p::Identity::load_or_create`. One live process or pod should own a
given Peer ID.

## Authenticated peer

The identity delivered to a protocol handler after mutual authentication. It
contains the verified Peer ID, Domain scope, subject, peer type, application
metadata, scopes, and credential lifetime. Authentication does not grant every
application capability.

## Route

A multiaddr describing where to dial a peer. Direct TCP and relay-circuit
routes are supported natively; browsers use WSS relay routes. A route is an
untrusted location hint and never authority.

## Relay

A libp2p Circuit Relay v2 provider used to make a peer reachable. DMS allocates
and renews relay reservations for the high-level runtime. A relay does not
discover peers or decide who may connect.

## Discovery

Any application or future SDK mechanism that supplies a remote Peer ID,
supported protocols, and complete routes. Static configuration, manual peer
cards, and product control planes are valid current sources. Discovery remains
separate from authentication.

## Known peer

A post-authentication observation of a peer with which the runtime has a live
authenticated relationship. It is not a membership roster, discovery result,
route catalog, or authorization source.

## Protocol ID

The immutable name of one application wire contract, such as
`/example/echo/1.0.0` or `/auki/auth/1/stream/0.2.0`. An incompatible framing,
schema, ordering, bound, or semantic change requires a new ID.

## Client

The outbound half of a portable protocol API. A Client opens an authenticated
stream through a running `AukiPeer`, performs the bounded conversation, and
returns validated application data.

## Endpoint

The inbound and lifecycle half of a portable protocol API. Mounting an Endpoint
explicitly registers one or more exact protocol IDs. Dropping or closing it
stops serving those protocols. A peer serves no application protocol by
default.

## Provider

Application-owned data or admission logic used by an Endpoint after the remote
peer has authenticated. Providers receive `AuthenticatedPeer` when policy may
depend on the caller.

## SessionProtocolProvider

The native adapter in `auki-protocols` that projects one exact
`auki_session::Peer` + `Session` into Catalog v3/v4 snapshots and Stream v2
sources. It is mechanical; the application remains responsible for access
policy.

## FsRegistryProvider

A native read-only Registry v3 provider rooted at one application directory
and one local Peer ID. It validates keys, ownership, IDs, and content hashes
before serving entries. Device Model listing returns current tips.

## FsBlobProvider

A native read-only Blob v1 provider rooted at one application directory. It
serves bounded ranges through content-addressed SHA-256 blob paths.

## Peer

The network-free long-lived identity and registry owner in `auki-session`.
This data-model type is distinct from the networking runtime `AukiPeer`.

## Session

One locally recorded timeline created by `Peer::start_session`. A Session owns
a fresh Session ID, its clocks, and registered Sensor, Pose, TimeTransform,
Detection, and Map Logs. It has no credentials or network lifecycle.

## Session ID

The ULID minted for one recording Session. It identifies a run, not a Domain or
scenegraph.

## Registry

A content-addressed collection of immutable metadata entries. The SDK has
Sensor, Clock, Frame, Detector, Map, and Device Model registries. A reference
contains the owner Peer ID, logical ID, and exact content hash.

## RegistryRef

An exact `(peer_id, id, hash)` pointer to a Registry entry. Consumers verify all
three fields and the entry content before using it.

## Blob

Immutable binary content addressed by lowercase SHA-256. Device Models and
other registry entries may reference blobs.

## Resource

Something another peer can consume, such as a Sensor Log, Pose Log,
TimeTransform Log, Detection Log, Map Log, or live message channel. A Detector
or Mapper implementation is a component, not itself a Resource.

## Resource catalog

A current, pollable snapshot of Resources a peer can serve. Catalog v3 carries
log rows and message channels; Catalog v4 carries Map Logs. Catalog v2 remains
a wire-only codec.

## Source peer / writer peer

`source_peer_id` identifies the physical origin of data.
`writer_peer_id` identifies the peer holding the manifest or bytes. They are
equal for origin data and may differ after materialization.

## Manifest

Canonical JSON metadata attached to a log. It pins the source/writer identity,
Session, clock, Registry references, and retention or extent needed to
interpret the stored payloads.

## Sensor Log

Timestamped sensor payloads whose manifest pins an exact Sensor Registry entry,
clock, and optional spatial frame.

## Pose Log

Timestamped `from_frame -> to_frame` spatial transforms. Each manifest pins the
two Frame Registry entries and the clock used for samples.

## TimeTransform Log

Timestamped relations between two exact clocks. These records preserve clock
lineage for future `convert_time` composition without electing a global clock.

## Detection Log

Timestamped detector outputs bound to the exact detector, input log, input
sensor contract, clock, and cadence that produced them.

## Map Log

Timestamped `MapUpdate` values bound to an immutable Map Registry contract and
clock. Map Logs support durable replay plus live subscription.

## Map

A spatial representation described by a content-addressed Map Registry entry
and updated through a Map Log. At the broader protocol level, a Domain owner
may designate a canonical Map for a physical space.

## Scenegraph

A structured spatial representation whose nodes are frames, sensors, clocks,
and other typed objects connected by transforms. Multiple scenegraphs may
describe one Domain.

## Frame

A named coordinate system. Its handedness, axis directions, and units are
declared by an exact Frame Registry entry; relations between frames live in
Pose Logs.

## Clock

A named time basis described by a Clock Registry entry. Every timestamp should
be interpreted through its pinned clock rather than an assumed global Domain
time.

## convert_pose / convert_time

Planned consumer operations that compose recorded frame or clock relations.
Convention conversion and affine time math exist, while general graph
selection and composition remain incomplete.
