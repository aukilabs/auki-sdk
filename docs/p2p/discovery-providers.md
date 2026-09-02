# Choose a discovery provider

[DDS discovery](discovery.md) is the only discovery provider integrated into
the SDK today. This guide explains when another mechanism may fit better and
the contract an application-owned prototype should preserve.

The SDK does not yet expose a public discovery-provider trait. We will define
one after a second provider gives us two real lifecycles to generalize from.

## One discovery boundary

A discovery mechanism produces bounded, untrusted observations:

- a Peer ID;
- one or more canonical routes the current runtime can dial;
- optional exact served-protocol hints;
- a validity period; and
- local provenance identifying the source.

```text
discovery observation
    -> exact-route protocol dial
    -> Noise verifies the expected Peer ID
    -> Auki authentication verifies the Domain
    -> the application decides what the peer may do
```

Discovery does not authenticate a peer, make it reachable, allocate a relay,
or insert it into `known_peers()`. It should not advertise product resources
such as cameras or robot capabilities either; query those through an
authenticated application protocol after connecting.

## Choose the smallest mechanism

| Mechanism | Best fit | Initial seed | Main cost |
| --- | --- | --- | --- |
| DDS tracker | Same-Domain peers across the internet | DDS URL | Depends on the Auki control plane |
| mDNS | Devices on one local network | None | Does not normally cross network boundaries |
| Rendezvous | Application namespaces across routed networks | Reachable Rendezvous peer | Registration, polling, and point availability |
| Kademlia | Larger decentralized overlays | Reachable bootstrap peers | More state, traffic, and operational complexity |

Start with DDS when it meets the product need. Add another source only when it
solves a concrete boundary such as offline local operation or independence
from the control plane.

## DDS tracker

DDS is the currently supported provider. It returns short-lived, same-Domain
entries and can filter by exact Auki protocol ID. Authentication, publication
renewal, and withdrawal are handled by `AukiPeer` when explicitly enabled.

Use DDS for the common case: a robot, compute node, controller, or application
that already authenticates with Auki and needs to find peers over the internet.
See [Discover peers](discovery.md) for the API.

## mDNS

[libp2p mDNS](https://github.com/libp2p/specs/blob/master/discovery/mdns.md)
uses multicast DNS so peers on the same local network can find each other
without a server or configured peer.

It fits local robot labs, development networks, and offline colocated devices.
It normally does not cross routers, subnets, VPN boundaries, or networks that
block multicast. Ordinary browser pages cannot directly participate in mDNS.

mDNS supplies peer and route candidates, not exact Auki protocol metadata.
Only retain routes supported by the Auki runtime, then authenticate the peer
normally. The `_auki._tcp.local` service described by the
[control API](../control-api.md) finds a local HTTP daemon; it is separate from
libp2p peer discovery.

## Rendezvous

The [libp2p Rendezvous protocol](https://github.com/libp2p/specs/blob/master/rendezvous/README.md)
lets peers register themselves under an application namespace and lets other
peers query that namespace through a known Rendezvous peer. Registrations are
leased and must be refreshed.

Rendezvous fits application-scoped discovery when operating a small, known
set of Rendezvous points is acceptable. The point is a directory, not a Domain
authority, and its results remain untrusted until the normal Auki handshake.

The route to the first Rendezvous peer is bootstrap configuration. Rendezvous
may then help find other peers, but Rendezvous and bootstrap are not synonyms.
The DDS HTTP tracker is rendezvous-like in purpose, but it does not implement
the libp2p Rendezvous wire protocol.

## Kademlia

The [libp2p Kademlia DHT](https://libp2p.io/docs/kademlia-dht/)
routes peer, value, and content-provider lookups through a distributed overlay.
A new node still needs at least one reachable bootstrap peer before it can
populate and maintain its routing table.

Kademlia can fit a larger network that needs decentralized routing and can
operate enough stable, reachable participants. It introduces substantially
more lifecycle, privacy, validation, and abuse-control work than the other
choices, so it should follow a demonstrated need.

A Kademlia provider record means “a peer provides this DHT key.” It is not
automatically an Auki served-protocol advertisement. Kademlia also does not
solve NAT traversal or relay allocation.

## Prototype another mechanism

Until the SDK exposes a provider trait, keep a new mechanism at the application
edge:

1. discover a bounded set of Peer IDs and routes;
2. discard malformed, expired, and unsupported routes;
3. retain the source and a local validity deadline;
4. deduplicate observations without treating agreement as authorization; and
5. pass the expected Peer ID and route to an exact Auki protocol operation.

Protocol awareness is provider-dependent. DDS has an exact protocol index;
other mechanisms may require connecting first and querying Info, Catalog, or
another application protocol. Do not overload discovery with large capability
documents.

Applications may combine providers and choose discovery-only or
discover-and-advertise policy independently for each one. A candidate from any
source crosses the same exact-dial and Domain-authentication boundary.
