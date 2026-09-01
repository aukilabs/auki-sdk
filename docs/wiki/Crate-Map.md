# Crate map

The workspace is layered so applications can use local data primitives without
pulling in networking, and can add authenticated networking without rebuilding
transport or relay machinery.

```text
application / product protocol
          |
          +-- auki-session          local registries and recording timelines
          +-- auki-protocols        opt-in wire contracts + Endpoint/Client
          +-- auki-sdk              authenticated AukiPeer lifecycle
                    |
                    +-- auki-auth    User/App authority preparation
                    +-- auki-p2p     authenticated libp2p transport
                    +-- auki-relay-booking
```

## Authenticated networking

### [`auki-sdk`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-sdk)

The canonical `AukiPeer` facade. It owns authority renewal, authenticated
transport, relay-backed reachability by default, routes, protocol registration,
fencing, and ordered shutdown. Native Rust, Web/Wasm, Python, and Swift/iOS
expose the same core peer and protocol concepts with platform-appropriate route
and lifecycle details.

### [`auki-auth`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-auth)

Turns trusted User email/password or App key/secret credentials into a
Peer-ID-bound `PreparedPeer` for one selected DDS Domain. It does not own
transport, relay allocation, discovery, or application protocols.

### [`auki-p2p`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-p2p)

Low-level authenticated libp2p transport: stable identities, DDS-signed
credentials, mutual-authentication framing, exact routes, relay reservations,
and peer observations. Most applications use it through `auki-sdk`.

### [`auki-relay-booking`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-relay-booking)

Bounded DMS relay-booking API and shared booking types. `AukiPeer` owns the
normal allocation and cleanup lifecycle.

### [`auki-protocols`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-protocols)

Compile-time opt-in SDK protocol families: info, Catalog, registry, blob,
message, and typed stream. Each family owns its exact IDs, wire types, framing,
validation, locked vectors, and—when its endpoint feature is selected—the
portable `Endpoint` and cloneable `Client` that mount on `AukiPeer`.

Catalog v3 is the live general resource endpoint. It carries unchanged Catalog
v2 log rows plus v3 message-channel rows. Catalog v2 remains available as a
wire-compatibility schema; Map Logs use Catalog v4.

## Local data model

### [`auki-session`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-session)

Network-free local application model. `Peer` owns durable registries;
`Peer::start_session()` creates one recording timeline with clocks and log
handles. The optional native `auki-protocols` session adapter projects this
state into Catalog and Stream providers.

### Data and storage crates

| Crate | Responsibility |
| --- | --- |
| [`auki-datatypes`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-datatypes) | Shared protobuf payloads and stream envelopes |
| [`auki-manifests`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-manifests) | Canonical log manifests and builders |
| [`auki-registry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-registry) | Sensor, Clock, Frame, Detector, Map, and related registries |
| [`auki-logs`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-logs) | Segmented append-only logs with retention and rollover |
| [`auki-layout`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-layout) | Canonical on-disk paths |
| [`auki-maps`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-maps) | Map data operations |
| [`auki-mappers`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-mappers) | Mapping-oriented composition over retained schemas |

## Foundations and adapters

| Crate | Responsibility |
| --- | --- |
| [`auki-hash`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-hash) | Content hashes |
| [`auki-jcs`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-jcs) | RFC 8785 JSON canonicalization |
| [`auki-identity`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-identity) | Wallet and deterministic child-key derivation |
| [`auki-time`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-time) | Clocks and time transforms |
| [`auki-geometry`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-geometry) | Spatial convention and transform math |
| [`auki-ros-adapter`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-ros-adapter) | ROS data conversion |
| [`auki-qr-detector`](https://github.com/aukilabs/auki-sdk/tree/develop/crates/auki-qr-detector) | QR detection support |

## Language bindings

- **Web/Wasm:** [`auki-sdk-web`](https://github.com/aukilabs/auki-sdk/tree/develop/bindings/web/auki-sdk-web)
  provides User authentication, Domain selection, ephemeral browser peers,
  required TCP/WSS relay reachability, and client/serving adapters for all six
  standard Rust protocols.
- **Python:** `auki-sdk-py` provides User/App authentication, Domain selection,
  persistent native peers, required TCP/WSS relay reachability, and same-module
  client/serving adapters for all six standard Rust protocols. Component
  bindings remain available separately.
- **Swift/iOS:** `auki-sdk-swift` provides User authentication, Domain
  selection, identity bytes, native relay-backed peers, routes, and ordered
  shutdown, plus client and serving adapters for all six standard protocols.
  Product protocols compile a UniFFI adapter into the same umbrella
  XCFramework.

## What an application normally imports

- Authenticated Rust networking: `auki-auth`, `auki-sdk`, and only the required
  `auki-protocols` endpoint features.
- Local recording: `auki-session`, plus specific data or registry types needed
  by the product.
- A product-specific protocol: one small Rust crate containing its private wire
  and endpoint modules; native, Web, Python, and Swift hosts bind the same
  public API.

See [Auki P2P](https://github.com/aukilabs/auki-sdk/blob/develop/docs/p2p/README.md)
for the runtime mental model and [Quickstart](Quickstart) for the local recording
path.

---

[← Back to: Design + Architecture](Design-and-Architecture) · [Release history →](Release-History)
