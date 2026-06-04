# Relay Circuit Libp2p Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Discovery-signaled WebRTC path with a generated SDK transport where browsers and native iOS/Rust peers connect through libp2p Circuit Relay v2 using browser-usable WebSocket relay addresses.

**Architecture:** Use pure Circuit Relay addresses for native/mobile manager reachability, not private relay-signaled `/webrtc` addresses. Discovery publishes relay catalogs and manager relayed multiaddrs; the relay handles connection routing; Auki protocols run over libp2p streams. Generated native and browser SDK packages own transport behavior; apps pass configuration only.

**Tech Stack:** Rust 2024, rust-libp2p 0.56, libp2p Circuit Relay v2, libp2p WebSocket transport, js-libp2p WebSockets + Circuit Relay v2, DNS multiaddrs, UniFFI, wasm-bindgen, generated JavaScript/Swift packages, Auki Domain `/auki/join/0.0.1` and `/auki/stream/0.1.0`.

---

## Core Correction

The previous target address was wrong for the Native iOS Producer Peer:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/webrtc/p2p/<manager-peer-id>
```

That `/webrtc` segment is for private WebRTC relay-signaling. The native Rust/iOS manager should instead reserve a Circuit Relay v2 slot and advertise:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/p2p/<manager-peer-id>
```

The browser dials this through `webSockets()` + `circuitRelayTransport()`. The native target accepts it through rust-libp2p relay-client/server plumbing. No app-owned `RTCPeerConnection`, no Discovery SDP/ICE, no `AukiSignaledWebRTCPeer`.

## External References Checked

- libp2p Circuit Relay uses `/p2p-circuit` addresses where a private peer maintains an outbound relay reservation and other peers dial it through the relay: <https://libp2p.io/docs/circuit-relay/>
- libp2p WebRTC distinguishes WebRTC Direct from private-to-private WebRTC, and this plan intentionally avoids using private `/webrtc` as the native manager target: <https://libp2p.io/docs/webrtc/>
- libp2p browser connectivity requires browser-usable relay transports such as secure WebSockets for production browser nodes: <https://libp2p.io/docs/browser-connectivity/>
- rust-libp2p 0.56 exposes `.with_websocket(...).await` in the swarm builder when `dns` and `websocket` features are enabled.

## Non-Negotiable Constraints

- No compatibility fallback to `/auki-webrtc-signaling/.../p2p/...`.
- Remove `AukiSignaledWebRTCPeer` and Swift `AukiNetworkSignaledWebRTC` / `AukiDomainSignaledWebRTC` targets during the migration.
- Discovery must not carry SDP, ICE candidates, offers, answers, or data-channel state.
- Overwatch and AukiCameraStreamer must not own relay signaling or transport glue.
- A Discovery manager address must be directly dialable by the generated browser SDK.
- The implementation may break current Discovery-signaled WebRTC demos.

## Target Address Contract

Relay public advertised address:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>
```

Native/mobile manager advertised through Discovery:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/p2p/<manager-peer-id>
```

Local smoke target:

```text
/ip4/127.0.0.1/tcp/<port>/ws/p2p/<relay-peer-id>/p2p-circuit/p2p/<manager-peer-id>
```

Discovery cluster response:

```json
{
  "name": "demo",
  "manager_peer_id": "12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC",
  "manager_addrs": [
    "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC"
  ],
  "relay_policy": {
    "required": true,
    "catalog_version": "2026-05-29"
  }
}
```

## File Structure

