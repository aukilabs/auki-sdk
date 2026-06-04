# Relay-Backed Libp2p Transport Migration Plan

> **Superseded:** This plan used the wrong native manager address contract
> (`/p2p-circuit/webrtc/p2p/<target>`). Use
> [`2026-05-29-relay-circuit-libp2p-transport.md`](2026-05-29-relay-circuit-libp2p-transport.md)
> instead.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move Auki browser/mobile interop away from host-specific Discovery-signaled WebRTC adapters toward relay-backed libp2p transports that reuse the same Rust/network/domain protocol surfaces across hosts.

**Architecture:** Replace the current Discovery-signaled WebRTC path with relay-backed libp2p transport. Discovery becomes a cluster directory plus relay catalog, not an SDP/ICE mailbox; peers reserve public relay circuits, advertise relay-backed multiaddrs, and use libp2p streams for `/auki/join`, catalogs, registry, diagnostics, and camera streams. Browser support remains generated-package-owned, but the target is js-libp2p relay/WebRTC rather than app-owned `RTCPeerConnection` glue.

**Tech Stack:** Rust 2024, rust-libp2p 0.56, libp2p Circuit Relay v2, js-libp2p WebRTC and circuit relay, DNS multiaddrs, Discovery directory APIs, UniFFI, wasm-bindgen, generated JavaScript packages, generated Swift packages, Auki Domain stream protocols.

---

## External References Checked

- libp2p Circuit Relay documents `p2p-circuit` relay addresses and the model where a private peer keeps a long-lived outbound relay connection so others can dial through the relay: <https://libp2p.io/docs/circuit-relay/>
- libp2p WebRTC documents the difference between WebRTC Direct and relay-signaled WebRTC/private-to-private connections: <https://libp2p.io/docs/webrtc/>
- libp2p browser connectivity documents production browser requirements such as secure WebSocket relay access behind TLS: <https://libp2p.io/docs/browser-connectivity/>
- The js-libp2p WebRTC browser guide shows browser peers connecting to a relay and becoming dialable through Circuit Relay: <https://libp2p.io/docs/webrtc-browser-connectivity/>

## File Structure

- Create: `docs/relay-backed-libp2p-transport.md` - architecture decision record for the relay-backed transport direction and removal rules.
- Modify: `crates/auki-network/src/discovery_client.rs` - relay catalog DTOs and Discovery client methods.
- Modify: `crates/auki-network/src/ffi.rs` - UniFFI relay catalog records and native JSON methods.
- Modify: `crates/auki-network/src/wasm.rs` - wasm/browser relay catalog helpers.
- Modify: `crates/auki-network/src/swarm.rs` - relay server/client construction and address helpers.
- Modify: `crates/auki-network/src/network_runtime.rs` - relay reservation lifecycle and advertised relay address management.
- Create: `crates/auki-network/src/relay_catalog.rs` - relay catalog parsing, priority selection, transport filtering, and DNS multiaddr validation.
- Create: `crates/auki-network/src/relay_address.rs` - construction/validation of relay-backed manager addresses.
- Create: `crates/auki-relay/Cargo.toml` - public relay node crate once the smoke proves the runtime shape.
- Create: `crates/auki-relay/src/main.rs` - relay node binary using `auki-network` relay server plumbing.
- Modify: `Cargo.toml` - add `crates/auki-relay` to workspace after the relay node crate exists.
- Modify: `crates/auki-domain/src/ffi.rs` - optional relay-aware native bootstrap parameters for generated hosts.
- Modify: `crates/auki-domain/bindings/swift/Package.swift.tmpl` - remove signaled WebRTC support targets from the generated Swift package.
- Modify: `crates/auki-network/bindings/javascript/*` - add generated js-libp2p relay/WebRTC runtime support for browser peers.
- Modify: `examples/overwatch/src/sdk/*` - migrate Overwatch to the generated relay-backed runtime.
- Modify: `examples/ios/AukiCameraStreamer/*` - switch back to generated Rust-backed Domain runtime.
- Update leaf and parent changelogs per `AGENTS.md`.

