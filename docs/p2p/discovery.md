# Discover peers

Discovery finds peers in the same Domain without copying peer cards. V0 uses
one provider: the DDS tracker.

## Quick start

Enable the tracker when starting the peer:

```rust,ignore
use auki_protocols::info::{InfoClient, v1::ID as INFO_PROTOCOL_ID};
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
```

Find peers serving one exact protocol and use the existing protocol client:

```rust,ignore
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
```

Use `peer.discover().await?` when the application wants every current
candidate rather than one protocol.

Protocol filtering matches the complete versioned ID. V0 does not infer
prefixes, version ranges, or compatibility.

## Choose whether to advertise

DDS discovery has two explicit modes:

| Mode | Behavior |
| --- | --- |
| `DiscoverOnly` | Can query DDS but never publishes the local peer |
| `DiscoverAndAdvertise` | Can query DDS and maintains a local advertisement |

`DiscoverOnly` is useful for controllers and operator interfaces. It performs
no tracker request during peer startup and runs no publication, renewal, or
withdrawal task. Tracker errors are returned only when the application calls
`discover()`.

`DiscoverAndAdvertise` is useful for robots, compute nodes, and services. Its
first advertisement is a startup requirement; later changes are published
asynchronously.

Relay reachability is configured separately. A discover-only peer may still
own a relay, but DDS does not reveal it. Conversely, an advertising peer may
advertise explicit direct routes without owning a relay on native targets. A
Web `OutboundOnly` peer has no public route, so it supports `DiscoverOnly` but
rejects `DiscoverAndAdvertise`.

## What DDS advertises

Each short-lived advertisement contains:

| Field | Meaning |
| --- | --- |
| Peer ID | The identity bound to the publisher's DDS P2P credential |
| Routes | Current direct and relay routes supplied by the peer |
| Served protocols | Exact inbound protocol IDs currently mounted on the peer |
| Expiry | The end of the DDS-issued advertisement lease |

The SDK derives served protocols from successful endpoint mounts. Applications
do not maintain a second protocol list. Mounting or closing an endpoint updates
the DDS advertisement automatically across Rust, Web, Python, and Swift.

Every peer with a valid DDS P2P credential for the Domain can list these fields
for every unexpired advertisement in that Domain. V0 has no private protocol
advertisements or per-peer visibility rules. Discover-only peers are absent
from that list.

Because endpoints mount after peer startup, the first advertisement may have
an empty protocol list. It is replaced as soon as endpoints mount.

## What a protocol advertisement means

An advertised protocol is a filtering hint. It means the peer recently claimed
to have an inbound handler for that exact protocol ID.

It does **not** prove that:

- the peer is still online;
- a route currently works;
- the handler is still mounted;
- the peer will accept this requester; or
- the peer owns a particular camera, robot capability, or resource.

For example, advertising the Stream protocol means the peer speaks the Stream
wire contract. It does not mean the peer has a camera or offers a particular
stream. Use Info, Catalog, Registry, or the selected application protocol for
that richer information after connecting.

## Trust boundary

```text
DDS candidate
    -> exact-route protocol dial
    -> Noise verifies the expected Peer ID
    -> Auki authentication verifies the Domain
    -> known_peers() observes the live connection
```

A discovered candidate is never authorization and is never inserted directly
into `known_peers()`. The existing exact-route protocol operation remains the
security boundary.

## How the DDS provider behaves

In `DiscoverAndAdvertise`, the SDK runs a publisher that:

1. publishes its current routes and mounted inbound protocols;
2. renews the short DDS lease;
3. replaces the advertisement when routes or protocols change;
4. attempts withdrawal during ordered peer shutdown.

In either mode, `discover()` and `discover_protocol()` perform a same-Domain
lookup only when the application calls them.

If a peer crashes or cannot renew, its advertisement expires. A tracker outage
does not terminate existing authenticated connections.

DDS discovery is explicit. Without DDS tracker configuration, manual exact
routes continue to work as before.

`AukiPeerBootstrap::with_dds_tracker` is the normal configuration path and
uses the same trusted DDS origin that authenticated the session. Low-level
external-authority integrations may use
`DdsTrackerConfig::for_trusted_dds`; its endpoint receives the renewable DDS
P2P bearer and must therefore be controlled and trusted by the application.

The DDS tracker is the only Discovery v0 provider. It is an HTTP tracker, not
the libp2p Rendezvous protocol. Its bootstrap seed is the configured DDS URL;
mDNS, Rendezvous, and Kademlia remain future options.

See [Choose a discovery provider](discovery-providers.md) for how DDS differs
from mDNS, Rendezvous, and Kademlia and how to prototype another source without
weakening the authentication boundary.

## Try it

- [Portable Echo](../../examples/portable-echo/README.md) is the smallest
  advertise, discover, select, and exact-call example.
- [Standard protocol playground](../../examples/standard-protocols/README.md)
  mounts all six SDK protocol families and probes DDS-discovered peers across
  native Rust, Python, and two browser tabs. Its Swift app exposes the same
  discovery flow for simulator and device testing.
