# Auki SDK vision

The Auki SDK should make the physical world as easy to build on as the web:
robots, phones, browsers, and services can identify each other, exchange typed
data, and reason about where and when that data was produced.

The implementation should stay mechanical and boring. Applications choose
policy; the SDK supplies small, explicit primitives with bounded behavior.

## Five questions

Every participant should eventually be able to answer:

1. **Identity** — who produced or is serving this data?
2. **Spatial** — in which frame and physical space does it make sense?
3. **Temporal** — which clock produced the timestamp, and how can it be
   translated?
4. **Networking** — how can I reach and authenticate the peer?
5. **Tokenomics** — how should useful work or data be compensated?

This workspace implements the foundations of the first four. Tokenomics is not
implemented.

## Domain means physical context and authority

A **Domain** is a DDS-owned identifier for a physical space and its access
policy. Data tagged with the same Domain is asserted to concern the same place.
A Domain may contain many scenegraphs; its owner may designate a canonical Map.

A Domain is not:

- a Rust networking object;
- a topology leader or Manager;
- a mutable peer roster;
- a coordinate frame; or
- a source of application capability policy.

DDS-signed authority decides whether a Peer ID may authenticate in a Domain.
Product code separately decides whether that authenticated peer may drive a
robot, read a camera, or invoke a capability.

## One peer facade

`auki_sdk::AukiPeer` is the canonical network lifecycle on every supported
platform. It should remain a small composition of:

- one cryptographic Peer ID;
- renewable authority for one exact Domain;
- direct and relay reachability;
- mutually authenticated protocol streams;
- local routes and peer observations; and
- explicit shutdown.

Native User and trusted App credentials are prepared through `auki-auth`.
Robot and Compute products may supply externally refreshed authority through
`AukiPeer::start_external`. Both paths converge on the same transport and
protocol surface.

Relay-backed reachability is the safe default. Discovery and route publication
are separate concerns and should remain separate: a route can help locate a
peer but can never authorize it.

## Protocols are opt-in products

A peer starts with no application protocol handlers. Each protocol has:

- one immutable, versioned ID;
- a bounded transport-neutral wire contract;
- a `Client` for outbound operations;
- an `Endpoint` that owns inbound registration and lifecycle;
- an application-owned provider or handler; and
- locked wire and adversarial contract tests.

The Rust implementation is the source of truth. Native, Web/Wasm, Python, and
Swift surfaces should bind the same Rust protocol implementation instead of
reimplementing conversations in each language.

`auki-protocols` contains the SDK-owned families. Product-owned protocols may
live in their own repositories and depend on the same `AukiPeerProtocols`
surface. A protocol ID never changes meaning; incompatible wire changes get a
new ID.

Catalog v2 is kept as a wire-only codec because v3 embeds its locked log-row
shape. Registry support begins at v3. Current portable hosting is Catalog
v3/v4, Registry v3, Blob v1, Message v1, Stream v2, and Info v1. There is no
invisible version fallback.

## Local data stays network-free

`auki-session` models one long-lived local `Peer`, its content-addressed
registries, and independently minted recording `Session`s. It owns no
credentials, routes, relay, or protocol runtime.

The important local objects are:

- Sensor, Clock, Frame, Detector, Map, and Device Model Registry entries;
- Sensor, Pose, TimeTransform, Detection, and Map Logs;
- canonical manifests that pin every referenced registry hash; and
- application-controlled Detector and Mapper tasks.

Networking adapts this model without owning it. Native
`SessionProtocolProvider` projects a `Peer` + `Session` into Catalog v3/v4 and
Stream v2. `FsRegistryProvider` and `FsBlobProvider` expose immutable local
registry entries and blob ranges. Applications compose these providers with
the endpoints they want to serve and keep authorization policy at that
boundary.

## Spatial and temporal reasoning

Every position names a frame and every timestamp names a clock. The SDK should
never silently invent a global frame or clock.

Pose Logs record edges between exact Frame Registry entries. TimeTransform Logs
record relations between exact Clock Registry entries. The eventual core
consumer operations are:

```text
convert_pose(value, source_frame, target_frame, time)
convert_time(value, source_clock, target_clock)
```

Convention conversion and affine time math exist today. General graph search,
composition, interpolation policy, and trust-aware selection remain future
work.

## Resources describe data, not processes

A Resource is something another peer can consume: a live or recorded log, a
Map Log, or a live message channel. Detectors and Mappers are implementations
the application may start; their output logs are Resources.

Catalog responses are current snapshots, not promises. A row should be present
only while the matching operation is actually serviceable. Content-addressed
references let consumers verify the exact metadata used to interpret payloads.

## Cross-platform direction

The target shape is one Rust core with thin platform hosts:

| Platform | Direction |
| --- | --- |
| Native Rust | Complete `AukiPeer` and protocol surface |
| Web/Wasm | Complete User-authenticated relay peer; Rust protocols bound into the same Wasm module |
| Python | Bind the canonical peer and selected protocol clients/endpoints; pending |
| Swift/iOS | Bind the canonical peer and selected protocol clients/endpoints; pending |

Browser and mobile peers will normally be relay-backed. Persisting a browser
Peer ID is not required for the first iteration; the current Web facade uses an
ephemeral identity. App access secrets remain restricted to trusted native or
backend processes.

## What comes next

The near-term order is intentionally conservative:

1. make the Rust clients, endpoints, providers, and examples small and stable;
2. prove native/Web interop for shared Rust protocols;
3. add Python bindings over the same peer/runtime concepts;
4. add Swift/iOS bindings over the same peer/runtime concepts; and
5. design discovery and route publication from real application needs.

Discovery should not be smuggled into authentication, and platform bindings
should not fork protocol logic. Those two constraints keep the foundation
small enough to reason about.

## Engineering rules

- Bound frames, queues, concurrency, retries, and shutdown.
- Fail closed on identity, Domain, route, hash, and manifest mismatches.
- Persist one native identity per live process or pod.
- Treat `known_peers` as post-authentication observation, never authority.
- Keep protocol wire code portable and deterministic.
- Close endpoints before shutting down their peer.
- Prefer explicit composition over a runtime that guesses application policy.

The repository is the source of truth for implemented behavior. Broader design
discussion may live elsewhere, but code, tests, and maintained docs must agree.