- Modify: `docs/relay-backed-libp2p-transport.md` - replace `/p2p-circuit/webrtc` with pure `/p2p-circuit/p2p`, record the corrected decision.
- Modify: `docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md` - mark superseded by this plan.
- Modify: `crates/auki-network/Cargo.toml` - enable rust-libp2p `dns` and `websocket` features under `swarm`.
- Modify: `crates/auki-network/src/swarm.rs` - add async WebSocket/DNS-capable swarm builder and migrate tests.
- Modify: `crates/auki-network/src/network_runtime.rs` - add relay reservation lifecycle and advertised relay address state.
- Modify: `crates/auki-network/src/ffi.rs` - expose relay config/status to UniFFI and block signaled WebRTC surfaces.
- Modify: `crates/auki-network/src/wasm.rs` - expose relay address/catalog helpers to generated JavaScript.
- Create: `crates/auki-network/src/relay_address.rs` - format/parse pure Circuit Relay manager addresses.
- Create: `crates/auki-network/src/relay_catalog.rs` - parse and select relay catalog entries.
- Modify: `crates/auki-network/examples/relay_native_target_smoke.rs` - local `/ws` relay smoke with pure circuit target.
- Modify: `examples/relay-smoke/browser-smoke.mjs` - remove `webRTC()`, dial pure circuit target.
- Modify: `examples/relay-smoke/README.md` - update commands and expected addresses.
- Create: `crates/auki-relay/Cargo.toml` - relay node crate.
- Create: `crates/auki-relay/src/lib.rs` - relay node config parsing and validation.
- Create: `crates/auki-relay/src/main.rs` - public relay binary.
- Create: `crates/auki-relay/tests/relay_smoke.rs` - relay config and local startup smoke.
- Modify: root `Cargo.toml` - add `crates/auki-relay` workspace member.
- Modify: `crates/auki-domain/src/ffi.rs` - relay-aware bootstrap for generated native hosts.
- Modify: `crates/auki-domain/src/core.rs` - carry relay-backed manager addrs into Discovery registration.
- Modify: `crates/auki-domain/bindings/swift/Package.swift.tmpl` - remove signaled WebRTC package targets.
- Modify: `crates/auki-network/bindings/swift/Package.swift.tmpl` - remove signaled WebRTC package targets.
- Modify: `crates/auki-network/bindings/javascript/*` - generated relay-backed browser runtime.
- Modify: `examples/overwatch/src/sdk/*` - use generated relay-backed runtime.
- Modify: `examples/ios/AukiCameraStreamer/*` - use generated Rust-backed Domain runtime and relay config.
- Update: all relevant leaf and parent `changelog.md` files per `AGENTS.md`.

## Task 1: Supersede `/p2p-circuit/webrtc` and Lock the Pure Circuit Contract

**Files:**
- Modify: `docs/relay-backed-libp2p-transport.md`
- Modify: `docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md`
- Modify: `examples/relay-smoke/browser-smoke.mjs`
- Modify: `examples/relay-smoke/README.md`
- Modify: `docs/superpowers/plans/changelog.md`
- Modify: `docs/superpowers/changelog.md`
- Modify: `docs/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update the ADR decision text**

Replace the target address in `docs/relay-backed-libp2p-transport.md` with:

```text
/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/p2p/<manager-peer-id>
```

Add this explicit sentence:

```markdown
Native/mobile managers use pure Circuit Relay v2 addresses. They do not advertise
private relay-signaled `/webrtc` addresses; `/webrtc` is not part of the Native
iOS Producer Peer transport contract.
```

- [ ] **Step 2: Mark the old plan superseded**

Add this immediately below the title in `docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md`:

```markdown
> **Superseded:** This plan used the wrong native manager address contract
> (`/p2p-circuit/webrtc/p2p/<target>`). Use
> [`2026-05-29-relay-circuit-libp2p-transport.md`](2026-05-29-relay-circuit-libp2p-transport.md)
> instead.
```

- [ ] **Step 3: Change the browser smoke validation**

In `examples/relay-smoke/browser-smoke.mjs`, replace `hasPrivateWebRtcTarget` with:

```js
function hasPureCircuitTarget(addr) {
  const names = addr.getComponents().map((component) => component.name)
  const circuitIndex = names.indexOf('p2p-circuit')
  const p2pCount = names.filter((name) => name === 'p2p').length

  return circuitIndex >= 0 && p2pCount >= 2 && !names.includes('webrtc')
}
```

Replace the validation block with:

```js
if (!hasPureCircuitTarget(targetAddr)) {
  throw new Error(
    `relay target must include /p2p-circuit/p2p/<target> and must not include /webrtc: ${targetAddrString}`
  )
}
```

Remove:

```js
import { webRTC } from '@libp2p/webrtc'
```

and remove `webRTC()` from `transports`.

- [ ] **Step 4: Run the browser smoke with the old bad address**

Run:

```bash
AUKI_RELAY_TARGET_ADDR='/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/webrtc/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC' \
  node examples/relay-smoke/browser-smoke.mjs
```

Expected:

```text
FAIL with "must not include /webrtc"
```

- [ ] **Step 5: Run the browser smoke with a pure circuit but unavailable relay**

Run:

```bash
AUKI_RELAY_DIAL_TIMEOUT_MS=1000 \
AUKI_RELAY_TARGET_ADDR='/dns4/localhost/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC' \
  node examples/relay-smoke/browser-smoke.mjs
