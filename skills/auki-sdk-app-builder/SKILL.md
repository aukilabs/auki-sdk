---
name: auki-sdk-app-builder
description: Build or review Auki SDK applications, robot producers, and portable peer protocols using the public AukiPeer, Peer/Session, registry, log, and geometry surfaces. Use when application code must authenticate, connect peers, mount or author a protocol, expose robot data, or decide whether behavior belongs in the SDK or an app-side adapter; not for implementing SDK internals.
---

# Auki SDK App Builder

Keep applications thin: compose public SDK owners, then add product behavior at
the edge. Do not recreate authentication, P2P, relay, protocol, registry, clock,
or spatial machinery in application code.

## Read the maintained P2P guides

- [Auki P2P mental model](../../docs/p2p/README.md)
- [Native Rust getting started](../../docs/p2p/getting-started.md)
- [Portable protocol authoring](../../docs/p2p/authoring-protocols.md)

Use those guides for runnable flows. This skill records the architectural
choices that should guide application work. If documentation and the pinned SDK
revision disagree, inspect that revision's public exports, tests, and examples.

## Choose the correct owner

- `auki_sdk::AukiPeer` is the canonical networking facade for new Rust and Web
  applications. It owns one authenticated Peer ID in one DDS Domain, authority
  supervision, P2P transport, DMS relay allocation, routes, protocol hosting,
  lifecycle fencing, and ordered shutdown. Native exposes readiness status;
  Web exposes a terminal lifecycle observer.
- `auki_session::Peer` is the long-lived local recording owner. Register
  peer-level frames, sensors, detectors, maps, device models, and URDF packages
  there.
- `auki_session::Session` is one recording timeline created with
  `Peer::start_session()`. It owns session clocks and sensor, pose,
  time-transform, detection, and map logs.
- `auki_protocols` contains transport-neutral SDK wire contracts and opt-in
  portable `Endpoint` / cloneable `Client` implementations for each supported
  family. Compiling a wire feature does not mount its endpoint.
- `auki_geometry` owns convention conversion, transform composition and
  inversion, and pose/matrix spatial math.

`auki_session::Peer` and `auki_sdk::AukiPeer` have different ownership roles
even when a product associates them with the same participant.

The legacy Manager and Rust/Python `Domain` runtimes are removed. Use an old tag
only to understand or migrate historical consumers. Python and Swift do not yet
have the canonical high-level `AukiPeer` facade.

## Start an authenticated peer

For a trusted native User or App host:

1. Persist identity with `auki_sdk::Identity::load_or_create`; one live runtime
   owns that Peer ID at a time.
2. Authenticate with `auki_auth::AuthClient` and User email/password or App
   access-key/secret credentials.
3. Select one Domain and authorize the exact identity proof, producing a
   `PreparedPeer`. List accessible Domains first only when presenting a choice.
4. Start `AukiPeer`. Relay-backed reachability through DMS is the default.

App secrets belong only in trusted native or headless processes. Never embed
them in a browser, mobile binary, public repository, image, URL, or log.

Robot and Compute hosts whose product control plane owns machine authentication
use `AukiPeer::start_external`. The host supplies complete authority updates and
responds to refresh requests through the returned control handle; `AukiPeer`
still owns transport, relay, protocols, fencing, status, and shutdown. Keep
task scheduling, heartbeat, and product safety policy in the host.

Native hosts may explicitly choose `AukiPeerConfig::direct_only()` when they do
not want DMS relay allocation. Inbound direct reachability needs a listener plus
a dialable route shared by the application. Configure an advertised direct
route only when the application publishes from the SDK's local route catalog;
outbound-only peers need neither. Browser peers have no direct-only mode.

## Routes, peers, and authority

- Dial an exact expected Peer ID through a complete compatible route: normally
  TCP from native Rust and WSS from a browser.
- A route is an untrusted location hint. The authenticated stream must still
  prove the expected Peer ID and selected Domain.
- Native Rust exposes confirmed local routes and authenticated peer
  observations; `known_peers()` is not an authorization roster.
- Automatic remote-peer discovery and route publication are not available yet.
  Obtain remote Peer IDs and routes from configuration, a product control
  plane, or an explicit application exchange.
- Domain authentication permits a peer onto the transport. Product capability
  and safety policy still decides who may operate a robot or invoke a command.

