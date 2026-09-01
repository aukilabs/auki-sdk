# The Five Questions

The SDK separates five concerns that robotics applications often blur
together:

1. **Identity — who am I?**
2. **Spatial — where am I?**
3. **Temporal — when is this?**
4. **Networking — how do I talk to you?**
5. **Tokenomics — how is value settled?**

The first four have concrete SDK layers today. Tokenomics remains future work.

| Question | Current owner |
| --- | --- |
| Identity | `auki-identity`, registries, manifests, `auki-session::Peer` |
| Spatial | Frame Registry, `auki-geometry`, spatial manifest references |
| Temporal | Clock Registry, `auki-time`, session clocks, timestamped logs |
| Networking | `auki-auth` → `PreparedPeer` → `auki_sdk::AukiPeer` → opt-in protocol endpoints |
| Tokenomics | Future wallet-backed settlement and incentives |

## Identity — Who am I?

There are several identities because they answer different questions:

- A wallet derives stable secret material.
- A network `Identity` yields the cryptographic libp2p Peer ID.
- `auki_session::Peer` owns local registries and the application ID.
- A `Session` identifies one recording timeline.
- Registry references bind metadata to exact `(peer_id, id, hash)` content.

Do not use a route, session ID, or human-readable label as a substitute for a
Peer ID. Native applications normally persist one network identity and support
one live runtime for that Peer ID.

## Spatial — Where am I?

Spatial data names its frame and convention instead of relying on implicit ROS,
OpenGL, Unity, or camera assumptions. Frame Registry entries define those
conventions; manifests and sensor entries hold immutable references to the
exact frame version.

`auki-geometry` performs convention conversion and transform math. Product
calibration still belongs to the application or device adapter.

## Temporal — When is this?

Every recorded sample is qualified by a clock. `Peer::start_session()` creates
the session timeline and its standard clocks; applications can register
additional clocks and time-transform logs when data crosses clock domains.

No networking runtime silently invents synchronized time. Consumers must keep
clock lineage and timestamp semantics attached to the data.

## Networking — How do I talk to you?

The normal trusted Rust path is mechanical:

```text
credentials + Domain selection + identity proof
                    |
                    v
               PreparedPeer
                    |
                    v
                 AukiPeer
                    |
                    v
       explicitly mounted Endpoint / Client
```

`AukiPeer` owns renewable authority, authenticated transport, relay booking,
routes, fencing, and ordered shutdown. It does not automatically choose product
protocols, discover peers, publish routes, or decide robot capability policy.

Each `auki-protocols` family is compile-time opt-in. Mounting its `Endpoint` is
the runtime opt-in for serving; the cloneable `Client` handles outbound calls.
The provider sees the verified `AuthenticatedPeer` and applies product policy.

Catalog v3 is the active general resource service. It retains v2-shaped sensor,
pose, time-transform, and detection rows while adding message-channel rows.
Catalog v2 is compatibility wire data, not the live mounted endpoint. Map Logs
use Catalog v4.

Relay allocation makes a peer reachable. Discovery remains explicit: the
application supplies the expected remote Peer ID and a complete TCP or WSS
route, and the SDK authenticates both identity and Domain before application
bytes flow.

Native Rust, Web/Wasm, and Python have an `AukiPeer` facade today. All six
standard protocol families expose client and serving roles on those hosts over
the same Rust implementations. Swift/iOS remains pending.

## Tokenomics — How is value settled?

Wallet derivation is present, but payment, metering, pricing, and incentive
policy are not part of the `0.1` peer runtime. Protocols should expose useful,
bounded operations without baking in a speculative settlement mechanism.

## How the pieces compose

Local recording and networking remain separate until an application connects
them:

```text
auki_session::Peer
        |
        +-- registries
        +-- start_session() -> Session -> clocks + logs
                                      |
                                      v
                         SessionProtocolProvider
                                      |
                                      v
                         CatalogEndpoint / StreamEndpoint
                                      |
                                      v
                                  AukiPeer
```

`SessionProtocolProvider` is a native mechanical adapter. It does not replace
product authorization or availability decisions.

## Continue

- [Auki P2P mental model](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/README.md)
- [Build with an existing protocol](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/getting-started.md)
- [Author a portable protocol](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/authoring-protocols.md)
- [Concept: peer-owned logs](Concept-Peer-Owned-Logs)

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Glossary →](Glossary)
