# Discover peers

Discovery answers: “Which peers might I be able to reach?” It returns
short-lived candidates. It does not authenticate a peer, grant product
permission, or establish a connection.

The SDK currently provides one discovery mechanism: the DDS tracker.

## Enable DDS discovery

Choose the policy when building the peer:

~~~rust,ignore
use auki_sdk::{
    AukiPeerBootstrap, Credentials, DdsTrackerMode, DomainSelection,
};

let bootstrap = AukiPeerBootstrap::dev(
    Credentials::user_password(email, password),
)
.await?
.with_dds_tracker(DdsTrackerMode::DiscoverAndAdvertise);

let peer = bootstrap
    .start_persistent_peer(DomainSelection::new(domain_id), identity_file)
    .await?;
~~~

The two modes are explicit:

| Mode | Local peer is listed | Can query DDS |
| --- | --- | --- |
| `DiscoverOnly` | No | Yes |
| `DiscoverAndAdvertise` | Yes | Yes |

Use `DiscoverOnly` for controllers and monitoring interfaces. Use
`DiscoverAndAdvertise` for robots, cameras, compute nodes, and services that
accept inbound connections.

Discovery is disabled when `with_dds_tracker` is omitted. Manual routes and
product control planes continue to work.

## Find a protocol

Filter by the complete, versioned protocol ID:

~~~rust,ignore
use auki_protocols::info::{InfoClient, v1::ID as INFO_PROTOCOL_ID};

let info = InfoClient::new(peer.protocols());

for candidate in peer.discover_protocol(INFO_PROTOCOL_ID).await? {
    for route in candidate.routes() {
        if let Ok(remote) = info
            .fetch_exact(candidate.peer_id(), route.clone())
            .await
        {
            println!("found {}", remote.name);
            break;
        }
    }
}
~~~

Use `peer.discover()` when the application wants every current candidate.
Protocol matching is exact: the tracker does not infer prefixes, version
ranges, or compatibility.

## What a candidate contains

Each unexpired DDS entry contains:

- the advertised Peer ID;
- current direct or relay routes;
- exact mounted inbound protocol IDs; and
- the advertisement expiry.

The SDK derives protocol IDs from endpoints that are actually mounted.
Applications do not maintain a second list. Mounting or closing an endpoint
updates the advertisement.

The tracker contract is forward-compatible: required fields are validated,
while unknown JSON fields are ignored. A service may therefore add metadata
without breaking older SDK clients.

An advertised protocol is only a filtering hint. It does not prove that the
peer is still online, the route works, the handler will accept this caller, or
the peer owns a particular camera or robot capability. Query Info, Catalog,
Registry, or a product protocol after connecting.

## Reachability is separate

Advertising requires at least one route another peer can dial:

- a relay-backed peer advertises its confirmed relay routes;
- a native direct peer may advertise an application-supplied direct route; and
- an outbound-only Web peer has no inbound route, so it can discover but cannot
  advertise.

A discover-only peer may still own a relay; DDS simply does not publish it.

## Trust boundary

~~~text
DDS candidate
    -> exact Peer ID + route dial
    -> Noise verifies the Peer ID
    -> Auki authentication verifies the Domain
    -> product policy decides what the peer may do
~~~

Discovery never inserts a candidate into `known_peers()`. That collection
contains observations from authenticated connections, not a Domain roster and
not an authorization list.

All peers with a valid DDS P2P credential for a Domain can currently list every
unexpired advertisement in that Domain. The first version has no private
advertisements or per-peer visibility rules.

## Publication lifecycle

In `DiscoverAndAdvertise` mode, `AukiPeer`:

1. publishes current routes and mounted protocols;
2. renews the short lease;
3. replaces the entry when routes or protocols change; and
4. attempts withdrawal during ordered shutdown.

If a process crashes, its entry disappears when the lease expires. A tracker
outage does not terminate existing authenticated connections.

The DDS tracker is an HTTP directory, not the libp2p Rendezvous protocol. See
[Discovery providers](discovery-providers.md) for mDNS, Rendezvous, Kademlia,
and the difference between discovery and bootstrap.

## Try it

- [Portable Echo](../../examples/portable-echo/README.md) is the smallest
  advertise, discover, select, and exact-call example.
- [Standard Protocol Playground](../../examples/standard-protocols/README.md)
  filters discovery by all six SDK protocol families across Rust, Web, Python,
  and Swift.
- [Camera Mesh](../../examples/camera-mesh/README.md) discovers publishers but
  keeps camera access behind explicit Peer ID approval.