```

Expected:

```text
FAIL after dial attempt, not during address validation.
```

- [ ] **Step 6: Commit**

```bash
git add docs/relay-backed-libp2p-transport.md docs/superpowers/plans/2026-05-28-relay-backed-libp2p-transport.md examples/relay-smoke docs/superpowers/plans/changelog.md docs/superpowers/changelog.md docs/changelog.md changelog.md
git commit -m "docs: correct relay circuit transport contract"
```

## Task 2: Add WebSocket/DNS Transport to the Rust Swarm Builder

**Files:**
- Modify: `crates/auki-network/Cargo.toml`
- Modify: `crates/auki-network/src/swarm.rs`
- Modify: `crates/auki-network/src/network_runtime.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `examples/diagnostic-app/src/sdk_runtime.rs`
- Modify: `crates/auki-domain/tests/cluster_manager_integration.rs`
- Modify: `crates/auki-network/README.md`
- Modify: `crates/auki-network/src/readme.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write failing WebSocket listen test**

Add this test in `crates/auki-network/src/swarm.rs` test module:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_listens_on_websocket_address() {
    let identity = PeerIdentity::from_seed(&[91u8; 32]);
    let mut swarm = build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse().unwrap()],
            agent_version: "ws-test/0".into(),
            enable_relay_server: false,
        },
    )
    .await
    .expect("build websocket swarm");

    let addr = wait_for_listen_addr(&mut swarm).await;
    assert!(
        addr.to_string().contains("/ws"),
        "expected websocket listen addr, got {addr}"
    );
}
```

- [ ] **Step 2: Run failing test**

Run:

```bash
cargo test -p auki-network --features swarm build_listens_on_websocket_address -- --nocapture
```

Expected:

```text
FAIL because build_swarm is not async and the current libp2p feature set does not include websocket/dns.
```

- [ ] **Step 3: Enable libp2p features**

Change the `libp2p` dependency in `crates/auki-network/Cargo.toml` to include `dns` and `websocket`:

```toml
libp2p = { version = "0.56", default-features = false, features = ["tokio", "tcp", "quic", "dns", "websocket", "noise", "yamux", "identify", "ping", "relay", "request-response", "json", "macros", "ed25519"], optional = true }
```

- [ ] **Step 4: Make `build_swarm` async**

Change the signature in `crates/auki-network/src/swarm.rs`:

```rust
pub async fn build_swarm(
    identity: &PeerIdentity,
    config: SwarmConfig,
) -> Result<Swarm<Behaviour>, BuildError> {
```

Replace the builder chain with:

```rust
let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
    .with_tokio()
    .with_tcp(
        tcp::Config::default(),
        noise::Config::new,
        yamux::Config::default,
    )
    .map_err(|e| BuildError::Transport(format!("tcp: {e}")))?
    .with_quic()
    .with_dns()
    .map_err(|e| BuildError::Transport(format!("dns: {e}")))?
    .with_websocket(noise::Config::new, yamux::Config::default)
    .await
    .map_err(|e| BuildError::Transport(format!("websocket: {e}")))?
    .with_relay_client(noise::Config::new, yamux::Config::default)
    .map_err(|e| BuildError::Transport(format!("relay_client: {e}")))?
    .with_behaviour(|key, relay_client| Behaviour {
        identify: identify::Behaviour::new(
            identify::Config::new(IDENTIFY_PROTOCOL.into(), key.public())
                .with_agent_version(agent_version),
        ),
        ping: ping::Behaviour::default(),
        allow_list: allow_block_list::Behaviour::<allow_block_list::BlockedPeers>::default(),
        relay_client,
        relay: Toggle::from(
            enable_relay_server
                .then(|| relay::Behaviour::new(local_pid, relay::Config::default())),
        ),
        stream: libp2p_stream::Behaviour::new(),
    })
    .expect("behaviour construction is infallible")
    .with_swarm_config(|c| c.with_idle_connection_timeout(IDLE_TIMEOUT))
    .build();
```

- [ ] **Step 5: Update call sites**

Update every `build_swarm(...)` call found by:

```bash
rg "build_swarm\\(" crates examples
```

Rules:

```text
Inside async functions/tests: append `.await`.
Inside `AukiNetworkRuntime::spawn`: use the already-created Tokio runtime:
  runtime.block_on(swarm::build_swarm(...))
Inside sync tests: convert test to `#[tokio::test]`.
```

Concrete `AukiNetworkRuntime::spawn` replacement in `crates/auki-network/src/ffi.rs`:

```rust
let swarm = runtime
    .block_on(swarm::build_swarm(
        &identity,
        SwarmConfig {
            listen_addresses,
            agent_version: config.agent_version,
            enable_relay_server: false,
        },
    ))
    .map_err(|err| BindingNetworkError::Runtime {
        message: err.to_string(),
    })?;