## Non-Negotiable Constraints

- Remove `AukiSignaledWebRTCPeer` and the Discovery-signaled WebRTC Swift support targets as part of this migration; do not keep host-specific fallback transports.
- Do not make Overwatch or AukiCameraStreamer own relay signaling logic; app code may pass relay/discovery configuration, but generated SDK packages must own transport behavior.
- Discovery must not carry SDP/ICE in the relay-backed path.
- A relay-backed address returned by Discovery must be directly dialable by the generated browser SDK.
- The implementation may break current Discovery-signaled WebRTC demos; do not add feature flags to preserve that path.
- If rust-libp2p cannot act as the private WebRTC target behind relay for iOS, stop at the decision gate and report the architecture blocker instead of retaining the Swift WebRTC backend.

## Target Address Contract

Relay node public address:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>
```

Native/mobile manager advertised through Discovery:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/webrtc/p2p/<manager-peer-id>
```

Discovery cluster response:

```json
{
  "name": "demo",
  "manager_peer_id": "12D3Manager",
  "manager_addrs": [
    "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3Relay/p2p-circuit/webrtc/p2p/12D3Manager"
  ],
  "relay_policy": {
    "required": true,
    "catalog_version": "2026-05-28"
  }
}
```

## Task 1: Relay Capability Spike and Decision Gate

**Files:**
- Create: `docs/relay-backed-libp2p-transport.md`
- Create: `examples/relay-smoke/README.md`
- Create: `examples/relay-smoke/package.json`
- Create: `examples/relay-smoke/browser-smoke.mjs`
- Create: `examples/relay-smoke/changelog.md`
- Create: `examples/relay-smoke/parking_lot.md`
- Create: `crates/auki-network/examples/relay_native_target_smoke.rs`
- Modify: `crates/auki-network/Cargo.toml`
- Modify: `docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md`

- [x] **Step 1: Write the spike design note**

Create `docs/relay-backed-libp2p-transport.md` with these sections:

```markdown
# Relay-Backed Libp2p Transport

## Decision

Replace Discovery-signaled WebRTC with relay-backed libp2p transport. Discovery
remains the domain and relay directory. Circuit Relay/WebRTC carries connection
establishment and Auki protocol streams.

## Decision Gate

The relay-backed replacement is acceptable only if a generated browser peer can
dial a native iOS/Rust-backed peer through a public relay-backed multiaddr and
open `/auki/join/0.0.1` plus `/auki/stream/0.1.0`.

## Removal Rule

Remove `/auki-webrtc-signaling/.../p2p/...`, `AukiSignaledWebRTCPeer`, and the
Swift `AukiNetworkSignaledWebRTC` / `AukiDomainSignaledWebRTC` support targets
as part of the relay-backed migration. Do not keep them as fallback transports.
```

- [x] **Step 2: Write the browser relay smoke target**

Create `examples/relay-smoke/browser-smoke.mjs` that starts a js-libp2p browser-compatible node in Node.js and dials a relay-backed multiaddr from `process.env.AUKI_RELAY_TARGET_ADDR`.

Expected behavior:

```text
AUKI_RELAY_TARGET_ADDR must be a full /p2p-circuit/.../p2p/<target> address.
The script exits 0 only after libp2p reports a connection to the target peer id.
```

- [x] **Step 3: Write the native target smoke target**

Create `crates/auki-network/examples/relay_native_target_smoke.rs` that starts the current Rust `auki-network` runtime, connects to a relay multiaddr from `AUKI_RELAY_ADDR`, and prints the target relay-backed address it expects the browser to dial.

Expected behavior:

```text
The smoke exits 0 only if the native peer reserves a relay circuit and observes
an inbound browser connection over that relay-backed address.
```

- [x] **Step 4: Run the decision-gate spike**

Run:

```bash
npm install --prefix examples/relay-smoke
cargo run -p auki-network --features swarm --example relay_native_target_smoke
node examples/relay-smoke/browser-smoke.mjs
```

Expected:

```text
If both commands pass, proceed to Task 2.
If native rust-libp2p cannot accept the private WebRTC relay dial, stop and record the architecture blocker in docs/relay-backed-libp2p-transport.md. Do not preserve or revive the Discovery-signaled Swift backend as a fallback.
```

Actual result, May 28, 2026:

```text
cargo check -p auki-network --features swarm --example relay_native_target_smoke
PASS

npm install --prefix examples/relay-smoke
PASS

cargo run -p auki-network --features swarm --example relay_native_target_smoke
PARTIAL: native Rust reserves an in-process TCP relay circuit and prints
/ip4/127.0.0.1/tcp/<port>/p2p/<relay>/p2p-circuit/webrtc/p2p/<target>.

node examples/relay-smoke/browser-smoke.mjs
FAIL: relay target must use a browser-usable /ws or /wss relay path before /p2p-circuit.
```

Stop at this decision gate. The current native runtime does not provide the
browser-usable relay/WebRTC target contract required by the migration.

- [ ] **Step 5: Commit**

```bash
git add docs/relay-backed-libp2p-transport.md examples/relay-smoke docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md
git commit -m "docs: record relay-backed libp2p transport gate"
```

## Task 2: Relay Catalog in Discovery and `auki-network`

**Files:**
- Create: `crates/auki-network/src/relay_catalog.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/discovery_client.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`

- [ ] **Step 1: Write failing relay catalog unit tests**

Add tests in `relay_catalog.rs`:

```rust
#[test]
fn parses_relay_catalog_and_filters_browser_usable_relays() {
    let catalog = RelayCatalog::from_json(r#"{
      "relays": [
        {
          "peer_id": "12D3Relay",
          "addrs": ["/dns4/relay.auki.network/tcp/443/wss/p2p/12D3Relay"],
          "region": "global",
          "priority": 10,
          "transports": ["wss", "quic-v1"]
        }
      ]
    }"#).unwrap();

    let browser = catalog.browser_usable_relays();
    assert_eq!(browser.len(), 1);
    assert_eq!(browser[0].peer_id, "12D3Relay");
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test -p auki-network relay_catalog -- --nocapture
```

Expected:

```text
FAIL because relay_catalog does not exist.
```

- [ ] **Step 3: Implement relay catalog records**

