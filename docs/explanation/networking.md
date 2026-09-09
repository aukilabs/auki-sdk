# How Auki networking works

An `AukiPeer` is one running network participant with one Peer ID in one
selected Domain. Your app starts it, mounts any application protocols it wants
to serve, makes calls, and shuts it down.

## Identity and authority

A **Peer ID** identifies a networking keypair. Persisting the key preserves
the ID. A User or App login supplies authority to participate; several peers
may belong to the same account.

A **Domain** is the DDS authority boundary selected for the peer. DDS admits
the identity and supplies renewable signed authority. The SDK verifies the
expected remote Peer ID and Domain before an application exchange.

Successful peer authentication establishes who is connected. Your application
still decides what that peer may do.

## Finding and reaching another peer

**Discovery** supplies short-lived hints about Peer IDs, routes, and served
protocols. The SDK can query and advertise through DDS when explicitly enabled.
Applications may also exchange addresses through their own control plane.

A **route** is an address to try. Native peers can connect directly over TCP or
through a relay. Browser peers reach relay circuits over WSS. DMS allocates
relay capacity; the SDK maintains its bookings, reservations, and route updates.

Discovery and reachability are independent choices. A peer can have a relay
without advertising, or discover other peers while only making outbound calls.

## Application protocols

Your application chooses its protocol ID, messages, codec, and permissions.
The engine provides authenticated streams and bounded protocol registration;
it does not interpret application payloads or mount endpoints automatically.

`auki-protocols` currently contains optional experimental implementations.
It is the intended home for selected protocols once they are frozen as stable.
An explicit version or a locked wire fixture does not itself make a protocol
stable.

## Where Posemesh fits

| Layer | Responsibility |
| --- | --- |
| Auki networking engine | Authenticated peer lifetime, transport, relay reachability, discovery, and protocol hosting |
| Your application/protocol | Message meaning, permissions, UI, and application behavior |
| Posemesh runners | Robot and compute task execution, leases, heartbeats, and task input/output |

Posemesh composes `AukiPeer` with externally managed machine authority. Robot
peers live for the process; compute peers are scoped to tasks. The runner APIs
and their capabilities belong in
[Posemesh](https://github.com/aukilabs/posemesh/tree/main/core/compute-node),
including its
[runner interface](https://github.com/aukilabs/posemesh/tree/main/core/compute-node-runner-api).