```

- [ ] **Step 6: Run focused WebSocket tests**

Run:

```bash
cargo test -p auki-network --features swarm build_listens_on_websocket_address -- --nocapture
cargo test -p auki-network --features swarm relay_server_accepts_reservation -- --nocapture
```

Expected:

```text
Both pass.
```

- [ ] **Step 7: Run broader affected tests**

Run:

```bash
cargo test -p auki-network --features swarm swarm:: -- --nocapture
cargo test -p auki-domain --features swarm cluster_manager_integration -- --nocapture
cargo check -p auki-diagnostic-app
```

Expected:

```text
All pass.
```

- [ ] **Step 8: Commit**

```bash
git add crates/auki-network crates/auki-domain examples/diagnostic-app crates/changelog.md changelog.md
git commit -m "feat(network): add websocket relay transport"
```

## Task 3: Make the Relay Smoke Pass with Pure Circuit Relay

**Files:**
- Modify: `crates/auki-network/examples/relay_native_target_smoke.rs`
- Modify: `examples/relay-smoke/browser-smoke.mjs`
- Modify: `examples/relay-smoke/README.md`
- Modify: `examples/relay-smoke/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update native smoke relay listener to `/ws`**

In `crates/auki-network/examples/relay_native_target_smoke.rs`, change the default in-process relay listener to:

```rust
listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse()?],
```

Change target address formatting to:

```rust
let browser_target_addr = format!("{circuit_listen_addr}/p2p/{target_peer_id}");
```

This must produce:

```text
/ip4/127.0.0.1/tcp/<port>/ws/p2p/<relay>/p2p-circuit/p2p/<target>
```

- [ ] **Step 2: Remove browser WebRTC transport**

In `examples/relay-smoke/browser-smoke.mjs`, ensure transports are exactly:

```js
transports: [
  webSockets(),
  circuitRelayTransport()
],
```

- [ ] **Step 3: Run the full smoke**

Terminal A:

```bash
AUKI_RELAY_SMOKE_TIMEOUT_SECS=30 \
  cargo run -p auki-network --features swarm --example relay_native_target_smoke
```

Terminal B:

```bash
node examples/relay-smoke/browser-smoke.mjs
```

Expected Terminal B:

```text
connected <target-peer-id>
```

Expected Terminal A:

```text
browser_peer_id=<browser-peer-id>
```

- [ ] **Step 4: Commit**

```bash
git add crates/auki-network/examples/relay_native_target_smoke.rs examples/relay-smoke crates/auki-network/changelog.md crates/changelog.md examples/changelog.md changelog.md
git commit -m "test: prove browser to native circuit relay smoke"
```

## Task 4: Add Pure Circuit Relay Address Helpers

**Files:**
- Create: `crates/auki-network/src/relay_address.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`
- Modify: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write failing unit tests**

Create `crates/auki-network/src/relay_address.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_relayed_circuit_manager_addr() {
        let addr = format_relayed_circuit_address(
            "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx",
            "12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC",
        )
        .unwrap();

        assert_eq!(
            addr,
            "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC"
        );
    }

    #[test]
    fn rejects_webrtc_relay_target() {
        let err = parse_relayed_circuit_address(
            "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/webrtc/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC",
        )
        .unwrap_err();

        assert!(err.to_string().contains("webrtc"));
    }
}
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cargo test -p auki-network relay_address -- --nocapture
```

Expected:

```text
FAIL because module exports and helper implementations do not exist.
```

- [ ] **Step 3: Implement helper types**

Implement these public APIs:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ParsedRelayedCircuitAddress {
    pub relay_peer_id: String,
    pub target_peer_id: String,
    pub relay_addr: String,
    pub full_addr: String,
}

pub fn format_relayed_circuit_address(
    relay_addr: &str,
    target_peer_id: &str,
) -> Result<String, RelayAddressError>;

pub fn parse_relayed_circuit_address(
    address: &str,
) -> Result<ParsedRelayedCircuitAddress, RelayAddressError>;
```

Validation rules:

```text
relay_addr must parse as Multiaddr
relay_addr must end with /p2p/<relay-peer-id>
target_peer_id must parse as PeerId
full address must contain exactly /p2p/<relay>/p2p-circuit/p2p/<target>
full address must not contain /webrtc or /webrtc-direct
```

- [ ] **Step 4: Expose to UniFFI and wasm**

Native FFI names:

```rust
pub fn format_relayed_circuit_address_json(
    relay_addr: String,
    target_peer_id: String,
) -> Result<String, BindingNetworkError>;

pub fn parse_relayed_circuit_address_json(
    address: String,
) -> Result<String, BindingNetworkError>;
```

Wasm export names:

```rust
#[wasm_bindgen(js_name = formatRelayedCircuitAddress)]
pub fn format_relayed_circuit_address_wasm(
    relay_addr: String,
    target_peer_id: String,
) -> Result<String, JsValue>;

