# Auki SDK

Rust-first building blocks for authenticated peer-to-peer robotics and spatial
computing. The SDK records typed data locally, authenticates peers into a DDS
Domain, makes them reachable through direct or relay routes, and lets
applications opt into exact protocol versions.

Start here:

- [Build a P2P application](docs/p2p/README.md)
- [Run the native echo example](docs/p2p/getting-started.md)
- [Author a portable protocol](docs/p2p/authoring-protocols.md)
- [Understand the longer-term direction](VISION.md)
- [Look up terminology](GLOSSARY.md)

## The architecture

A **Domain** is a DDS-owned physical-space and authority boundary. It is not a
Rust runtime object, a leader, or a peer roster.

`auki_sdk::AukiPeer` is the networking runtime. One instance owns one Peer ID
inside one selected Domain:

```text
credentials + Domain choice + identity
                  |
                  v
             PreparedPeer
                  |
                  v
              AukiPeer
       authority · relay/routes · protocols · shutdown
                              |
                              v
             explicit Client / Endpoint pairs
```

The split is deliberate:

- `auki-auth` turns User or trusted native App credentials into a validated
  `PreparedPeer` for one Domain.
- `auki-sdk` owns authority renewal, authenticated transport, default DMS relay
  allocation, atomic TCP/WSS route pairs, optional DDS discovery, protocol
  registration, status, and ordered shutdown.
- `auki-protocols` provides opt-in wire contracts and portable `Client` /
  `Endpoint` APIs. A peer serves nothing until an application mounts an
  endpoint.
- `auki-session` owns the network-free `Peer`, `Session`, registries, and local
  logs.
- `SessionProtocolProvider` mechanically projects a local Session into Catalog
  v3/v4 and Stream v2 providers. The application still decides who may see or
  subscribe to it.

Routes are location hints, not authority. Relay allocation makes a peer
reachable; opt-in DDS discovery publishes and finds short-lived route and
mounted-protocol hints. Applications may instead exchange the expected Peer ID
and complete route through configuration, a peer card, or their own control
plane. See [Discover peers](docs/p2p/discovery.md).

## Protocols

`auki-protocols` has no default features. Enable only what the application
uses:

| Family | Active endpoint | Useful provider |
| --- | --- | --- |
| Info | `InfoClient` / `InfoEndpoint` | application `InfoProvider` |
| Catalog | `CatalogClient` / `CatalogEndpoint` for v3 resources and v4 maps | `SessionProtocolProvider` |
| Registry | `RegistryClient` / `RegistryEndpoint` for v3 | native `FsRegistryProvider` |
| Blob | `BlobClient` / `BlobEndpoint` for v1 | native `FsBlobProvider` |
| Message | `MessageClient` / `MessageEndpoint` for v1 | endpoint-owned channel declarations |
| Stream | `StreamClient` / `StreamEndpoint` for v2 | `SessionProtocolProvider` or an application provider |

Catalog v2 remains available only as a wire codec because v3 embeds its locked
log-row shape. Registry support begins at v3. The portable endpoints serve only
the listed current versions; compatibility is not silently negotiated.

Product protocols can live outside this repository. Keep one immutable wire
contract and its small `AukiPeer` client/endpoint in one Rust crate, then reuse
that crate from native, Web, Python, and Swift hosts. Posemesh follows this
model for its dataset protocol.

## Platform status

| Platform | Authenticated peer facade |
| --- | --- |
| Native Rust | User/App authentication, persistent identity, default TCP reservation with TCP/WSS routes, exact protocols, ordered shutdown |
| Web/Wasm | User authentication, ephemeral identity, mandatory WSS reservation with TCP/WSS routes, client and serving roles for all six standard protocols, ordered shutdown, and custom same-module Rust adapters |
| Python | User/App authentication, persistent identity, default TCP reservation with TCP/WSS routes, client and serving roles for all six standard protocols, ordered shutdown, and custom same-module Rust adapters |
| Swift/iOS | User authentication, ephemeral or application-persisted identity, default TCP reservation with TCP/WSS routes, client and serving roles for all six standard protocols, custom same-artifact Rust adapters, and ordered shutdown |

Robot and Compute products that already manage machine authority use
`AukiPeer::start_external`. They keep task, capability, heartbeat, and safety
policy; the SDK still owns transport, relay/routes, protocol hosting, and
shutdown.

Native applications should persist `auki_p2p::Identity` and run only one live
process or pod for that Peer ID. Web generates a new in-memory identity on each
start. Swift exposes canonical identity bytes but leaves persistence policy to
the application; the portable echo app intentionally stays ephemeral. Never
ship App secrets in a browser or mobile binary.

## Workspace map

The main layers are:

| Crate | Responsibility |
| --- | --- |
| [`auki-sdk`](crates/auki-sdk) | High-level `AukiPeer` lifecycle |
| [`auki-auth`](crates/auki-auth) | User/App authentication and Domain-scoped peer preparation |
| [`auki-p2p`](crates/auki-p2p) | Low-level identity, mutual authentication, native TCP/browser WSS transport, relay circuits |
| [`auki-protocols`](crates/auki-protocols) | Opt-in wire contracts, clients, endpoints, providers, and adapters |
| [`auki-session`](crates/auki-session) | Network-free recording model and log lifecycle |
| [`auki-registry`](crates/auki-registry) | Content-addressed Sensor, Clock, Frame, Detector, Map, and Device Model entries plus blobs |
| [`auki-logs`](crates/auki-logs) | Segmented append-only storage |
| [`auki-datatypes`](crates/auki-datatypes) | Shared protobuf payloads |
| [`auki-manifests`](crates/auki-manifests) | Canonical log manifests |
| [`auki-mappers`](crates/auki-mappers) | SDK-native Map producers |
| [`auki-maps`](crates/auki-maps) | Deterministic Map accumulation and updates |
| [`auki-time`](crates/auki-time) | Local clocks and time-transform math |
| [`auki-geometry`](crates/auki-geometry) | Coordinate-convention conversion |

Supporting crates provide canonical JSON, hashing, identity, filesystem layout,
detectors, and ROS adapters. The Python directory also exposes the native peer
facade; the Web directory exposes the browser peer and built-in protocol
facade. Swift exposes the native peer lifecycle through one generated Apple
artifact; application protocols compile their Rust adapter into that same
artifact.

## Example

[`examples/portable-echo`](examples/portable-echo) demonstrates one bounded
Rust protocol shared by native, Web, Python, and Swift applications. Its hosts
prove that exact relay routes interoperate across those runtimes; they do not
provide discovery.

The [standard protocol playground](examples/standard-protocols) exposes matching
client and serving roles for all six families in Rust, Web/Wasm, Python, and
Swift/iOS. It proves the same Rust wire implementations across native, browser,
Python, and iOS simulator peers; portable echo remains the small custom-protocol
authoring example.

## Release status

The workspace is pre-stable. The coordinated source target is `0.1.0` with an
MSRV of Rust `1.89.0`. A source version does not imply that a Git tag, crates,
wheels, Web package, or mobile facade has been published. Downstream projects
should pin one reviewed SDK revision until a release is cut.

MIT — see [LICENSE](LICENSE). Copyright © 2026 Auki Labs Limited.