Create `relay_catalog.rs` with public records:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelayCatalog {
    pub relays: Vec<RelayInfo>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelayInfo {
    pub peer_id: String,
    pub addrs: Vec<String>,
    pub region: String,
    pub priority: u32,
    pub transports: Vec<String>,
}
```

Implement:

```rust
impl RelayCatalog {
    pub fn from_json(json: &str) -> Result<Self, RelayCatalogError>;
    pub fn browser_usable_relays(&self) -> Vec<RelayInfo>;
    pub fn native_usable_relays(&self) -> Vec<RelayInfo>;
}
```

- [ ] **Step 4: Expose Discovery relay catalog methods**

Add `AukiDiscoveryClient.fetch_relay_catalog_json(timeout_ms)` to native FFI and wasm.

Expected JSON shape:

```json
{
  "relays": [
    {
      "peer_id": "12D3Relay",
      "addrs": ["/dns4/relay.auki.network/tcp/443/wss/p2p/12D3Relay"],
      "region": "global",
      "priority": 10,
      "transports": ["wss"]
    }
  ]
}
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm relay_catalog -- --nocapture
cargo test -p auki-network --features discovery_client,swarm native_discovery_client -- --nocapture
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/src crates/auki-network/tests/full_binding_surface.rs
git commit -m "feat(network): add relay catalog binding surface"
```

## Task 3: Relay-Backed Address Helpers

**Files:**
- Create: `crates/auki-network/src/relay_address.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`

- [ ] **Step 1: Write failing address-helper tests**

Add tests:

```rust
#[test]
fn builds_browser_relayed_webrtc_manager_addr() {
    let addr = format_relayed_webrtc_address(
        "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3Relay",
        "12D3Manager",
    ).unwrap();

    assert_eq!(
        addr,
        "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3Relay/p2p-circuit/webrtc/p2p/12D3Manager"
    );
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-network relay_address -- --nocapture
```

Expected:

```text
FAIL because relay_address does not exist.
```

- [ ] **Step 3: Implement relay-backed address helpers**

Expose:

```rust
pub fn format_relayed_webrtc_address(
    relay_addr: &str,
    target_peer_id: &str,
) -> Result<String, RelayAddressError>;

pub fn parse_relayed_webrtc_address(
    address: &str,
) -> Result<ParsedRelayedWebRtcAddress, RelayAddressError>;
```

Rules:

```text
relay_addr must end with /p2p/<relay-peer-id>
target_peer_id must be non-empty and parse as a libp2p peer id
result must contain /p2p-circuit/webrtc/p2p/<target-peer-id>
```

- [ ] **Step 4: Expose helpers to native and browser bindings**

Add native FFI methods:

```rust
pub fn format_relayed_webrtc_address_json(relay_addr: String, target_peer_id: String) -> Result<String, BindingNetworkError>;
pub fn parse_relayed_webrtc_address_json(address: String) -> Result<String, BindingNetworkError>;
```

Add wasm exports with matching generated JS names:

```text
formatRelayedWebRtcAddress
parseRelayedWebRtcAddressJson
```

- [ ] **Step 5: Verify**

```bash
cargo test -p auki-network relay_address -- --nocapture
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test -- test/framed-handler.test.mjs
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network bindings/javascript/auki-network
git commit -m "feat(network): add relayed WebRTC address helpers"
```

## Task 4: Public `auki-relay` Node

**Files:**
- Create: `crates/auki-relay/Cargo.toml`
- Create: `crates/auki-relay/src/main.rs`
- Modify: `Cargo.toml`
- Modify: `crates/auki-network/src/swarm.rs`
- Test: `crates/auki-relay/tests/relay_smoke.rs`

- [ ] **Step 1: Write failing relay binary smoke test**

Create `crates/auki-relay/tests/relay_smoke.rs` with:

```rust
#[test]
fn relay_binary_parses_wss_and_quic_listen_addrs() {
    let config = auki_relay::RelayConfig {
        listen_addrs: vec![
            "/ip4/0.0.0.0/tcp/9001/ws".to_string(),
            "/ip4/0.0.0.0/udp/4001/quic-v1".to_string(),
        ],
        advertise_addrs: vec![
            "/dns4/relay.local/tcp/443/wss".to_string(),
            "/dns4/relay.local/udp/4001/quic-v1".to_string(),
        ],
    };

    assert_eq!(config.listen_addrs.len(), 2);
    assert_eq!(config.advertise_addrs.len(), 2);
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-relay relay_binary_parses_wss_and_quic_listen_addrs
```

Expected:

```text
FAIL because auki-relay is not a workspace crate.
```

- [ ] **Step 3: Add `auki-relay` crate**

Create a binary crate that:

```text
accepts --listen <multiaddr> repeated
accepts --advertise <multiaddr> repeated
starts rust-libp2p relay-server behavior
prints relay peer id and advertised addresses
exits nonzero if no browser-usable /ws or /wss address is configured
```

- [ ] **Step 4: Verify local relay startup**

```bash
cargo run -p auki-relay -- \
  --listen /ip4/127.0.0.1/tcp/9001/ws \
  --advertise /dns4/localhost/tcp/9001/ws
```

Expected:

```text
stdout contains relay peer id and at least one advertised multiaddr.
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/auki-relay crates/auki-network/src/swarm.rs
git commit -m "feat(relay): add public relay node binary"
```

## Task 5: Native Relay Reservation Runtime

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`
- Modify: `crates/auki-network/src/swarm.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`

- [ ] **Step 1: Write failing runtime test**

Add a full binding surface test:

```rust
#[test]
fn native_runtime_advertises_relay_backed_addresses_after_reservation() {
    let relay_addr = "/ip4/127.0.0.1/tcp/9001/ws/p2p/12D3Relay".to_string();
    let config = BindingSwarmConfig {
        listen_multiaddrs: vec![],
        relay_multiaddrs: vec![relay_addr],
        advertise_via_relays: true,
        ..binding_test_config()
    };

    let runtime = AukiNetworkRuntime::spawn(config).unwrap();
    let advertised = runtime.advertised_multiaddrs();
    assert!(advertised.iter().any(|addr| addr.contains("/p2p-circuit")));
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-network --features discovery_client,swarm native_runtime_advertises_relay_backed_addresses_after_reservation -- --nocapture
```

Expected:

```text
FAIL because BindingSwarmConfig has no relay fields and NetworkRuntime has no advertised relay address API.
```

- [ ] **Step 3: Add relay runtime config**

Extend `BindingSwarmConfig`:

```rust
pub relay_multiaddrs: Vec<String>,
pub advertise_via_relays: bool,
```

Add runtime API:

```rust
pub fn advertised_multiaddrs(&self) -> Vec<String>;
pub fn relay_reservation_status_json(&self) -> Result<String, BindingNetworkError>;
```

- [ ] **Step 4: Implement reservation lifecycle**

Runtime behavior:

```text
on startup, dial each configured relay
reserve a circuit where supported
construct /p2p-circuit/... manager address for local peer id
renew reservations before expiry
remove stale relay addresses when relay disconnects
emit info events when relay status changes
```

- [ ] **Step 5: Verify**

```bash
cargo test -p auki-network --features discovery_client,swarm relay -- --nocapture
cargo test -p auki-network --features discovery_client,swarm --test full_binding_surface -- --test-threads=1
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network
git commit -m "feat(network): reserve relay circuits from native runtime"
```

## Task 6: Discovery Cluster Registration Uses Relay-Backed Addresses

**Files:**
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `crates/auki-domain/src/core.rs`
- Modify: `crates/auki-domain/tests/full_binding_surface.rs`
- Modify: `crates/auki-domain/bindings/swift/Sources/*`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift`

- [ ] **Step 1: Write failing domain bootstrap test**

Add a test proving a manager registers relay-backed addresses:

```rust
#[test]
fn domain_manager_registers_relay_backed_manager_addrs() {
    let manager = bootstrap_domain_cluster_manager_auto_advertise_with_relays(
        ClusterTargetMode::Create,
        "demo".into(),
        seed32(),
        "http://127.0.0.1:8080".into(),
        vec!["/dns4/relay.local/tcp/443/wss/p2p/12D3Relay".into()],
        "ios-camera".into(),
    ).unwrap();

    let addrs = manager.local_multiaddrs();
    assert!(addrs.iter().any(|addr| addr.contains("/p2p-circuit")));
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-domain --test full_binding_surface domain_manager_registers_relay_backed_manager_addrs -- --nocapture
```

Expected:

```text
FAIL because relay-aware bootstrap does not exist.
```

- [ ] **Step 3: Add relay-aware native bootstrap**

Expose:

```rust
pub async fn bootstrap_domain_cluster_manager_relay_advertise(
    target_mode: ClusterTargetMode,
    target_name: String,
    wallet_seed: Vec<u8>,
    discovery_url: String,
    relay_multiaddrs: Vec<String>,
    app_instance_id: String,
    agent_version: String,
) -> Result<Arc<DomainClusterManager>, BindingDomainError>
```

Behavior:

```text
start NetworkRuntime with no public listen address requirement
reserve relays through auki-network
register relay-backed manager addresses in Discovery
serve Domain protocols over libp2p streams
```

- [ ] **Step 4: Verify Swift generation**

```bash
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
swift build --package-path bindings/swift/auki-domain
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain bindings/swift/auki-domain bindings/swift/auki-network
git commit -m "feat(domain): bootstrap managers with relay-backed addresses"
```

## Task 7: Generated Browser Runtime Dials Relay-Backed Managers

**Files:**
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-network/bindings/javascript/src/adapter.ts.tmpl`
- Modify: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`
- Modify: `examples/overwatch/src/sdk/runtime.ts`
- Modify: `examples/overwatch/src/sdk/runtime.test.ts`

- [ ] **Step 1: Write failing generated JS test**

Add a test:

```javascript
test("AukiNetworkPeer dials relayed WebRTC manager addresses through js-libp2p", async () => {
  const peer = await AukiNetworkPeer.create({ seed: testSeed });
  const managerAddr = "/dns4/relay.local/tcp/443/wss/p2p/12D3Relay/p2p-circuit/webrtc/p2p/12D3Manager";

  await peer.configureLibp2pRelayRuntime({
    relayAddrs: ["/dns4/relay.local/tcp/443/wss/p2p/12D3Relay"],
  });

  await assert.rejects(
    () => peer.requestFramed(managerAddr, "/auki/join/0.0.1", new Uint8Array()),
    /relay dial failed/
  );
});
```

This first test can use a fake js-libp2p adapter and assert the dial path receives the relay-backed address.

- [ ] **Step 2: Run failing JS test**

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test -- test/framed-handler.test.mjs
```

Expected:

```text
FAIL because configureLibp2pRelayRuntime does not exist.
```

- [ ] **Step 3: Implement generated browser relay runtime facade**

Add generated JS APIs:

```javascript
await peer.configureLibp2pRelayRuntime({
  relayAddrs,
  transports: ["webSockets", "webRTC"],
});

await peer.requestFramed(managerAddr, protocol, payload);
await peer.openStream(managerAddr, protocol);
```

Routing rule:

```text
if address includes /p2p-circuit/webrtc/, use js-libp2p relay/WebRTC path
if address is native TCP/QUIC and runtime supports it, use existing generated runtime behavior
if address includes /auki-webrtc-signaling/, reject it as unsupported
```

- [ ] **Step 4: Move Overwatch to generated relay runtime**

In Overwatch, remove Discovery-signaled WebRTC runtime setup and use generated relay-backed libp2p runtime unconditionally:

```text
default browser transport: relay-libp2p
unsupported browser transport: discovery-signaled-webrtc
```

- [ ] **Step 5: Verify**

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
npm --prefix examples/overwatch test
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/bindings/javascript bindings/javascript/auki-network examples/overwatch
git commit -m "feat(network): add generated browser relay runtime"
```

## Task 8: Physical iOS to Overwatch Relay Smoke

**Files:**
- Modify: `examples/ios/AukiCameraStreamer/README.md`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamerTests/AukiCameraModelsTests.swift`

- [ ] **Step 1: Add runbook section**

Document:

```text
1. Start public or LAN relay.
2. Start Discovery with relay catalog enabled.
3. Start Overwatch.
4. Build AukiCameraStreamer on physical iPhone.
5. Confirm Overwatch sees participant and continuously updating camera stream.
```

- [ ] **Step 2: Add app-facing relay transport config test**

Add a test that asserts the configured transport is relay-backed:

```swift
func testSessionCanSelectRelayBackedLibp2pTransport() {
    let config = AukiCameraSession.Configuration(
        transportKind: .relayBackedLibp2p,
        relayMultiaddrs: ["/dns4/relay.local/tcp/443/wss/p2p/12D3Relay"]
    )

    XCTAssertEqual(config.transportKind.rawValue, "relay-backed-libp2p")
    XCTAssertEqual(config.relayMultiaddrs.count, 1)
}
```

- [ ] **Step 3: Run simulator tests**

```bash
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 16' CODE_SIGNING_ALLOWED=NO
```

Expected:

```text
All tests pass.
```

- [ ] **Step 4: Run manual physical smoke**

Expected browser result:

```text
participant ios-camera appears
stream opens
preview updates for at least 60 seconds
browser console has no WebRTC signaling timeout
browser console has no WebRTC connection disconnected error
```

- [ ] **Step 5: Commit**

```bash
git add examples/ios/AukiCameraStreamer
git commit -m "test(ios): document relay-backed Overwatch smoke"
```

## Task 9: Remove Host-Specific Signaled Backend

**Files:**
- Modify: `crates/auki-network/bindings/swift/Package.swift.tmpl`
- Modify: `crates/auki-network/bindings/swift/Sources/AukiNetworkSignaledWebRTC/AukiSignaledWebRTC.swift.tmpl`
- Modify: `crates/auki-domain/bindings/swift/Sources/AukiDomainSignaledWebRTC/AukiSignaledDomainWebRTC.swift.tmpl`
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`
- Modify: `examples/ios/AukiCameraStreamer/AukiCameraStreamer/AukiCameraSession.swift`
- Modify: `examples/overwatch/src/sdk/runtime.ts`
- Modify: changelogs

- [ ] **Step 1: Write failing removal tests**

Add tests that assert the old signaled address path is gone:

```javascript
test("AukiNetworkPeer rejects Discovery-signaled WebRTC addresses", async () => {
  const peer = await AukiNetworkPeer.create({ seed: testSeed });
  await assert.rejects(
    () => peer.requestFramed(
      "/auki-webrtc-signaling/aHR0cDovL2Rpc2NvdmVyeQ/p2p/12D3Peer",
      "/auki/join/0.0.1",
      new Uint8Array()
    ),
    /Discovery-signaled WebRTC is no longer supported/
  );
});
```

- [ ] **Step 2: Run tests to verify they fail before removal**

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test -- test/framed-handler.test.mjs
```

Expected:

```text
FAIL because the generated JavaScript binding still accepts /auki-webrtc-signaling addresses.
```

- [ ] **Step 3: Remove Swift signaled WebRTC support targets**

Remove:

```text
crates/auki-network/bindings/swift/Sources/AukiNetworkSignaledWebRTC/AukiSignaledWebRTC.swift.tmpl
crates/auki-domain/bindings/swift/Sources/AukiDomainSignaledWebRTC/AukiSignaledDomainWebRTC.swift.tmpl
bindings/swift/auki-network/Sources/AukiNetworkSignaledWebRTC/AukiSignaledWebRTC.swift
bindings/swift/auki-domain/Sources/AukiDomainSignaledWebRTC/AukiSignaledDomainWebRTC.swift
```

Update Swift package templates so generated packages expose only the relay-backed/native runtime targets.

- [ ] **Step 4: Remove JavaScript Discovery-signaled WebRTC path**

Remove:

```text
configureDiscoverySignaling
_connectSignalingPeer
_openSignalingDataChannel
_handleSignalMessage
_handleSignalOffer
_handleSignalAnswer
_handleSignalCandidate
/auki-webrtc-signaling address parsing from dial path
```

Keep only relay-backed libp2p routing for browser interop.

- [ ] **Step 5: Switch examples to relay-backed transport**

Change examples:

```text
AukiCameraStreamer transport: relay-backed-libp2p
Overwatch transport: relay-libp2p
No Discovery-signaled WebRTC environment variable or fallback path remains.
```

- [ ] **Step 6: Verify**

```bash
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
npm --prefix examples/overwatch test
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 16' CODE_SIGNING_ALLOWED=NO
git diff --check
```

Expected:

```text
All commands exit 0.
```

- [ ] **Step 7: Commit**

```bash
git add crates/auki-network crates/auki-domain examples
git commit -m "refactor: replace discovery-signaled webrtc with relay-backed libp2p"
```

## Migration Success Criteria

- Discovery cluster records contain relay-backed manager addresses, not `/signals` mailbox addresses.
- A browser Overwatch peer dials the iOS manager through a relay-backed libp2p multiaddr.
- `/auki/join/0.0.1`, participant info, sensor/resource catalogs, registry fetches, and `/auki/stream/0.1.0` all travel over libp2p streams.
- The iOS app uses generated Swift bindings without importing `AukiNetworkSignaledWebRTC`.
- The Discovery-signaled WebRTC backend, Swift support targets, generated browser dial path, and `/auki-webrtc-signaling` address contract are removed.

## Self-Review

- Spec coverage: the plan addresses relay discovery, public relay infrastructure, native relay reservation, browser relay dialing, iOS migration, Overwatch migration, and removal of host-specific signaling.
- Placeholder scan: no unresolved implementation placeholders are present; the main uncertainty is explicitly handled as the Task 1 decision gate.
- Type consistency: relay catalog, relay address, runtime reservation, and domain bootstrap names are consistent across tasks.