## Protocols stay explicit and portable

Applications should mount an existing product endpoint through
`peer.protocols()`, use its small typed API, and keep the endpoint alive while
serving. A peer serves no product protocol merely because wire types compile or
a client method exists. Close mounted endpoints before `peer.shutdown()` and
monitor the native status or Web lifecycle observer for terminal failure.

For SDK resource discovery, Catalog v3 is the live general endpoint. It carries
unchanged v2-shaped sensor, pose, time-transform, and detection rows plus v3
message-channel rows. Catalog v2 remains a compatibility wire schema; Map Logs
use Catalog v4.

When authoring a protocol, use one product-owned Rust crate with two focused
modules:

```text
wire.rs      exact ID, types, bounded codec, conversation, locked vectors
endpoint.rs  AukiPeer registration, exact-route dialing, deadlines, cleanup
```

The wire module stays executor-, transport-, and platform-neutral. The endpoint
adapts that one conversation to both native and Wasm peers. Platform hosts add
only thin bindings or UI; they do not duplicate the codec. Protocol authors own
the ID, bounds, versioning, compatibility tests, and endpoint behavior. Change
the exact protocol ID when its wire contract or observable conversation becomes
incompatible.

Do not put product endpoints in `auki_protocols` or generic application glue in
`auki_sdk`. Keep the SDK runtime generic and the protocol crate product-owned.

## Web behavior

The current Web/Wasm facade supports User authentication through
`auki-sdk-web::AukiUserSession`, accessible-Domain selection, and `AukiPeer`
startup. Each start creates a fresh in-memory identity and mandatory confirmed
WSS relay; reload or restart therefore produces a new Peer ID. Web applications
use exact WSS routes, mount portable Rust endpoints through thin Wasm bindings,
and close endpoints before shutting down the peer. They do not accept App
credentials or persist credentials or identity.

## Recording and spatial data

Register durable metadata on `auki_session::Peer`, start a `Session`, then
register extra clocks and owned logs on that session. Keep registry references
and hashes attached to data rather than guessing contracts from names. Preserve
the source clock, frame, payload type, sequence, and timestamp semantics.

Use `auki_geometry` for frame convention conversion, point/vector/direction and
pose conversion, transform composition or inversion, and matrix conversion.
When spatial output is wrong, verify registry conventions, transform direction,
and timestamp alignment before applying device-specific calibration. Do not fix
spatial bugs with unexplained sign flips, axis swaps, or quaternion reordering.

## Robot producers

Inventory the actual vendor APIs, runtime frame names, calibration sources, and
availability behavior before declaring resources. Consider cameras and depth,
lidar/radar/point clouds, joints and actuators, audio/IMU/force state, poses and
extrinsics, frames, clocks, and other capabilities the product truly exposes.

For each advertised resource, confirm:

- stable resource ID and correct SDK variant/payload;
- required registry metadata, frame reference and convention, and clock/time
  semantics;
- whether it can accept a request or stream now; and
- open, first-sample, no-signal, error, and end-of-stream behavior.

Advertise only currently requestable resources. A temporarily unavailable
resource may disappear from the live catalog while retaining its stable ID when
it returns.

## Stop signs

Stop and inspect the SDK before adding:

- a second P2P runtime, authentication handshake, relay booker, renewal loop, or
  reconnect owner beside `AukiPeer`;
- authorization based on routes, discovery data, cached metadata, or native
  `known_peers()` observations;
- an assumption that discovery, route publication, or inbound protocols happen
  automatically;
- separate native and Web implementations of one wire protocol;
- a custom catalog, registry/hash format, payload wrapper, clock model, or
  spatial math already represented by the SDK; or
- new-app networking built around removed Manager or `Domain` semantics.

If the SDK genuinely lacks a capability, state the gap precisely and isolate the
smallest app-side adapter behind a narrow interface. Do not create a competing
public contract.

## Completion check

Before finishing an SDK application change, verify the exact pinned SDK surface,
target and authentication method; intentional protocol opt-ins; exact route and
expected-peer handling; recording metadata ownership; robot resource
availability; native/Wasm compilation as applicable; and ordered endpoint then
peer shutdown. Exercise a focused local test or real authenticated exchange for
the behavior changed.
