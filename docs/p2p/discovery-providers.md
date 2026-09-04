# Choose a discovery mechanism

[DDS discovery](discovery.md) is the only discovery mechanism integrated into
the SDK today. Add another one only when it solves a concrete product boundary,
such as offline local operation.

## The shared result

Every discovery source should produce a bounded, untrusted observation:

- a Peer ID;
- one or more routes the current runtime can dial;
- optional exact protocol hints;
- an expiry or local validity deadline; and
- a simple source label, such as `dds` or `mdns`.

Every source crosses the same trust boundary:

~~~text
candidate -> exact route dial -> Peer ID verification
          -> Domain authentication -> product authorization
~~~

Discovery does not allocate a relay, make a peer reachable, or prove what the
peer is allowed to do.

## Compare the options

| Mechanism | Best fit | First contact | Trade-off |
| --- | --- | --- | --- |
| DDS tracker | Same-Domain peers over the internet | DDS URL | Depends on the Auki control plane |
| mDNS | Peers on one local network | None | Usually stops at router or subnet boundaries |
| Rendezvous | Application namespaces over routed networks | Known Rendezvous peer | Operate a leased directory point |
| Kademlia | Decentralized overlays | Known bootstrap peers | More state, traffic, validation, and abuse controls |

Start with DDS when it meets the need.

## DDS tracker

DDS is a small HTTP directory for authenticated Domain members. It stores
short-lived Peer ID, route, and exact mounted-protocol hints. `AukiPeer` owns
publication, renewal, withdrawal, and same-Domain queries when the application
opts in.

Use DDS for the ordinary internet-connected robot, compute node, camera, or
controller. See [Discover peers](discovery.md).

## mDNS

[libp2p mDNS](https://github.com/libp2p/specs/blob/master/discovery/mdns.md)
uses multicast DNS to find peers on the same local network without a server.

It fits robot labs, development networks, and offline colocated devices. It
normally does not cross routers, subnets, VPN boundaries, or networks that
block multicast. Ordinary browser pages cannot participate directly.

mDNS usually supplies peer and route candidates, not Auki protocol metadata.
Connect and query the peer when richer information is needed.

## Rendezvous

The [libp2p Rendezvous protocol](https://github.com/libp2p/specs/blob/master/rendezvous/README.md)
lets peers register under a namespace and query that namespace through a known
Rendezvous peer. Registrations are leased and refreshed.

Rendezvous is useful when an application can operate a small set of directory
peers independently of DDS. The directory is not a Domain authority, and its
results remain untrusted.

**Bootstrap and Rendezvous are different.** Bootstrap is how a new peer learns
its first reachable contact. Rendezvous is a protocol used through that contact
to find more peers. The DDS tracker is rendezvous-like in purpose, but it does
not implement the libp2p Rendezvous wire protocol.

## Kademlia

The [libp2p Kademlia DHT](https://libp2p.io/docs/kademlia-dht/) distributes
peer, value, and content-provider lookups across an overlay. A new node still
needs reachable bootstrap peers before it can populate its routing table.

Kademlia can fit a larger decentralized network with enough stable
participants. It does not solve NAT traversal, relay allocation, Domain
authentication, or product authorization.

## Prototype another source

The SDK intentionally has no public discovery-provider trait yet. We will
generalize that boundary after a second real provider gives us two lifecycles
to compare.

Until then, keep another source at the application edge:

1. collect a bounded number of Peer IDs and routes;
2. reject malformed, expired, and unsupported routes;
3. retain a source label and local deadline;
4. deduplicate observations; and
5. pass the expected Peer ID and route into an exact Auki protocol operation.

DDS can filter by exact mounted protocol. Another source may require connecting
first and querying Info, Catalog, or a product protocol. Do not turn discovery
into a large capability document, and never treat agreement between discovery
sources as authorization.
