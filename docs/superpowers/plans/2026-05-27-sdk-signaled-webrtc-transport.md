# SDK Signaled WebRTC Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement SDK-owned Discovery-signaled WebRTC transport so native Swift/iOS and browser peers can bidirectionally exchange Auki Domain protocols over data channels.

**Architecture:** `auki-network` owns the signaled address contract, Discovery signal mailbox client, transport-neutral connection state machine, framed request/response router, and stream router. Generated JavaScript and Swift packages provide platform WebRTC backends while preserving a shared SDK API. `auki-domain` adds a signaled peer facade over the signaled network peer so apps configure and publish data without owning signaling or protocol routing.

**Tech Stack:** Rust 2024, UniFFI 0.31, wasm-bindgen, reqwest, serde_json, generated Swift packages, SwiftPM XCFramework-backed WebRTC, browser `RTCPeerConnection`, TypeScript/JavaScript package templates, Discovery `/signals`, Auki Domain protocol bytes.

---

## File Structure

- Create: `crates/auki-network/src/signaled_address.rs` - canonical `/auki-webrtc-signaling/.../p2p/...` format and parser.
- Modify: `crates/auki-network/src/lib.rs` - export signaled modules.
- Modify: `crates/auki-network/src/wasm.rs` - expose signaled address helpers to generated JavaScript.
- Modify: `crates/auki-network/src/discovery_client.rs` - add typed `/signals` send/poll client methods.
- Modify: `crates/auki-network/src/ffi.rs` - expose signal records and JSON methods through native UniFFI.
- Create: `crates/auki-network/src/signaled_peer.rs` - transport-neutral connection state machine, command/event types, framed router, and stream router.
- Modify: `crates/auki-network/bindings/javascript/*` - use shared address helpers and preserve existing browser signaling behavior.
- Modify: `crates/auki-network/bindings/swift/*` - add a generated Swift support target for native WebRTC backend integration.
- Create: `crates/auki-domain/src/signaled_peer.rs` - Domain-level facade over a signaled network peer.
- Modify: `crates/auki-domain/src/{core.rs,lib.rs,ffi.rs,wasm.rs}` - expose the signaled Domain facade.
- Modify: `crates/auki-domain/bindings/javascript/*` - expose the shared signaled Domain facade while preserving `AukiBrowserDomainPeer`.
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift` - migrate from WebRTC Direct `DomainClusterManager` bootstrap to SDK signaled Domain facade after the facade exists.
- Modify: `examples/overwatch/src/sdk/*` - migrate to the shared signaled Domain facade after generated browser bindings expose it.
- Update leaf and parent changelogs per `AGENTS.md`.

## Constraints

- App/example code must not implement Discovery signaling or WebRTC offer/answer/candidate routing.
- The signaled path is additive; do not remove the existing libp2p `DomainClusterManager`.
- Browser `AukiNetworkPeer.configureDiscoverySignaling(...)` must remain compatible during migration.
- Start with fake-backend Rust tests before real WebRTC integration.
- Do not stage unrelated Xcode user files already present in the working tree.
- Every production behavior change starts with a failing test.

## Task 1: Signaled Address Core

**Files:**
- Create: `crates/auki-network/src/signaled_address.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`

- [x] **Step 1: Write the failing Rust tests**

Add tests in `signaled_address.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_discovery_signaling_address() {
        let peer_id = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar";
        let address = format_signaled_address("http://127.0.0.1:8080/", peer_id).unwrap();

        assert_eq!(
            address,
            "/auki-webrtc-signaling/aHR0cDovLzEyNy4wLjAuMTo4MDgw/p2p/12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );

        let parsed = parse_signaled_address(&address).unwrap();
        assert_eq!(parsed.discovery_url, "http://127.0.0.1:8080");
        assert_eq!(parsed.peer_id, peer_id);
    }

    #[test]
    fn rejects_malformed_signaled_addresses() {
        assert!(parse_signaled_address("/ip4/127.0.0.1/tcp/4001").is_err());
        assert!(parse_signaled_address("/auki-webrtc-signaling/not-base64/p2p/peer").is_err());
        assert!(format_signaled_address("", "peer").is_err());
        assert!(format_signaled_address("http://127.0.0.1:8080", "").is_err());
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p auki-network signaled_address -- --nocapture`

Expected: fail to compile because `signaled_address` does not exist.

- [x] **Step 3: Write minimal implementation**

Create `signaled_address.rs` with:

```rust
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSignaledAddress {
    pub discovery_url: String,
    pub peer_id: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SignaledAddressError {
    #[error("missing discovery url")]
    MissingDiscoveryUrl,
    #[error("missing peer id")]
    MissingPeerId,
    #[error("invalid signaled address: {0}")]
    InvalidAddress(String),
    #[error("invalid discovery url encoding")]
    InvalidDiscoveryEncoding,
}

pub const SIGNALED_ADDRESS_PREFIX: &str = "/auki-webrtc-signaling/";

pub fn format_signaled_address(
    discovery_url: impl AsRef<str>,
    peer_id: impl AsRef<str>,
) -> Result<String, SignaledAddressError> {
    let discovery_url = discovery_url.as_ref().trim_end_matches('/');
    let peer_id = peer_id.as_ref();
    if discovery_url.is_empty() {
        return Err(SignaledAddressError::MissingDiscoveryUrl);
    }
    if peer_id.is_empty() {
        return Err(SignaledAddressError::MissingPeerId);
    }
    Ok(format!(
        "{SIGNALED_ADDRESS_PREFIX}{}/p2p/{peer_id}",
        URL_SAFE_NO_PAD.encode(discovery_url.as_bytes())
    ))
}

pub fn parse_signaled_address(address: &str) -> Result<ParsedSignaledAddress, SignaledAddressError> {
    let Some(rest) = address.strip_prefix(SIGNALED_ADDRESS_PREFIX) else {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    };
    let Some((encoded_url, peer_id)) = rest.split_once("/p2p/") else {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    };
    if encoded_url.is_empty() || peer_id.is_empty() {
        return Err(SignaledAddressError::InvalidAddress(address.to_string()));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded_url)
        .map_err(|_| SignaledAddressError::InvalidDiscoveryEncoding)?;
    let discovery_url =
        String::from_utf8(bytes).map_err(|_| SignaledAddressError::InvalidDiscoveryEncoding)?;
    if discovery_url.is_empty() {
        return Err(SignaledAddressError::MissingDiscoveryUrl);
    }
    Ok(ParsedSignaledAddress {
        discovery_url,
        peer_id: peer_id.to_string(),
    })
}
```

Add `pub mod signaled_address;` and `pub use signaled_address::*;` in `lib.rs`.

- [x] **Step 4: Expose wasm helpers**

Add to `wasm.rs`:

```rust
#[wasm_bindgen(js_name = formatSignaledAddress)]
pub fn format_signaled_address_js(discovery_url: String, peer_id: String) -> Result<String, JsValue> {
    crate::signaled_address::format_signaled_address(discovery_url, peer_id)
        .map_err(|err| js_error(err.to_string()))
}

#[wasm_bindgen(js_name = parseSignaledAddressJson)]
pub fn parse_signaled_address_json(address: String) -> Result<String, JsValue> {
    let parsed = crate::signaled_address::parse_signaled_address(&address)
        .map_err(|err| js_error(err.to_string()))?;
    serde_json::to_string(&serde_json::json!({
        "discovery_url": parsed.discovery_url,
        "peer_id": parsed.peer_id,
    }))
    .map_err(json_error)
}
```

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network signaled_address -- --nocapture
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
```

Expected: tests pass and wasm check succeeds.

- [x] **Step 6: Commit**

```bash
git add crates/auki-network/src/signaled_address.rs crates/auki-network/src/lib.rs crates/auki-network/src/wasm.rs crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl
git commit -m "Add signaled address helpers"
```

## Task 2: Native Discovery Signal Client and Bindings

**Files:**
- Modify: `crates/auki-network/src/discovery_client.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`
- Modify: `crates/auki-network/bindings/javascript/test/discovery-directory-client.test.mjs.tmpl`

- [x] **Step 1: Write failing tests**

Add `signal_wire_shape_is_locked` in `discovery_client.rs` and `native_discovery_signaling_is_exposed` in `tests/full_binding_surface.rs`:

```rust
#[test]
#[cfg(all(feature = "discovery_client", feature = "swarm"))]
fn native_discovery_signaling_is_exposed() {
    let client = auki_network::discovery_client("http://127.0.0.1:1".into()).unwrap();
    let request = auki_network::BindingSignalRequest {
        recipient_peer_id: "peer-b".into(),
        from_peer_id: "peer-a".into(),
        connection_id: "conn-1".into(),
        kind: "offer".into(),
        payload_json: r#"{"sdp":"v=0"}"#.into(),
    };
    assert_eq!(request.kind, "offer");
    drop(client);
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p auki-network --features discovery_client,swarm native_discovery_signaling_is_exposed -- --nocapture`

Expected: compile failure because `BindingSignalRequest` does not exist.

- [x] **Step 3: Implement typed Discovery methods**

Add `SignalRequest`, `SignalMessage`, `SignalPoll`, `DiscoveryClient::send_signal`, and `DiscoveryClient::poll_signals`. Use the same wire fields already exposed by `wasm.rs`: `from_peer_id`, `connection_id`, `kind`, `payload`.

- [x] **Step 4: Implement native UniFFI JSON methods**

Add `BindingSignalRequest` and `BindingSignalPoll` records plus `AukiDiscoveryClient.send_signal_json(...)` and `.poll_signals_json(...)`. `payload_json` parses to `serde_json::Value`; return JSON strings shaped as `SignalMessage` and `{ "messages": [...] }`.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm native_discovery_signaling_is_exposed -- --nocapture
cargo test -p auki-network --features discovery_client signal_wire_shape_is_locked -- --nocapture
```

Expected: both pass.

- [x] **Step 6: Commit**

```bash
git add crates/auki-network/src/discovery_client.rs crates/auki-network/src/ffi.rs crates/auki-network/tests/full_binding_surface.rs crates/auki-network/bindings/javascript/test/discovery-directory-client.test.mjs.tmpl
git commit -m "Expose Discovery signaling to native bindings"
```

## Task 3: Transport-Neutral Signaled Peer Core

**Files:**
- Create: `crates/auki-network/src/signaled_peer.rs`
- Modify: `crates/auki-network/src/lib.rs`

- [x] **Step 1: Write failing state-machine tests**

Add tests for outbound dial and inbound offer:

```rust
#[test]
fn outbound_dial_emits_peer_connection_command() {
    let mut peer = SignaledPeerCore::new("peer-a".into(), "http://discovery".into());
    let commands = peer.connect("peer-b".into(), "conn-1".into()).unwrap();

    assert_eq!(commands[0], SignaledPeerCommand::CreatePeerConnection {
        connection_id: "conn-1".into(),
        remote_peer_id: "peer-b".into(),
        role: SignaledPeerRole::Initiator,
    });
}

#[test]
fn inbound_offer_creates_responder_and_sets_remote_description() {
    let mut peer = SignaledPeerCore::new("peer-b".into(), "http://discovery".into());
    let commands = peer.handle_signal(SignalEnvelope {
        from_peer_id: "peer-a".into(),
        connection_id: "conn-1".into(),
        kind: "offer".into(),
        payload_json: r#"{"type":"offer","sdp":"v=0"}"#.into(),
    }).unwrap();

    assert!(commands.contains(&SignaledPeerCommand::CreatePeerConnection {
        connection_id: "conn-1".into(),
        remote_peer_id: "peer-a".into(),
        role: SignaledPeerRole::Responder,
    }));
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p auki-network signaled_peer -- --nocapture`

Expected: compile failure because `SignaledPeerCore` does not exist.

- [x] **Step 3: Implement minimal state machine**

Define `SignaledPeerRole`, `SignaledPeerCommand`, `SignalEnvelope`, and `SignaledPeerCore`. The core tracks connections by connection id and remote peer id, emits platform commands, and does not call WebRTC directly.

- [x] **Step 4: Add conflict and candidate tests**

Add tests for queued ICE before remote description and simultaneous dial duplicate closure. Implement only the state required by those tests.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network signaled_peer -- --nocapture
cargo test -p auki-network --no-default-features
```

Expected: signaled peer tests pass and no-default build remains green.

- [x] **Step 6: Commit**

```bash
git add crates/auki-network/src/signaled_peer.rs crates/auki-network/src/lib.rs
git commit -m "Add signaled peer state machine"
```

## Task 4: Framed and Stream Routers with Fake Channels

**Files:**
- Modify: `crates/auki-network/src/signaled_peer.rs`

- [x] **Step 1: Write failing framed router test**

Add a test where `handle_framed("/auki/info/0.0.1", ...)` receives request bytes from a fake remote peer and returns response bytes.

- [x] **Step 2: Write failing stream router test**

Add a test where a fake stream request for `{"sensor_id":"camera"}` creates an open request, accepts with a manifest JSON, pushes one entry, and emits a JSON stream entry message.

- [x] **Step 3: Run tests to verify red**

Run: `cargo test -p auki-network signaled_peer -- --nocapture`

Expected: compile failure for missing router methods.

- [x] **Step 4: Implement routers**

Add framed handler map, pending stream responder map, active stream map, and test helper APIs. Keep production-facing method names aligned with JavaScript: `request_framed`, `handle_framed`, `open_stream`, `handle_stream`.

- [x] **Step 5: Verify**

Run: `cargo test -p auki-network signaled_peer -- --nocapture`

Expected: framed and stream router tests pass.

- [x] **Step 6: Commit**

```bash
git add crates/auki-network/src/signaled_peer.rs
git commit -m "Route signaled framed requests and streams"
```

## Task 5: Generated JavaScript Compatibility

**Files:**
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-network/bindings/javascript/index.d.ts.tmpl`
- Modify: `crates/auki-network/bindings/javascript/src/adapter.ts.tmpl`
- Modify: generated files under `bindings/javascript/auki-network`

- [x] **Step 1: Write failing generated JavaScript tests**

Add assertions that `AukiNetworkPeer.configureDiscoverySignaling(...)` uses the same address as `wasm.formatSignaledAddress(...)`, and parsing goes through `wasm.parseSignaledAddressJson(...)`.

- [x] **Step 2: Run test to verify red**

Run:

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test -- framed-handler
```

Expected: test fails until the template uses shared helpers.

- [x] **Step 3: Update templates**

Replace local JS address formatting/parsing with:

```javascript
function signalingAddress(discoveryUrl, peerId) {
  return wasm.formatSignaledAddress(discoveryUrl, peerId);
}

function parseSignalingAddress(address) {
  const parsed = JSON.parse(wasm.parseSignaledAddressJson(address));
  return { discoveryUrl: parsed.discovery_url, peerId: parsed.peer_id };
}
```

- [x] **Step 4: Verify**

Run:

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
```

Expected: generated JavaScript tests pass.

- [x] **Step 5: Commit**

```bash
git add crates/auki-network/bindings/javascript bindings/javascript/auki-network
git commit -m "Use shared signaled address helpers in JavaScript"
```

## Task 6: Native Swift Binding Scaffold

**Files:**
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`
- Modify: `crates/auki-network/bindings/swift/Package.swift.tmpl`
- Create: `crates/auki-network/bindings/swift/Sources/AukiNetworkSignaledWebRTC/AukiSignaledWebRTC.swift.tmpl`

- [x] **Step 1: Write failing binding-surface test**

Add `native_signaled_peer_core_is_exposed` that constructs `AukiSignaledPeerCore`, reads `local_peer_id()`, and asserts `signaled_multiaddr()` starts with `/auki-webrtc-signaling/`.

- [x] **Step 2: Run test to verify red**

Run: `cargo test -p auki-network --features discovery_client,swarm native_signaled_peer_core_is_exposed -- --nocapture`

Expected: compile failure because `AukiSignaledPeerCore` is not exported.

- [x] **Step 3: Add UniFFI object wrapper**

Expose a minimal native object backed by `SignaledPeerCore` with constructor, `local_peer_id()`, and `signaled_multiaddr()`.

- [x] **Step 4: Add Swift support target template**

Add `AukiNetworkSignaledWebRTC` target with a backend protocol:

```swift
public protocol AukiWebRTCBackend {
    func createPeerConnection(connectionId: String, remotePeerId: String, role: String) async throws
    func createDataChannel(connectionId: String, label: String) async throws
    func closeConnection(connectionId: String) async
}
```

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm native_signaled_peer_core_is_exposed -- --nocapture
just generate-swift-bindings auki-network
swift build --package-path bindings/swift/auki-network
```

Expected: Rust test and Swift package build pass.

- [x] **Step 6: Commit**

```bash
git add crates/auki-network/src/ffi.rs crates/auki-network/tests/full_binding_surface.rs crates/auki-network/bindings/swift bindings/swift/auki-network
git commit -m "Expose signaled peer core to Swift bindings"
```

## Task 7: Domain Signaled Peer Facade

**Files:**
- Create: `crates/auki-domain/src/signaled_peer.rs`
- Modify: `crates/auki-domain/src/core.rs`
- Modify: `crates/auki-domain/src/lib.rs`
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `crates/auki-domain/tests/full_binding_surface.rs`

- [x] **Step 1: Write failing Domain facade test**

Add `native_signaled_domain_peer_surface_is_exposed` that constructs a test signaled Domain peer, checks `local_peer_id`, `cluster_name`, and one signaled multiaddr.

- [x] **Step 2: Run test to verify red**

Run: `cargo test -p auki-domain --test full_binding_surface native_signaled_domain_peer_surface_is_exposed -- --nocapture`

Expected: compile failure because the facade does not exist.

- [x] **Step 3: Implement minimal facade**

Create `SignaledDomainPeer` holding local peer id, Discovery URL, cluster name, a signaled network peer, static sensor catalog JSON, static resource catalog JSON, static registry entries JSON, and stream-open state.

- [x] **Step 4: Add UniFFI wrapper**

Expose `AukiSignaledDomainPeer` with constructor, `local_peer_id`, `cluster_name`, `multiaddrs`, static catalog setters, stream open drain/accept/decline, push entry, and finish stream. Full Discovery create/join wiring happens in Task 8.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p auki-domain --test full_binding_surface native_signaled_domain_peer_surface_is_exposed -- --nocapture
cargo test -p auki-domain --lib signaled_peer -- --nocapture
```

Expected: tests pass.

- [x] **Step 6: Commit**

```bash
git add crates/auki-domain/src/signaled_peer.rs crates/auki-domain/src/core.rs crates/auki-domain/src/lib.rs crates/auki-domain/src/ffi.rs crates/auki-domain/tests/full_binding_surface.rs
git commit -m "Add signaled Domain peer facade"
```

## Task 8: SDK Domain Protocol Handlers over Signaled Transport

**Files:**
- Modify: `crates/auki-domain/src/signaled_peer.rs`
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `crates/auki-domain/bindings/javascript/*`
- Modify: generated files under `bindings/javascript/auki-domain`

- [ ] **Step 1: Write failing protocol tests**

Add tests proving the facade registers handlers for `/auki/join/0.0.1`, `/auki/info/0.0.1`, `/auki/sensors/0.0.1`, `/auki/resources/0.0.1`, `/auki/registries/0.0.1`, and `/auki/stream/0.1.0` using fake signaled transport.

- [ ] **Step 2: Run tests to verify red**

Run: `cargo test -p auki-domain signaled_peer_protocols -- --nocapture`

Expected: tests fail because handlers are not registered.

- [ ] **Step 3: Implement handlers**

Wire the signaled framed router to participant info, static catalogs, static registries, Manager join admission, and stream open request state.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p auki-domain signaled_peer -- --nocapture
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-domain test -- browser-domain-peer-join
npm --prefix bindings/javascript/auki-domain test -- browser-domain-peer-stream
```

Expected: Rust tests and generated browser tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain/src/signaled_peer.rs crates/auki-domain/src/ffi.rs crates/auki-domain/bindings/javascript bindings/javascript/auki-domain
git commit -m "Serve Domain protocols over signaled transport"
```

## Task 9: Example Migration and End-to-End Verification

**Files:**
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift`
- Modify: `examples/ios/AukiCameraStreamer/README.md`
- Modify: `examples/overwatch/src/sdk/createOverwatchPeer.ts`
- Modify: `examples/overwatch/src/sdk/runtime.ts`

- [ ] **Step 1: Write failing example assertions**

Update iOS and Overwatch tests to expect `/auki-webrtc-signaling/` advertised addresses.

- [ ] **Step 2: Run tests to verify red**

Run the relevant Overwatch and iOS simulator tests. Expected: signaled assertions fail before migration.

- [ ] **Step 3: Migrate examples**

Replace iOS WebRTC Direct bootstrap with the generated signaled Domain peer facade. Keep camera capture, logs, catalog JSON, and fanout unchanged. Migrate Overwatch to the shared browser signaled Domain facade when generated.

- [ ] **Step 4: Verify automated tests**

Run:

```bash
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate javascript auki-network
python3 scripts/bindings/generate_bindings.py generate javascript auki-domain
npm --prefix examples/overwatch test
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' -derivedDataPath /tmp/auki-camera-streamer-derived-data
```

Expected: all automated tests pass.

- [ ] **Step 5: Manual physical-device smoke**

Run Discovery, Overwatch, and iOS on the same network. Confirm both peers advertise `/auki-webrtc-signaling/...`, either peer can initiate catalog requests, and Overwatch renders iOS camera frames.

- [ ] **Step 6: Commit**

```bash
git add examples/ios/AukiCameraStreamer examples/overwatch
git commit -m "Use SDK signaled transport in iOS and Overwatch"
```

## Task 10: Docs, Changelogs, and Final Verification

**Files:**
- Modify: `crates/auki-network/{README.md,src/readme.md,src/sprint.md,changelog.md}`
- Modify: `crates/auki-domain/{README.md,src/readme.md,src/sprint.md,changelog.md}`
- Modify: `crates/changelog.md`, `examples/changelog.md`, `docs/superpowers/plans/changelog.md`, `docs/superpowers/changelog.md`, `docs/changelog.md`, `changelog.md`

- [ ] **Step 1: Update docs**

Document the signaled address format, native signal binding methods, signaled peer state machine, Swift backend boundary, Domain signaled facade, and migration away from WebRTC Direct for browser interop.

- [ ] **Step 2: Update changelogs from leaf to root**

Follow `AGENTS.md` propagation immediately for each touched subtree.

- [ ] **Step 3: Run final verification**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm --lib
cargo test -p auki-network --features discovery_client,swarm --test full_binding_surface
cargo test -p auki-domain --test full_binding_surface
python3 scripts/bindings/generate_bindings.py generate swift auki-network
python3 scripts/bindings/generate_bindings.py generate swift auki-domain
python3 scripts/bindings/generate_bindings.py generate javascript auki-network
python3 scripts/bindings/generate_bindings.py generate javascript auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
npm --prefix examples/overwatch test
git diff --check
```

Expected: all pass.

- [ ] **Step 4: Commit docs and final status**

```bash
git add crates/auki-network crates/auki-domain crates/changelog.md examples/changelog.md docs changelog.md
git commit -m "Document SDK signaled WebRTC transport"
```