#[wasm_bindgen(js_name = parseRelayedCircuitAddressJson)]
pub fn parse_relayed_circuit_address_json_wasm(address: String) -> Result<String, JsValue>;
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network relay_address -- --nocapture
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test -- test/framed-handler.test.mjs
```

Expected:

```text
All pass.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network bindings/javascript/auki-network crates/changelog.md changelog.md
git commit -m "feat(network): add relayed circuit address helpers"
```

## Task 5: Add Relay Catalog DTOs and Discovery Client Surface

**Files:**
- Create: `crates/auki-network/src/relay_catalog.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/discovery_client.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write failing relay catalog tests**

Create `relay_catalog.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_catalog_and_filters_browser_usable_relays() {
        let catalog = RelayCatalog::from_json(r#"{
          "relays": [
            {
              "peer_id": "12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx",
              "addrs": ["/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx"],
              "region": "global",
              "priority": 10,
              "transports": ["wss"]
            },
            {
              "peer_id": "12D3KooWLbqPHcCPecVC6oBWjoC1nJsQ2gJjuz7QPBaHTPEudeGm",
              "addrs": ["/dns4/relay.auki.network/udp/4001/quic-v1/p2p/12D3KooWLbqPHcCPecVC6oBWjoC1nJsQ2gJjuz7QPBaHTPEudeGm"],
              "region": "global",
              "priority": 5,
              "transports": ["quic-v1"]
            }
          ]
        }"#).unwrap();

        let browser = catalog.browser_usable_relays();
        assert_eq!(browser.len(), 1);
        assert_eq!(browser[0].peer_id, "12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx");
    }
}
```

- [ ] **Step 2: Implement catalog API**

Public records:

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

Methods:

```rust
impl RelayCatalog {
    pub fn from_json(json: &str) -> Result<Self, RelayCatalogError>;
    pub fn browser_usable_relays(&self) -> Vec<RelayInfo>;
    pub fn native_usable_relays(&self) -> Vec<RelayInfo>;
    pub fn preferred_browser_relay_addr(&self) -> Option<String>;
}
```

- [ ] **Step 3: Add Discovery client method**

Add to native and wasm Discovery clients:

```rust
pub async fn fetch_relay_catalog_json(&self, timeout_ms: u64) -> Result<String, DiscoveryError>;
```

Endpoint contract:

```text
GET /relays
```

Expected JSON shape:

```json
{
  "relays": [
    {
      "peer_id": "12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx",
      "addrs": ["/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx"],
      "region": "global",
      "priority": 10,
      "transports": ["wss"]
    }
  ]
}
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm relay_catalog -- --nocapture
cargo test -p auki-network --features discovery_client,swarm native_discovery_client -- --nocapture
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
```

Expected:

```text
All pass.
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network crates/changelog.md changelog.md
git commit -m "feat(network): add relay catalog surface"
```

## Task 6: Add Public `auki-relay` Node

**Files:**
- Create: `crates/auki-relay/Cargo.toml`
- Create: `crates/auki-relay/src/lib.rs`
- Create: `crates/auki-relay/src/main.rs`
- Create: `crates/auki-relay/tests/relay_smoke.rs`
- Create: `crates/auki-relay/changelog.md`
- Create: `crates/auki-relay/parking_lot.md`
- Modify: root `Cargo.toml`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write failing relay config tests**

Create `crates/auki-relay/tests/relay_smoke.rs`:

```rust
#[test]
fn relay_config_requires_browser_usable_advertise_addr() {
    let config = auki_relay::RelayConfig {
        listen_addrs: vec!["/ip4/0.0.0.0/tcp/9001/ws".to_string()],
        advertise_addrs: vec!["/dns4/relay.auki.network/tcp/443/wss".to_string()],
        agent_version: "auki-relay/0".to_string(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn relay_config_rejects_quic_only_browser_relay() {
    let config = auki_relay::RelayConfig {
        listen_addrs: vec!["/ip4/0.0.0.0/udp/4001/quic-v1".to_string()],
        advertise_addrs: vec!["/dns4/relay.auki.network/udp/4001/quic-v1".to_string()],
        agent_version: "auki-relay/0".to_string(),
    };

    assert!(config.validate().unwrap_err().to_string().contains("ws"));
}
```

- [ ] **Step 2: Run failing tests**

Run:

```bash
cargo test -p auki-relay relay_config -- --nocapture
```

Expected:

```text
FAIL because `auki-relay` is not a workspace crate.
```

- [ ] **Step 3: Add crate**

`crates/auki-relay/Cargo.toml`:

```toml
[package]
name = "auki-relay"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true

[dependencies]
auki-network = { path = "../auki-network", default-features = false, features = ["swarm"] }
auki-identity = { path = "../auki-identity", default-features = false }
clap = { version = "4", features = ["derive"] }
futures = "0.3"
libp2p = { version = "0.56", default-features = false, features = ["tokio", "tcp", "quic", "dns", "websocket", "noise", "yamux", "identify", "ping", "relay", "macros", "ed25519"] }
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "time"] }
```

Add `crates/auki-relay` to root workspace members.

- [ ] **Step 4: Implement config and binary**

`RelayConfig`:

```rust
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub listen_addrs: Vec<String>,
    pub advertise_addrs: Vec<String>,
    pub agent_version: String,
}
```

Required CLI:

```text
auki-relay --listen /ip4/0.0.0.0/tcp/9001/ws --advertise /dns4/relay.auki.network/tcp/443/wss
```

Behavior:

```text
parse listen multiaddrs
validate at least one advertised addr contains /ws or /wss
start `auki-network::swarm::build_swarm(... enable_relay_server: true).await`
add advertised addresses with /p2p/<relay-peer-id>
print relay peer id and advertised relay addresses
run until SIGINT
```

- [ ] **Step 5: Verify local relay starts**

Run:

```bash
cargo run -p auki-relay -- \
  --listen /ip4/127.0.0.1/tcp/9001/ws \
  --advertise /dns4/localhost/tcp/9001/ws
```

Expected:

```text
stdout contains relay_peer_id and /dns4/localhost/tcp/9001/ws/p2p/<relay-peer-id>
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/auki-relay crates/changelog.md changelog.md
git commit -m "feat(relay): add websocket circuit relay node"
```

## Task 7: Add Native Relay Reservation Lifecycle to `NetworkRuntime`

**Files:**
- Modify: `crates/auki-network/src/network_runtime.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/tests/full_binding_surface.rs`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Extend binding config**

Add fields to `BindingSwarmConfig`:

```rust
pub relay_multiaddrs: Vec<String>,
pub advertise_via_relays: bool,
```

Update all test constructors to set:

```rust
relay_multiaddrs: vec![],
advertise_via_relays: false,
```

- [ ] **Step 2: Write failing reservation status test**

Add to `crates/auki-network/tests/full_binding_surface.rs`:

```rust
#[test]
fn binding_config_exposes_relay_fields() {
    let cfg = BindingSwarmConfig {
        wallet_seed: seed32(),
        listen_multiaddrs: vec![],
        allowed_peers: vec![],
        relay_multiaddrs: vec!["/ip4/127.0.0.1/tcp/9001/ws/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx".to_string()],
        advertise_via_relays: true,
        agent_version: "test/0".to_string(),
        heartbeat_clock_id: "clock".to_string(),
        heartbeat_clock_hash_hex: "00".repeat(32),
    };

    assert!(cfg.advertise_via_relays);
}
```

- [ ] **Step 3: Add runtime state APIs**

Expose:

```rust
impl AukiNetworkRuntime {
    pub fn advertised_multiaddrs(&self) -> Vec<String>;
    pub fn relay_reservation_status_json(&self) -> Result<String, BindingNetworkError>;
}
```

Status JSON:

```json
{
  "relays": [
    {
      "relay_addr": "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx",
      "state": "reserved",
      "advertised_addr": "/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC",
      "last_error": null
    }
  ]
}
```

- [ ] **Step 4: Implement reservation lifecycle**

Runtime behavior:

```text
on spawn, dial each configured relay
wait for identify with relay
listen_on /.../p2p/<relay>/p2p-circuit
on ReservationReqAccepted, add formatted /p2p-circuit/p2p/<local-peer> to advertised_multiaddrs
on relay disconnect, mark stale and remove advertised address
renew before expiry if libp2p exposes the reservation limit event; otherwise re-listen on reconnect
emit info events for reservation state changes
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test -p auki-network --features discovery_client,swarm relay -- --nocapture
cargo test -p auki-network --features discovery_client,swarm --test full_binding_surface -- --test-threads=1
```

Expected:

```text
All pass.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network crates/changelog.md changelog.md
git commit -m "feat(network): reserve relay circuits from native runtime"
```

## Task 8: Register Relay-Backed Manager Addresses Through Domain Bootstrap

