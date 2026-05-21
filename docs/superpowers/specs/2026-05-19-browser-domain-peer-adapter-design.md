# Browser Domain Peer Adapter Design

## Purpose

Park's web-peer Milestone 0 exposed the next missing SDK surface: a browser client must be able to become a real Auki Domain peer without Park inventing a parallel networking path.

The browser adapter is an SDK-owned package that lets a web app load persistent peer identity, discover Domains, join or create a Domain, publish participant and sensor metadata, and eventually stream audio/video sensor frames through SDK protocols. Park is the first consumer, but the adapter belongs in `auki-sdk`.

## Product Contract

The first consumer contract is Park's `window.aukiBrowserPeer.createPeer()` surface. The returned peer must implement these concepts:

- persistent browser peer identity
- `getSelfPeerId()`
- `listDomains(discoveryUrl)`
- `createDomain(discoveryUrl, domainName)`
- `joinDomain(discoveryUrl, domainName)`
- `leaveDomain()`
- `observeParticipants(onSnapshot)`
- `setParticipantMetadata({ appId, displayName })`
- `declareLocalSensors(sensors)`
- `setSensorPublication(sensorId, enabled)`
- `subscribeToSensor(peerId, sensorId)`
- `unsubscribeFromSensor(peerId, sensorId)`

The SDK package may expose a richer API, but Park must be able to adapt it to this contract without owning peer-to-peer transport, media streams, join protocol bytes, membership gossip, or stream framing.

## Hard Rule: SDK-Owned Networking

No app-level shortcut may move peer-to-peer data outside SDK control.

Allowed browser primitives:

- IndexedDB/localStorage for browser-owned persistent identity material.
- `fetch` for Discovery HTTP requests.
- microphone/camera APIs for local capture.
- WebSocket, WebTransport, WebRTC, wasm libp2p, or relay APIs only when they are hidden under the SDK's Domain peer model and carry SDK-defined protocols.

Disallowed shortcuts:

- WebRTC as a separate "call" product path.
- Park-owned WebSocket/WebTransport relays for media or presence.
- Manual peer-address shortcuts in Park.
- Tab-owned throwaway identity.
- A browser UI that merely remote-controls a native Park peer while pretending to be the peer.

## Existing SDK Surface

Reusable pieces:

- `auki-identity` is WASM-friendly for wallet and deterministic child derivation. Filesystem seed loading is native-only, so browser persistence must be owned by the browser package.
- `auki-network` already defines `PeerIdentity`, Discovery client shapes, `ParticipantInfo`, peer protocols, and stream protocol payloads. Its default feature set is intentionally small, while the `swarm` feature is native Tokio TCP/QUIC today.
- `auki-domain` owns `ClusterManager`, membership, Manager election, Discovery liveness, peer info/resource/sensor catalogs, registry fetching, stream opening, and shutdown.
- `auki-datatypes` owns stream payload records including `AudioFrame`.

Missing piece:

- No `package.json`, TypeScript package, wasm-pack scaffold, `wasm-bindgen` entrypoint, or browser `ClusterManager` equivalent exists today.

## Package Shape

Add a new browser binding component following the SDK's per-component binding pattern:

```text
crates/auki-domain-browser/
  README.md
  parking_lot.md
  changelog.md
  package.json
  tsconfig.json
  src/
    README.md
    sprint.md
    index.ts
    identity.ts
    discovery.ts
    peer.ts
    transport/
```

The package name should be `@aukilabs/auki-domain-browser` unless release tooling requires a different private package convention.

Rationale:

- The consumer-facing object is Domain-level, not just network-level.
- Park wants one browser peer handle, analogous to `ClusterManager`, not a pile of low-level protocol helpers.
- Lower-level browser networking helpers can still live under this package first and be split into `auki-network-browser` later if another consumer needs them.

## Identity

The adapter loads or creates a 32-byte browser seed and persists it in IndexedDB. LocalStorage may hold non-secret preferences, but not seed material.

The PeerId derivation must match the SDK locked vector:

```text
Wallet::from_seed(seed)
  .derive_child("peer/v1")
  -> PeerIdentity
  -> libp2p PeerId
```

Acceptance for identity:

- Same browser profile reloads to the same PeerId.
- Clearing the browser store creates a new PeerId.
- A fixed seed reproduces the SDK's locked PeerId vector.
- The default visible display name can be `Park <last-6-peer-id>`, but the SDK should expose identity and leave display-label policy to consumers.

## Discovery

The browser adapter can use `fetch` for Discovery because Discovery is an HTTP service and not peer-to-peer media/data transfer.

Required operations:

- list clusters/domains from `GET /clusters`
- create a cluster/domain using the SDK Discovery contract
- read Manager peer id and Manager multiaddrs from Discovery
- surface Discovery failures as structured UI-friendly errors

