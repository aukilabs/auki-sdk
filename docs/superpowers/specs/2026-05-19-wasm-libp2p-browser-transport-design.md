# WASM libp2p Browser Transport Spike Design

## Purpose

Park's browser peer cannot become a true Domain participant until the SDK can run browser-legal peer networking. The first `auki-domain-browser` tranche deliberately fails closed for `joinDomain`, `createDomain`, and stream operations because no browser transport exists yet.

This spike asks a narrower question:

> Can the Auki SDK run a rust-libp2p peer in the browser, derive the same SDK PeerId, and open an SDK-owned protocol stream to a native SDK peer over a browser-compatible libp2p transport?

If yes, `auki-domain-browser` should wrap that wasm networking engine. If no, the SDK should learn that quickly and choose a different SDK-owned transport path before audio work begins.

## Decision Direction

Use rust-libp2p compiled to wasm as the first transport candidate.

The browser primitive is still a transport detail. Browsers cannot dial raw TCP/QUIC sockets, so wasm libp2p must use browser-compatible transports:

- **Primary candidate:** WebRTC Direct for browser-to-native peer dialing.
- **Secondary candidate:** WebTransport if the native side can advertise compatible `/webtransport/certhash/...` multiaddrs.
- **Fallback candidate:** Secure WebSocket only if certificate/addressing constraints are acceptable for local Park development.

The user-facing rule remains unchanged: Park must not own peer-to-peer data transfer. WebRTC/WebTransport/WebSocket APIs are allowed only under the SDK/libp2p peer model.

Reference context:

- libp2p browser connectivity notes that browsers cannot dial raw TCP/QUIC and must use browser-compatible transports: <https://libp2p.io/docs/browser-connectivity/>
- libp2p documents rust-libp2p Wasm support for WebTransport: <https://libp2p.io/docs/webtransport/>
- rust-libp2p browser WebRTC support exists through wasm-bindgen/web-sys wrapping browser WebRTC data channels: <https://libp2p.io/blog/rust-libp2p-browser-webrtc/>

## Non-Goals

This spike does not implement Park audio.

This spike does not make browsers Domain Managers.

This spike does not use a Park-owned relay, app-level WebRTC call path, or bespoke TypeScript implementation of SDK peer protocols.

This spike does not require browser-to-browser connectivity. The first proof is browser-to-native Manager, because that is enough for a browser leaf peer to join a running robot Domain.

## Package Shape

Add a low-level wasm networking component, then wire it into the existing browser Domain package later:

```text
crates/auki-network-browser-wasm/
  README.md
  parking_lot.md
  changelog.md
  Cargo.toml
  src/
    README.md
    sprint.md
    lib.rs
```

`auki-network-browser-wasm` owns the rust/wasm boundary for browser libp2p networking. It is intentionally lower-level than `auki-domain-browser`.

`auki-domain-browser` remains the Park-facing TypeScript package. It should eventually consume the wasm package and continue exposing `BrowserDomainPeer`.

This split keeps the Domain adapter small while making the transport spike reusable by future browser SDK consumers.

## Spike Surface

The wasm crate should expose the smallest surface that proves or falsifies the candidate:

```ts
type WasmPeerProbe = {
  peerIdFromSeed(seed: Uint8Array): string;
  supportedTransports(): string[];
  dialAndOpenProtocol(address: string, protocol: string, requestBytes: Uint8Array): Promise<Uint8Array>;
};
```

The exact exported names can change during implementation, but the behavior should stay this small:

- derive canonical SDK PeerId from a fixed seed
- report which browser-compatible libp2p transports compiled into the wasm bundle
- dial one multiaddr and open one request/response-style SDK protocol stream

`dialAndOpenProtocol` may initially target a test-only protocol before `/auki/info/0.0.1`. The transport proof is still useful if it exercises libp2p identity, dialing, stream opening, request bytes, response bytes, and browser packaging.

## Native Test Peer

The spike needs a native peer to dial. The native side should be SDK-owned, not an external libp2p demo.

The preferred native fixture is a small example or test harness under `auki-network` that:

- derives or accepts a deterministic peer identity
- listens on the selected browser-compatible libp2p transport
- advertises the exact multiaddr the browser must dial
- serves a tiny probe protocol such as `/auki/browser-probe/0.0.1`
- echoes or returns a deterministic response

If WebRTC Direct is selected first, the native fixture should advertise the WebRTC Direct multiaddr shape expected by rust-libp2p. If that cannot be made to work with the current dependency stack, the spike should document the exact compiler/runtime blocker and try WebTransport before falling back to Secure WebSocket.

## Transport Order

### 1. WebRTC Direct

Use this first if rust-libp2p 0.56 exposes the required wasm and native transport pieces in a way compatible with the SDK.

Why:

- avoids normal browser TLS certificate authority requirements
- is designed for browser-to-node libp2p connectivity
- keeps WebRTC as a libp2p data-channel transport, not as a media call shortcut

Stop condition:

- current rust-libp2p dependency set cannot compile the browser transport to wasm
- native side cannot advertise a compatible address without major SDK restructuring
- browser API requirements need app-owned signaling outside libp2p for browser-to-native dialing

### 2. WebTransport

Try this if WebRTC Direct is blocked.

Why:

- libp2p documents Wasm support
- WebTransport gives stream-multiplexed browser-to-node transport
- certificate hashes can be encoded into multiaddrs rather than depending on normal CA trust

Risk:

- native rust-libp2p WebTransport support may not be in the same state as wasm/go/js support
- browser support is newer than WebRTC Direct and may vary by browser

### 3. Secure WebSocket

Use this only as a local-development fallback.

Why:

- widely supported and conceptually simple

Risk:

- browsers require secure WebSockets from secure contexts
- local/LAN certificate setup can become the real project instead of the SDK peer proof

## Acceptance Criteria

The spike succeeds when:

1. A fixed 32-byte seed produces the same PeerId in the wasm package as the Rust SDK locked vector.
2. The wasm package builds for a browser target.
3. A browser test page or headless browser test loads the wasm package.
4. The wasm peer dials an SDK-owned native test peer using a browser-compatible libp2p transport.
5. The wasm peer opens one named protocol stream and exchanges deterministic bytes.
6. The result is exposed in a form `auki-domain-browser` can call without Park owning transport bytes.

The spike fails usefully when:

1. It identifies the first concrete blocker by transport.
2. It records whether the blocker is dependency/version, browser API, native listener, address advertisement, certificate, or SDK architecture.
3. It leaves `auki-domain-browser` fail-closed rather than adding a shortcut.

## Testing

Unit/conformance tests:

- fixed seed to PeerId vector
- exported wasm package can be imported from JavaScript
- unsupported transport returns a typed unsupported/blocked result

Native integration tests:

- native probe peer starts and prints a dialable multiaddr
- native probe peer responds to `/auki/browser-probe/0.0.1`

Browser integration tests:

- load the wasm package in a browser-capable test harness
- create the wasm peer from a fixed seed
- dial the native fixture multiaddr
- send deterministic request bytes and receive deterministic response bytes

The implementation plan may split browser integration into a manual script first if fully automated browser + native fixture orchestration is too large for the first pass.

## Handoff To Domain Join

If the spike succeeds, the next design/plan is browser leaf join:

- Discovery returns or derives a browser-dialable Manager multiaddr.
- `auki-domain-browser` creates a wasm peer using persisted seed material.
- `joinDomain` dials the Manager through the wasm network engine.
- `joinDomain` sends `/auki/join/0.0.1`.
- The adapter parses membership and fetches `/auki/info/0.0.1`.
- Park renders the real SDK participant roster.

Audio remains after this. The audio milestone should start only once browser membership and participant info are real.

## Open Questions Left After This Spec

- Which rust-libp2p feature set actually compiles for wasm in the SDK's current dependency graph?
- Does the SDK need to update libp2p beyond 0.56 for the preferred browser transport?
- Should browser-dialable multiaddrs be registered directly in Discovery, or derived from native Manager metadata?
- Does the native Manager need to listen on both native and browser transports simultaneously?
- Should the wasm package be published as a separate npm artifact or bundled into `@aukilabs/auki-domain-browser`?