**Files:**
- Modify: `crates/auki-domain/src/ffi.rs`
- Modify: `crates/auki-domain/src/core.rs`
- Modify: `crates/auki-domain/tests/full_binding_surface.rs`
- Modify: `crates/auki-domain/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Add relay-aware bootstrap API**

Expose:

```rust
#[uniffi::export(async_runtime = "tokio")]
pub async fn bootstrap_domain_cluster_manager_relay_advertise(
    target_mode: ClusterTargetMode,
    target_name: String,
    wallet_seed: Vec<u8>,
    discovery_url: String,
    relay_multiaddrs: Vec<String>,
    daemon_info: core::DaemonInfo,
    agent_version: String,
) -> Result<Arc<DomainClusterManager>, BindingDomainError>
```

- [ ] **Step 2: Write failing test**

Add a full binding-surface test:

```rust
#[test]
fn relay_bootstrap_api_is_exported() {
    assert_binding_symbol("bootstrap_domain_cluster_manager_relay_advertise");
}
```

- [ ] **Step 3: Implement behavior**

Behavior:

```text
build network runtime with listen_multiaddrs empty
set relay_multiaddrs from caller
set advertise_via_relays true
wait for at least one relay reservation before registering manager
register manager_addrs as pure /p2p-circuit/p2p addrs
fail startup with typed error if no relay reservation succeeds within timeout
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo test -p auki-domain --features discovery_client,swarm --test full_binding_surface relay_bootstrap -- --nocapture
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
swift build --package-path bindings/swift/auki-domain
```

Expected:

```text
All pass.
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-domain bindings/swift crates/changelog.md changelog.md
git commit -m "feat(domain): register relay-backed manager addresses"
```

## Task 9: Generated Browser Runtime Dials Pure Circuit Addresses

**Files:**
- Modify: `crates/auki-network/bindings/javascript/package.json.tmpl`
- Modify: `crates/auki-network/bindings/javascript/src/*.tmpl`
- Modify: `crates/auki-network/bindings/javascript/test/*.tmpl`
- Modify: `examples/overwatch/src/sdk/*`
- Modify: `examples/overwatch/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write generated JS test**

Add a generated JavaScript test that dials a pure circuit address:

```js
test('browser runtime accepts pure circuit relay manager address', async () => {
  const addr = '/dns4/relay.auki.network/tcp/443/wss/p2p/12D3KooWR4gqATy2UShTaJqs6Cp9us1kEeRzGrpvozp774EQSkgx/p2p-circuit/p2p/12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC'
  const parsed = parseRelayedCircuitAddressJson(addr)
  assert.equal(JSON.parse(parsed).target_peer_id, '12D3KooWQ9ZkuDS6UF3AeXusJbx4RjaZUkKa7y3ZK7y38zGBsuCC')
})
```

- [ ] **Step 2: Remove signaled WebRTC browser path**

Delete generated code paths that recognize:

```text
/auki-webrtc-signaling/
```

Generated browser runtime transports:

```js
transports: [
  webSockets(),
  circuitRelayTransport()
]
```

- [ ] **Step 3: Route Overwatch through generated runtime**

Overwatch must consume manager addresses from Discovery and call the generated runtime dial/open APIs. It must not parse SDP, poll `/signals`, or instantiate `RTCPeerConnection`.

- [ ] **Step 4: Verify**

Run:

```bash
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
```

Expected:

```text
All pass.
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network/bindings/javascript bindings/javascript/auki-network examples/overwatch crates/auki-network/changelog.md crates/changelog.md examples/changelog.md changelog.md
git commit -m "feat(browser): dial relay-backed circuit managers"
```

## Task 10: Remove Swift Signaled WebRTC Support Targets

**Files:**
- Modify: `crates/auki-network/bindings/swift/Package.swift.tmpl`
- Delete: Swift signaled support files under `crates/auki-network/bindings/swift/Sources/` templates
- Modify: `crates/auki-domain/bindings/swift/Package.swift.tmpl`
- Delete: Swift signaled support files under `crates/auki-domain/bindings/swift/Sources/` templates
- Modify: `examples/ios/AukiCameraStreamer/*`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/auki-domain/changelog.md`
- Modify: `examples/ios/AukiCameraStreamer/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Remove generated Swift signaled targets**

Remove these package products and targets:

```text
AukiNetworkSignaledWebRTC
AukiDomainSignaledWebRTC
```

Remove all references to:

```text
AukiSignaledWebRTCPeer
/auki-webrtc-signaling/
WebRTC.framework
```

- [ ] **Step 2: Regenerate Swift bindings**

Run:

```bash
just generate-swift-bindings auki-network
just generate-swift-bindings auki-domain
```

Expected:

```text
Generated Swift packages contain no SignaledWebRTC targets.
```

- [ ] **Step 3: Update AukiCameraStreamer**

The iOS app config should contain:

```swift
struct RelayConfig: Equatable {
    var discoveryURL: String
    var clusterName: String
    var relayMultiaddrs: [String]
}
```

Startup must call the generated relay-aware Domain bootstrap from Task 8.

- [ ] **Step 4: Verify**

Run:

```bash
swift build --package-path bindings/swift/auki-network
swift build --package-path bindings/swift/auki-domain
xcodebuild test -project examples/ios/AukiCameraStreamer/AukiCameraStreamer.xcodeproj -scheme AukiCameraStreamer -destination 'platform=iOS Simulator,name=iPhone 15,OS=17.5' CODE_SIGNING_ALLOWED=NO
rg "AukiSignaledWebRTCPeer|AukiNetworkSignaledWebRTC|AukiDomainSignaledWebRTC|auki-webrtc-signaling" crates examples bindings
```

Expected:

```text
Build/test commands pass. `rg` returns no production references; changelog history may still mention old names.
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network crates/auki-domain bindings/swift examples/ios crates/changelog.md examples/changelog.md changelog.md
git commit -m "refactor(swift): remove signaled WebRTC support targets"
```

## Task 11: End-to-End Relay Verification

**Files:**
- Modify: `docs/relay-backed-libp2p-transport.md`
- Modify: `examples/relay-smoke/README.md`
- Modify: `examples/ios/AukiCameraStreamer/README.md`
- Modify: `examples/overwatch/README.md`
- Modify: corresponding changelogs

- [ ] **Step 1: Local three-process verification**

Run terminal A:

```bash
cargo run -p auki-relay -- \
  --listen /ip4/127.0.0.1/tcp/9001/ws \
  --advertise /ip4/127.0.0.1/tcp/9001/ws
```

Run terminal B:

```bash
AUKI_RELAY_ADDR=/ip4/127.0.0.1/tcp/9001/ws/p2p/<relay-peer-id> \
  cargo run -p auki-network --features swarm --example relay_native_target_smoke
```

Run terminal C:

```bash
AUKI_RELAY_TARGET_ADDR=/ip4/127.0.0.1/tcp/9001/ws/p2p/<relay-peer-id>/p2p-circuit/p2p/<target-peer-id> \
  node examples/relay-smoke/browser-smoke.mjs
```

Expected:

```text
Browser connects to native target through the standalone relay.
```

- [ ] **Step 2: Public relay verification**

Deploy relay behind TLS:

```text
external: /dns4/relay.auki.network/tcp/443/wss
internal: /ip4/0.0.0.0/tcp/9001/ws
```

Run:

```bash
AUKI_RELAY_ADDR=/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id> \
  cargo run -p auki-network --features swarm --example relay_native_target_smoke

AUKI_RELAY_TARGET_ADDR=/dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>/p2p-circuit/p2p/<target-peer-id> \
  node examples/relay-smoke/browser-smoke.mjs
```

Expected:

```text
Browser connects through public WSS relay.
```

- [ ] **Step 3: Product verification**

Run:

```bash
npm --prefix examples/overwatch run dev
```

On physical iPhone:

```text
Build and run AukiCameraStreamer from Xcode.
Set discovery URL.
Set relay address /dns4/relay.auki.network/tcp/443/wss/p2p/<relay-peer-id>.
Start camera streamer.
```

Expected:

```text
Overwatch joins the same Discovery domain.
Overwatch sees the iOS participant.
Overwatch opens /auki/join/0.0.1 and /auki/stream/0.1.0 through libp2p streams.
Camera preview updates continuously, not just first frame.
Browser console has no WebRTC disconnected/signaling timeout errors.
```

- [ ] **Step 4: Record verification**

Update `docs/relay-backed-libp2p-transport.md` with:

```markdown
## Verification

- Local standalone relay smoke: PASS/FAIL with command output summary.
- Public WSS relay smoke: PASS/FAIL with command output summary.
- Physical iOS to Overwatch camera stream: PASS/FAIL with date, device, relay addr, and observed stream behavior.
```

- [ ] **Step 5: Commit**

```bash
git add docs examples crates changelog.md
git commit -m "docs: record relay-backed transport verification"
```

## Final Acceptance Criteria

- `rg "AukiSignaledWebRTCPeer|AukiNetworkSignaledWebRTC|AukiDomainSignaledWebRTC|auki-webrtc-signaling" crates examples bindings` returns no production references.
- `rg "/p2p-circuit/webrtc" crates examples docs bindings` returns only historical changelog references, or no references if history is intentionally left untouched.
- Browser-generated SDK dials `/wss/.../p2p-circuit/p2p/<manager>`.
- Native iOS/Rust-generated SDK reserves relay circuits and registers `/p2p-circuit/p2p/<manager>` addresses in Discovery.
- Discovery does not carry SDP/ICE.
- Overwatch and AukiCameraStreamer contain no transport-specific signaling code.
- Public WSS relay smoke passes.
- Physical iPhone camera stream updates continuously in Overwatch.