Discovery URL persistence remains a Park/client preference, not an SDK requirement.

## Domain Participation

The complete target is symmetric browser Domain participation:

- A browser peer can join a Domain.
- A browser peer can create a Domain and act as Manager.
- Browser peers appear in membership and participant rosters with stable PeerIds.
- Browser peers publish participant metadata, sensor declarations, and media presence.
- Browser peers can subscribe to selected remote sensors and publish selected local sensors.

The smallest honest implementation slice is narrower:

- Browser leaf peer joins an existing Domain with a native or already-browser-dialable Manager.
- It derives canonical PeerId.
- It lists Domains via Discovery.
- It dials the Manager through an SDK-owned browser transport.
- It sends `/auki/join/0.0.1`.
- It parses membership and maintains a roster.
- It fetches `/auki/info/0.0.1` for members.
- It emits Park-compatible participant snapshots.

Browser-created Domains are required for Park Milestone 0, but they are a second slice because a browser Manager must be reachable by other peers and must own admission, membership gossip, liveness, and Manager semantics.

## Browser Transport

This is the main architectural risk.

Native SDK peers currently advertise native multiaddrs such as TCP and QUIC. Browsers cannot dial arbitrary TCP/QUIC sockets. The SDK must provide a browser-compatible transport path before real browser peers can join native clusters.

Acceptable SDK-owned answers include:

- WebSocket multiaddrs served by native SDK peers and dialed by browser peers.
- WebTransport endpoints if the SDK owns protocol framing and negotiation.
- WebRTC only as an SDK transport under the Domain/libp2p model, not as an app-level call.
- An SDK relay that carries SDK peer protocols and participates in address advertisement.

The first implementation plan should not hide this. If transport is not implemented, `joinDomain` must fail with a structured `transport_unavailable`/equivalent error rather than silently using a Park-owned bridge.

## Participant Snapshot Mapping

The browser adapter should emit a stable snapshot model:

```ts
type PeerSnapshot = {
  selfPeerId: string;
  domainName: string | null;
  participants: Participant[];
  managerPeerId: string | null;
  electionState: "unknown" | "stable" | "degraded";
};
```

Rules:

- `domainName === null` means unjoined; participant rosters should be empty at the adapter boundary.
- After joining, the browser peer should see itself in `participants`.
- No remote peers is a neutral empty-Domain state, not an error.
- Missing self after join is an error.
- Manager/election state should be surfaced if available; otherwise `unknown`.

## Sensors And Audio

Milestone 0 proves the SDK peer can both produce and consume sensor streams. Microphone audio is the first concrete sensor, not a special app-level call.

Target sensor behavior:

- declare local audio sensor backed by browser microphone capture
- enable/disable local publication
- enumerate remote peer sensors
- subscribe/unsubscribe to one selected remote sensor
- expose stream health in UI-friendly terms
- capture mic audio and publish it as `AudioFrame` stream payloads
- play one selected remote audio stream
- support full open duplex
- publish symmetric media presence to all peers

These are later than identity + Discovery + roster. The first implementation slice should define the types and errors, then explicitly return unsupported for stream operations until the SDK browser transport and stream runtime are in place.

## Testing

Unit and conformance tests:

- fixed seed reproduces SDK PeerId vector
- browser identity persistence reloads correctly
- Discovery list/create error mapping
- unjoined snapshot is idle/empty
- joined self-only roster emits self plus no remotes
- malformed membership/info messages fail closed

Integration tests:

- browser package can be imported by a Vite app
- package can install `window.aukiBrowserPeer.createPeer()` for Park compatibility
- join attempt without browser transport returns structured transport error
- once transport exists, browser peer joins an existing native Manager and fetches participant info

Acceptance tests:

- two separate machines load Park's web-peer app
- both use the SDK browser adapter
- both join or create the same Domain through Discovery
- each sees itself and the other as stable `park` participants
- each sees the other's audio sensor and media presence
- both can publish and subscribe to audio with full open duplex

## Documentation

The SDK docs must clearly state:

- browser peers are true Domain peers, not remote controls for native daemons
- browser media and transport primitives are implementation details under SDK networking
- create-Domain/browser-Manager support is separate from leaf-peer join support
- Park's web-peer app is the first consumer and compatibility target

## Open Questions

1. Which browser transport should be first: WebSocket, WebTransport, WebRTC transport, or SDK relay?
2. How should native peers advertise browser-dialable addresses through Discovery?
3. Can a browser safely act as Manager in v1, or should create-Domain initially provision/require a native Manager?
4. Should the package install the global `window.aukiBrowserPeer` itself, or should Park import the package and install the global during development?
