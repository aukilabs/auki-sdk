---
name: auki-sdk-app-builder
description: Build or review applications that use Auki SDK to authenticate, connect peers, and exchange data through application protocols. Use for app-side Rust, Python, Web, Swift, or Expo integration and for networking in compute node or robot hosts. Not for implementing SDK internals.
---

# Auki SDK app builder

Build against the public SDK API for the application's pinned revision. Use
`AukiPeer` for networking and keep application behavior in the app.

Use the SDK checkout that the app depends on. Links below are relative to this
skill in that checkout; if the skill was copied elsewhere, find the matching
paths under the checkout's `docs/` and `core/` directories.

## Start from the app's requirements

Inspect the app's dependencies, target platform, login method, selected Domain,
and service URLs. Establish whether it needs incoming connections and whether
it gets addresses from DDS or its own source. Ask only for missing choices
that affect the implementation.

Read [how networking works](../../explanation/networking.md),
[apps, services, compute nodes, and robots](../../explanation/apps-nodes-and-robots.md),
and the affected binding or example README linked from the
[platform reference](../../reference/networking.md#platforms-and-installation).
Use the [first-peer tutorial](../../tutorials/first-peer.md) as a starting flow.
If a guide and the pinned revision disagree, check that revision's public
exports, tests, and examples before choosing an API.

## Choose the right authentication path

| Host | Start with |
| --- | --- |
| User app | User-password login or a supported imported ZITADEL session |
| Trusted backend service | User login or an Auki App access key and secret |
| Compute node | Posemesh runner with DDS node registration and a signing wallet |
| Robot | Posemesh robot entrypoint with DDS robot credentials |

Keep App secrets on trusted backends. User, App, compute node, and robot
credentials are separate paths; a Peer ID alone is not a backend principal.
Check the chosen platform and deployment support in
[authentication](../../how-to/authenticate.md).

For ordinary native Rust apps, start with `AukiPeerBootstrap`. Use the binding's
session and peer API on other platforms. Persist one network identity per
running peer when the platform supports it.

When importing ZITADEL credentials, give the SDK sole ownership of refresh.
Await persistence of all replacement credentials together and retain the session
after startup or persistence failure so it can recover. Use a known Domain ID;
imported sessions cannot currently list Domains.

Posemesh manages machine authentication and DMS execution for compute nodes and
robots. For an existing host that owns authentication, inspect
[`start_external`](../../reference/networking.md#public-rust-entry-points) and
its refresh contract. Keep one authentication owner and supply complete authority
updates. Source support does not establish support in the target deployment.

## Connect with an expected peer

Use the [connection guide](../../how-to/connect.md). DDS discovery is optional
and off by default; relay use is a separate choice. Native peers use TCP routes,
while browsers use WSS relay routes. A browser making only outgoing requests
can use outbound-only mode.

Pass the expected Peer ID and a compatible address to `open_exact`, or to the
protocol client's equivalent method. Native `open` uses configured peer routes;
a DDS lookup does not populate that route list. Discovery results and
`known_peers()` observations do not grant application permissions.

## Add only the protocols the app needs

Follow [custom protocols](../../how-to/protocols.md) and adapt
[Portable Echo](../../../core/examples/portable-echo/README.md) for a small
request/response flow. Keep product protocols in application-owned crates.
`labs/auki-protocols` is optional and experimental, including when a binding
bundles it through `standard-protocols`.

Use one Rust implementation for the wire format and conversation. Compile its
adapter and the SDK into the same Python extension, Wasm module, or Swift
framework; peer handles cannot cross separately compiled SDK copies.

Choose a versioned protocol ID in the app's namespace. Enforce payload bounds
in the decoder before allocation; a declared `AukiProtocolSpec` limit does not
do that for you. Set timeouts for opening, exchanging, and closing streams.
Use a new ID for incompatible wire changes and check affected consumers.

Authorize requests using the authenticated remote peer before doing application
work. P2P admission does not grant arbitrary operations or Domain Server writes.
Submit tasks through DMS and execute them through Posemesh runners. Registering
a protocol handler does not register a DMS task capability. Robot runners must
keep physical task execution exclusive and reject new tasks while busy.

## Own the full lifecycle

Keep protocol registrations alive while serving. Observe peer readiness or
terminal lifecycle using the APIs available on the target platform. Bound
retries and preserve cancellation; let `AukiPeer` manage its connections,
renewal, and relay bookings.

Stop new requests, close handlers, and await peer shutdown, including on error.
Run peer cleanup even if handler cleanup fails. On logout, stop all peers using
the session, close the session, then remove stored credentials. To change
Domains, stop the old peer before starting the next. See
[peer lifecycle](../../how-to/lifecycle.md).

## Validate the changed behavior

Use the relevant checks in [Contributing](../../../CONTRIBUTING.md) and the
binding's README. Check native and WASM consumers when code is shared. Exercise
the changed request flow, denied operations, message limits, cancellation, and
cleanup with local fixtures as applicable. Report the exact checks and any
unverified platform or backend support.

Read harness setup before running tests or examples. The tutorial and `dev`
helpers use shared services. Testing there needs approval for the environment,
account and Domain, operations, and cleanup. Keep service endpoints aligned;
do not fall back from local fixtures to shared services. Keep raw credentials
and private peer identity files out of logs and generated examples.
