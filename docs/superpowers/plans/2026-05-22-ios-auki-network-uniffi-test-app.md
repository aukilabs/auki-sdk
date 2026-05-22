# iOS Auki Network UniFFI Test App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an iOS test app that consumes generated Swift bindings from the Rust SDK crates and exchanges one `/auki/message/0.0.1` `MessageEnvelope` with a browser peer.

**Architecture:** Rust crates remain the source of SDK behavior. `auki-network` exposes a message-node facade through UniFFI, generated Swift packages expose that facade to iOS, and the iOS test app imports the generated packages as a host harness. Browser transport stays in the existing generated JavaScript package using js-libp2p.

**Tech Stack:** Rust 2024, UniFFI 0.31, libp2p 0.56, libp2p-stream, WebRTC Direct, Swift 5.9, SwiftProtobuf, Xcode/iOS Simulator, wasm-pack, js-libp2p.

---

## File Structure

- Modify: `scripts/bindings/generate_bindings.py` - add crate-owned native build feature selection for Swift and XCFramework generation.
- Modify: `crates/auki-network/Cargo.toml` - add `webrtc_direct` and `message_node` features; enable Swift binding generation with the needed build features.
- Modify: `crates/auki-network/bindings.toml` - enable Swift and set generator/build feature lists for the message-node surface.
- Modify: `crates/auki-network/src/core.rs` - move the message protocol id constant to an always-available module path.
- Modify: `crates/auki-network/src/message_protocol.rs` - re-export the always-available protocol id and keep protobuf framing behind `swarm`.
- Modify: `crates/auki-network/src/wasm.rs` - export `messageProtocol()` for browser JavaScript.
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl` - export message protocol helpers for browser tests.
- Modify: `crates/auki-network/bindings/javascript/index.d.ts.tmpl` - type the message protocol helper.
- Modify: `crates/auki-network/bindings/javascript/smoke.mjs` - lock the browser-visible message protocol id.
- Modify: `crates/auki-network/src/swarm.rs` - host the shared WebRTC Direct transport helper when `webrtc_direct` is enabled.
- Modify: `crates/auki-network/src/browser_probe.rs` - reuse the WebRTC Direct transport helper from `swarm.rs`.
- Create: `crates/auki-network/src/message_node.rs` - Rust message-node facade used by UniFFI and tested before Swift generation.
- Modify: `crates/auki-network/src/lib.rs` - export `message_node` behind the `message_node` feature.
- Modify: `crates/auki-network/src/ffi.rs` - expose `AukiMessageNode`, `AukiMessageEvent`, and typed errors through UniFFI.
- Modify: `crates/auki-network/src/readme.md` - document the generated Swift test-app surface.
- Modify: `crates/auki-network/src/sprint.md` - add the iOS test-app milestone state.
- Modify: `crates/auki-network/changelog.md`, `crates/changelog.md`, `changelog.md` - propagate crate changes.
- Create: `examples/ios/AukiNetworkTestApp/README.md` - how to generate bindings and run the sample host.
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj/project.pbxproj` - minimal Xcode project for the iOS host.
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/App.swift` - SwiftUI app entry point.
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/ContentView.swift` - simple controls for seed, listen, dial, send, and event log.
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/MessageNodeViewModel.swift` - host-app state that calls generated Swift bindings only.
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestAppTests/GeneratedBindingsSmokeTests.swift` - iOS test target proving generated packages import and construct the node.
- Modify: `examples/changelog.md`, `docs/superpowers/plans/changelog.md`, `docs/superpowers/changelog.md`, `docs/changelog.md`, `changelog.md` - propagate docs/example plan and app changes.

## Task 1: Add Crate-Owned Swift Build Features To The Binding Generator

**Files:**
- Modify: `scripts/bindings/generate_bindings.py`
- Test by command: `python3 scripts/bindings/generate_bindings.py plan swift auki-identity`

- [ ] **Step 1: Add helper functions for cargo feature arguments**

In `scripts/bindings/generate_bindings.py`, add these helpers after `validate_features`:

```python
def binding_build_features(binding_plan: dict) -> list[str]:
    binding_config = binding_plan.get("binding_config", {})
    features = binding_config.get("build_features", [])
    if not isinstance(features, list):
        raise BindingError("build_features must be a list")
    metadata = binding_plan["metadata"]
    validate_features(metadata, features)
    return features


def cargo_build_command(package_name: str, *, target: str | None = None, release: bool = False, features: list[str] | None = None) -> list[str]:
    cmd = ["cargo", "build"]
    if release:
        cmd.append("--release")
    cmd.extend(["-p", package_name])
    if target is not None:
        cmd.extend(["--target", target])
    if features:
        cmd.extend(["--features", ",".join(features)])
    return cmd
```

- [ ] **Step 2: Include binding config in generated plans**

In `plan(...)`, add the section to the returned dictionary:

```python
        "binding_config": section,
```

Expected location: next to `"generator_config": generator_config`.

- [ ] **Step 3: Use build features in `generate_swift`**

Replace:

```python
    run(["cargo", "build", "-p", package_name], cwd=root)
```

with:

```python
    build_features = binding_build_features(binding_plan)
    run(cargo_build_command(package_name, features=build_features), cwd=root)
```

- [ ] **Step 4: Use build features in `generate_swift_xcframework`**

Before the target loop in `generate_swift_xcframework`, add:

```python
    build_features = binding_build_features(binding_plan)
```

Replace:

```python
        run(["cargo", "build", "--release", "-p", package_name, "--target", target], cwd=root)
```

with:

```python
        run(
            cargo_build_command(
                package_name,
                target=target,
                release=True,
                features=build_features,
            ),
            cwd=root,
        )
```

- [ ] **Step 5: Verify existing Swift identity plan still works**

Run:

```bash
python3 scripts/bindings/generate_bindings.py plan swift auki-identity
```

Expected: JSON output includes `"binding_language": "swift"` and exits with status `0`.

- [ ] **Step 6: Verify formatting and status**

Run:

```bash
python3 -m py_compile scripts/bindings/generate_bindings.py
git diff --check
```

Expected: both commands exit with status `0`.

- [ ] **Step 7: Commit**

Run:

```bash
git add scripts/bindings/generate_bindings.py
git commit -m "Support Swift binding build features"
```

## Task 2: Expose `/auki/message/0.0.1` To Browser Helpers Without Pulling In Native Runtime

**Files:**
- Modify: `crates/auki-network/src/core.rs`
- Modify: `crates/auki-network/src/message_protocol.rs`
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-network/bindings/javascript/index.d.ts.tmpl`
- Modify: `crates/auki-network/bindings/javascript/smoke.mjs`
- Test: `cargo test -p auki-network --test surface`
- Test: `cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm`

- [ ] **Step 1: Move the message protocol id into the always-available core module**

In `crates/auki-network/src/core.rs`, add near `PEER_DERIVATION_LABEL`:

```rust
/// libp2p protocol id for generic peer messages.
pub const MESSAGE_PROTOCOL: &str = "/auki/message/0.0.1";
```

- [ ] **Step 2: Re-export the core protocol id from `message_protocol`**

In `crates/auki-network/src/message_protocol.rs`, replace the existing constant with:

```rust
pub use crate::core::MESSAGE_PROTOCOL;
```

Keep the existing `protocol_id_is_locked` test. It should continue to pass.

- [ ] **Step 3: Export `messageProtocol()` from wasm**

In `crates/auki-network/src/wasm.rs`, change the import to:

```rust
use crate::{BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, core};
```

Then add:

```rust
#[wasm_bindgen(js_name = messageProtocol)]
pub fn message_protocol() -> String {
    core::MESSAGE_PROTOCOL.to_string()
}
```

- [ ] **Step 4: Re-export the helper in generated JavaScript**

In `crates/auki-network/bindings/javascript/index.js.tmpl`, add `messageProtocol` to the export list:

```js
  messageProtocol,
```

- [ ] **Step 5: Type the helper**

In `crates/auki-network/bindings/javascript/index.d.ts.tmpl`, add:

```ts
export function messageProtocol(): string;
```

- [ ] **Step 6: Lock the browser-visible protocol id in smoke**

In `crates/auki-network/bindings/javascript/smoke.mjs`, import `messageProtocol` and add:

```js
assert(messageProtocol() === "/auki/message/0.0.1", "message protocol drifted");
```

- [ ] **Step 7: Verify Rust and wasm checks**

Run:

```bash
cargo test -p auki-network --test surface
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
git diff --check
```

Expected: all commands exit with status `0`.

- [ ] **Step 8: Commit**

Run:

```bash
git add crates/auki-network/src/core.rs crates/auki-network/src/message_protocol.rs crates/auki-network/src/wasm.rs crates/auki-network/bindings/javascript/index.js.tmpl crates/auki-network/bindings/javascript/index.d.ts.tmpl crates/auki-network/bindings/javascript/smoke.mjs
git commit -m "Expose message protocol id to browser bindings"
```

## Task 3: Add A Shared WebRTC Direct Transport Helper

**Files:**
- Modify: `crates/auki-network/Cargo.toml`
- Modify: `crates/auki-network/src/swarm.rs`
- Modify: `crates/auki-network/src/browser_probe.rs`
- Test: `cargo test -p auki-network --features browser_probe browser_probe_swarm_uses_sdk_peer_identity`

- [ ] **Step 1: Add feature flags**

In `crates/auki-network/Cargo.toml`, replace:

```toml
browser_probe = ["swarm", "dep:libp2p-webrtc", "dep:rand"]
```

with:

```toml
webrtc_direct = ["swarm", "dep:libp2p-webrtc", "dep:rand"]
browser_probe = ["webrtc_direct"]
message_node = ["webrtc_direct", "tokio/rt-multi-thread"]
```

- [ ] **Step 2: Move the WebRTC Direct transport helper into `swarm.rs`**

In `crates/auki-network/src/swarm.rs`, add under the imports:

```rust
#[cfg(feature = "webrtc_direct")]
use libp2p::{PeerId, core::{Transport as _, muxing::StreamMuxerBox, transport::Boxed}};

#[cfg(feature = "webrtc_direct")]
use rand::thread_rng;

#[cfg(feature = "webrtc_direct")]
use libp2p_webrtc as webrtc;
```

Then add this public helper before `build_swarm`:

```rust
#[cfg(feature = "webrtc_direct")]
pub fn webrtc_direct_transport(
    keypair: &libp2p::identity::Keypair,
) -> Boxed<(PeerId, StreamMuxerBox)> {
    let certificate = webrtc::tokio::Certificate::generate(&mut thread_rng())
        .expect("WebRTC certificate generation should succeed");
    webrtc::tokio::Transport::new(keypair.clone(), certificate)
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed()
}
```

- [ ] **Step 3: Reuse the helper in `browser_probe.rs`**

In `crates/auki-network/src/browser_probe.rs`, remove the local `webrtc_direct_transport` function and replace its use with:

```rust
use crate::swarm::webrtc_direct_transport;
```

- [ ] **Step 4: Verify feature tests**

Run:

```bash
cargo test -p auki-network --features browser_probe browser_probe
git diff --check
```

Expected: all commands exit with status `0`.

- [ ] **Step 5: Commit**

Run:

```bash
git add crates/auki-network/Cargo.toml crates/auki-network/src/swarm.rs crates/auki-network/src/browser_probe.rs
git commit -m "Add WebRTC Direct SDK swarm support"
```

## Task 4: Add The Rust Message Node Facade

**Files:**
- Create: `crates/auki-network/src/message_node.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Test: `cargo test -p auki-network --features message_node message_node`

- [ ] **Step 1: Export the module behind the feature**

In `crates/auki-network/src/lib.rs`, add:

```rust
#[cfg(feature = "message_node")]
pub mod message_node;
```

- [ ] **Step 2: Create the message-node public types**

Create `crates/auki-network/src/message_node.rs` with these public types at the top:

```rust
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
    identify, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
};
use prost::Message as _;
use thiserror::Error;
use tokio::{
    runtime::{Builder, Runtime},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    PeerIdentity,
    message_protocol::{
        MESSAGE_PROTOCOL, MessageAck, MessageEnvelope, MessageProtocolError,
        read_message_envelope, read_message_ack, write_message_envelope, write_message_ack,
    },
    swarm::webrtc_direct_transport,
};

const MESSAGE_NODE_COMMAND_BUFFER: usize = 16;
const MESSAGE_NODE_EVENT_BUFFER: usize = 64;
const MESSAGE_NODE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageNodeConfig {
    pub listen_addresses: Vec<Multiaddr>,
    pub agent_version: String,
}

impl Default for MessageNodeConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec![],
            agent_version: format!("auki-sdk/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageNodeEvent {
    pub peer_id: PeerId,
    pub envelope: MessageEnvelope,
}

#[derive(Debug, Error)]
pub enum MessageNodeError {
    #[error("runtime setup failed: {0}")]
    Runtime(String),
    #[error("swarm setup failed: {0}")]
    Swarm(String),
    #[error("node is stopped")]
    Stopped,
    #[error("command failed: {0}")]
    Command(String),
    #[error("message protocol: {0}")]
    Protocol(#[from] MessageProtocolError),
    #[error("protobuf decode: {0}")]
    Decode(#[source] prost::DecodeError),
    #[error("operation timed out after {0:?}")]
    Timeout(Duration),
}

pub struct MessageNode {
    runtime: Runtime,
    command_tx: mpsc::Sender<MessageNodeCommand>,
    event_rx: Mutex<mpsc::Receiver<MessageNodeEvent>>,
    listen_addrs: Arc<Mutex<Vec<Multiaddr>>>,
    local_peer_id: PeerId,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(NetworkBehaviour)]
struct MessageNodeBehaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    stream: libp2p_stream::Behaviour,
}
```

- [ ] **Step 3: Add command enum and constructor**

In the same file, add:

```rust
enum MessageNodeCommand {
    Dial {
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
        ack: oneshot::Sender<Result<(), MessageNodeError>>,
    },
    SendEnvelope {
        peer_id: PeerId,
        envelope: MessageEnvelope,
        ack: oneshot::Sender<Result<MessageAck, MessageNodeError>>,
    },
    Shutdown,
}

impl MessageNode {
    pub fn spawn(identity: PeerIdentity, config: MessageNodeConfig) -> Result<Self, MessageNodeError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| MessageNodeError::Runtime(err.to_string()))?;

        let local_peer_id = identity.peer_id();
        let listen_addrs = Arc::new(Mutex::new(Vec::new()));
        let (command_tx, command_rx) = mpsc::channel(MESSAGE_NODE_COMMAND_BUFFER);
        let (event_tx, event_rx) = mpsc::channel(MESSAGE_NODE_EVENT_BUFFER);

        let swarm = build_message_node_swarm(&identity, config)
            .map_err(|err| MessageNodeError::Swarm(err.to_string()))?;
        let listen_addrs_for_task = listen_addrs.clone();
        let task = runtime.spawn(run_message_node(
            swarm,
            command_rx,
            event_tx,
            listen_addrs_for_task,
        ));

        Ok(Self {
            runtime,
            command_tx,
            event_rx: Mutex::new(event_rx),
            listen_addrs,
            local_peer_id,
            task: Mutex::new(Some(task)),
        })
    }

    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    pub fn listen_addrs(&self) -> Vec<Multiaddr> {
        self.listen_addrs
            .lock()
            .expect("message node listen addrs mutex poisoned")
            .clone()
    }
}
```

- [ ] **Step 4: Add blocking host-call methods**

Still in `message_node.rs`, add:

```rust
impl MessageNode {
    pub fn dial(&self, peer_id: PeerId, addrs: Vec<Multiaddr>) -> Result<(), MessageNodeError> {
        let (ack, rx) = oneshot::channel();
        self.runtime.block_on(async {
            self.command_tx
                .send(MessageNodeCommand::Dial { peer_id, addrs, ack })
                .await
                .map_err(|_| MessageNodeError::Stopped)?;
            tokio::time::timeout(MESSAGE_NODE_TIMEOUT, rx)
                .await
                .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
                .map_err(|_| MessageNodeError::Stopped)?
        })
    }

    pub fn send_envelope_bytes(
        &self,
        peer_id: PeerId,
        envelope_bytes: Vec<u8>,
    ) -> Result<MessageAck, MessageNodeError> {
        let envelope = MessageEnvelope::decode(&*envelope_bytes).map_err(MessageNodeError::Decode)?;
        let (ack, rx) = oneshot::channel();
        self.runtime.block_on(async {
            self.command_tx
                .send(MessageNodeCommand::SendEnvelope { peer_id, envelope, ack })
                .await
                .map_err(|_| MessageNodeError::Stopped)?;
            tokio::time::timeout(MESSAGE_NODE_TIMEOUT, rx)
                .await
                .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
                .map_err(|_| MessageNodeError::Stopped)?
        })
    }

    pub fn next_event(&self) -> Result<Option<MessageNodeEvent>, MessageNodeError> {
        self.runtime.block_on(async {
            let mut rx = self
                .event_rx
                .lock()
                .expect("message node event receiver mutex poisoned");
            Ok(rx.recv().await)
        })
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.blocking_send(MessageNodeCommand::Shutdown);
        if let Some(task) = self
            .task
            .lock()
            .expect("message node task mutex poisoned")
            .take()
        {
            task.abort();
        }
    }
}
```

- [ ] **Step 5: Add the message-node swarm builder**

Add:

```rust
fn build_message_node_swarm(
    identity: &PeerIdentity,
    config: MessageNodeConfig,
) -> Result<Swarm<MessageNodeBehaviour>, MessageNodeError> {
    let agent_version = config.agent_version;
    let listen_addresses = config.listen_addresses;
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_other_transport(webrtc_direct_transport)
        .map_err(|err| MessageNodeError::Swarm(err.to_string()))?
        .with_behaviour(|key| MessageNodeBehaviour {
            identify: identify::Behaviour::new(
                identify::Config::new("/auki/identify/1.0.0".into(), key.public())
                    .with_agent_version(agent_version),
            ),
            ping: ping::Behaviour::default(),
            stream: libp2p_stream::Behaviour::new(),
        })
        .expect("message node behaviour construction is infallible")
        .build();

    for addr in listen_addresses {
        swarm
            .listen_on(addr.clone())
            .map_err(|err| MessageNodeError::Swarm(format!("listen {addr}: {err}")))?;
    }

    Ok(swarm)
}
```

- [ ] **Step 6: Add the driver task**

Add:

```rust
async fn run_message_node(
    mut swarm: Swarm<MessageNodeBehaviour>,
    mut command_rx: mpsc::Receiver<MessageNodeCommand>,
    event_tx: mpsc::Sender<MessageNodeEvent>,
    listen_addrs: Arc<Mutex<Vec<Multiaddr>>>,
) {
    let mut control = swarm.behaviour().stream.new_control();
    let proto = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_string())
        .expect("MESSAGE_PROTOCOL is a valid libp2p protocol id");
    let mut incoming = match control.accept(proto.clone()) {
        Ok(stream) => stream.boxed(),
        Err(_) => futures::stream::pending().boxed(),
    };

    loop {
        tokio::select! {
            event = swarm.next() => {
                match event {
                    Some(SwarmEvent::NewListenAddr { address, .. }) => {
                        listen_addrs
                            .lock()
                            .expect("message node listen addrs mutex poisoned")
                            .push(address);
                    }
                    Some(_) => {}
                    None => return,
                }
            }
            inbound = incoming.next() => {
                let Some((peer_id, substream)) = inbound else { return; };
                let tx = event_tx.clone();
                tokio::spawn(handle_inbound_message(peer_id, substream, tx));
            }
            command = command_rx.recv() => {
                let Some(command) = command else { return; };
                match command {
                    MessageNodeCommand::Dial { peer_id, addrs, ack } => {
                        let result = dial_message_peer(&mut swarm, peer_id, addrs);
                        let _ = ack.send(result);
                    }
                    MessageNodeCommand::SendEnvelope { peer_id, envelope, ack } => {
                        let mut outbound_control = control.clone();
                        let proto = proto.clone();
                        tokio::spawn(async move {
                            let result = send_outbound_message(peer_id, &mut outbound_control, proto, envelope).await;
                            let _ = ack.send(result);
                        });
                    }
                    MessageNodeCommand::Shutdown => return,
                }
            }
        }
    }
}
```

- [ ] **Step 7: Add inbound, outbound, and dial helpers**

Add:

```rust
fn dial_message_peer(
    swarm: &mut Swarm<MessageNodeBehaviour>,
    peer_id: PeerId,
    addrs: Vec<Multiaddr>,
) -> Result<(), MessageNodeError> {
    if addrs.is_empty() {
        return Err(MessageNodeError::Command("dial requires at least one multiaddr".into()));
    }
    for addr in addrs {
        let dial_addr = if addr.iter().any(|proto| matches!(proto, libp2p::multiaddr::Protocol::P2p(_))) {
            addr
        } else {
            addr.with(libp2p::multiaddr::Protocol::P2p(peer_id))
        };
        swarm
            .dial(dial_addr.clone())
            .map_err(|err| MessageNodeError::Command(format!("dial {dial_addr}: {err}")))?;
    }
    Ok(())
}

async fn handle_inbound_message(
    peer_id: PeerId,
    mut substream: libp2p::Stream,
    event_tx: mpsc::Sender<MessageNodeEvent>,
) {
    let envelope = match read_message_envelope(&mut substream).await {
        Ok(envelope) => envelope,
        Err(err) => {
            eprintln!("auki-network: message from {peer_id}: read failed: {err}");
            return;
        }
    };
    let ack = MessageAck {
        request_id: envelope.request_id.clone(),
        accepted: true,
        detail: "accepted".to_string(),
    };
    let _ = write_message_ack(&mut substream, &ack).await;
    let _ = event_tx.send(MessageNodeEvent { peer_id, envelope }).await;
}

async fn send_outbound_message(
    peer_id: PeerId,
    control: &mut libp2p_stream::Control,
    proto: StreamProtocol,
    envelope: MessageEnvelope,
) -> Result<MessageAck, MessageNodeError> {
    let open = control.open_stream(peer_id, proto);
    let mut substream = tokio::time::timeout(MESSAGE_NODE_TIMEOUT, open)
        .await
        .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
        .map_err(|err| MessageNodeError::Command(err.to_string()))?;
    write_message_envelope(&mut substream, &envelope).await?;
    tokio::time::timeout(MESSAGE_NODE_TIMEOUT, read_message_ack(&mut substream))
        .await
        .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
        .map_err(MessageNodeError::Protocol)
}
```

- [ ] **Step 8: Add Rust unit tests**

Add at the bottom of `message_node.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn envelope_bytes(request_id: &str) -> Vec<u8> {
        let envelope = MessageEnvelope {
            type_url: "auki.test/ping".to_string(),
            body: b"hello".to_vec(),
            request_id: request_id.to_string(),
        };
        envelope.encode_to_vec()
    }

    #[test]
    fn message_node_config_defaults_to_no_listeners() {
        let config = MessageNodeConfig::default();
        assert!(config.listen_addresses.is_empty());
        assert!(config.agent_version.starts_with("auki-sdk/"));
    }

    #[test]
    fn message_node_spawn_uses_identity_peer_id() {
        let identity = PeerIdentity::from_seed(&[71u8; 32]);
        let node = MessageNode::spawn(identity.clone(), MessageNodeConfig::default())
            .expect("message node spawns");
        assert_eq!(node.local_peer_id(), identity.peer_id());
        node.shutdown();
    }

    #[test]
    fn send_envelope_bytes_rejects_bad_protobuf() {
        let identity = PeerIdentity::from_seed(&[72u8; 32]);
        let node = MessageNode::spawn(identity, MessageNodeConfig::default())
            .expect("message node spawns");
        let err = node
            .send_envelope_bytes(PeerIdentity::from_seed(&[73u8; 32]).peer_id(), vec![0xff])
            .expect_err("bad protobuf should fail");
        assert!(matches!(err, MessageNodeError::Decode(_)));
        node.shutdown();
    }
}
```

- [ ] **Step 9: Verify message-node tests**

Run:

```bash
cargo test -p auki-network --features message_node message_node
git diff --check
```

Expected: all commands exit with status `0`.

- [ ] **Step 10: Commit**

Run:

```bash
git add crates/auki-network/src/lib.rs crates/auki-network/src/message_node.rs
git commit -m "Add auki-network message node facade"
```

## Task 5: Expose `AukiMessageNode` Through UniFFI Swift Bindings

**Files:**
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/bindings.toml`
- Test: `cargo test -p auki-network --features message_node --test surface`
- Test: `just generate-swift-bindings auki-network`

- [ ] **Step 1: Enable Swift binding generation for `auki-network`**

In `crates/auki-network/bindings.toml`, replace the Swift section with:

```toml
[bindings.swift]
enabled = true
generator = "uniffi"
template_dir = "bindings/swift"
templates = ["Package.swift.tmpl"]
build_features = ["message_node"]
```

Update `[generators.uniffi]` to:

```toml
[generators.uniffi]
features = ["cli", "message_node"]
bindgen_bin = "uniffi-bindgen"
```

- [ ] **Step 2: Add UniFFI records and error cases**

In `crates/auki-network/src/ffi.rs`, add:

```rust
#[cfg(feature = "message_node")]
use crate::message_node::{MessageNode, MessageNodeConfig};

#[cfg(feature = "message_node")]
use libp2p_identity::PeerId;

#[cfg(feature = "message_node")]
use multiaddr::Multiaddr;

#[cfg(feature = "message_node")]
use prost::Message as _;
```

Extend `NetworkError` with:

```rust
    #[error("invalid peer id: {value}")]
    InvalidPeerId { value: String },
    #[error("invalid multiaddr: {value}")]
    InvalidMultiaddr { value: String },
    #[error("message node: {message}")]
    MessageNode { message: String },
```

Add:

```rust
#[cfg(feature = "message_node")]
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct AukiMessageEvent {
    pub peer_id: String,
    pub envelope: Vec<u8>,
}
```

- [ ] **Step 3: Add the UniFFI object**

Add to `ffi.rs`:

```rust
#[cfg(feature = "message_node")]
#[derive(uniffi::Object)]
pub struct AukiMessageNode {
    inner: MessageNode,
}

#[cfg(feature = "message_node")]
#[uniffi::export]
impl AukiMessageNode {
    #[uniffi::constructor]
    pub fn from_wallet_seed(
        seed: Vec<u8>,
        listen_addrs: Vec<String>,
        agent_version: String,
    ) -> Result<Arc<Self>, NetworkError> {
        let wallet = Wallet::from_seed(&seed32(seed)?);
        let identity = core::PeerIdentity::from_wallet(&wallet);
        let listen_addresses = parse_multiaddrs(listen_addrs)?;
        let inner = MessageNode::spawn(
            identity,
            MessageNodeConfig {
                listen_addresses,
                agent_version,
            },
        )
        .map_err(network_error)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn peer_id(&self) -> String {
        self.inner.local_peer_id().to_string()
    }

    pub fn listen_addrs(&self) -> Vec<String> {
        self.inner
            .listen_addrs()
            .into_iter()
            .map(|addr| addr.to_string())
            .collect()
    }

    pub fn dial(&self, peer_id: String, addrs: Vec<String>) -> Result<(), NetworkError> {
        self.inner
            .dial(parse_peer_id(&peer_id)?, parse_multiaddrs(addrs)?)
            .map_err(network_error)
    }

    pub fn send_message_envelope_bytes(
        &self,
        peer_id: String,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, NetworkError> {
        let ack = self
            .inner
            .send_envelope_bytes(parse_peer_id(&peer_id)?, envelope)
            .map_err(network_error)?;
        Ok(ack.encode_to_vec())
    }

    pub fn next_event(&self) -> Result<Option<AukiMessageEvent>, NetworkError> {
        let Some(event) = self.inner.next_event().map_err(network_error)? else {
            return Ok(None);
        };
        Ok(Some(AukiMessageEvent {
            peer_id: event.peer_id.to_string(),
            envelope: event.envelope.encode_to_vec(),
        }))
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}
```

- [ ] **Step 4: Add parse helpers**

Add to `ffi.rs`:

```rust
#[cfg(feature = "message_node")]
fn parse_peer_id(value: &str) -> Result<PeerId, NetworkError> {
    value
        .parse()
        .map_err(|_| NetworkError::InvalidPeerId {
            value: value.to_string(),
        })
}

#[cfg(feature = "message_node")]
fn parse_multiaddrs(values: Vec<String>) -> Result<Vec<Multiaddr>, NetworkError> {
    values
        .into_iter()
        .map(|value| {
            value
                .parse()
                .map_err(|_| NetworkError::InvalidMultiaddr { value })
        })
        .collect()
}

#[cfg(feature = "message_node")]
fn network_error(error: crate::message_node::MessageNodeError) -> NetworkError {
    NetworkError::MessageNode {
        message: error.to_string(),
    }
}
```

- [ ] **Step 5: Verify Rust tests and Swift generation**

Run:

```bash
cargo test -p auki-network --features message_node --test surface
just generate-swift-bindings auki-network
git diff --check
```

Expected: tests pass, `bindings/swift/auki-network/` is generated, and diff check passes.

- [ ] **Step 6: Commit**

Run:

```bash
git add crates/auki-network/src/ffi.rs crates/auki-network/bindings.toml bindings/swift/auki-network
git commit -m "Expose auki-network message node to Swift"
```

## Task 6: Generate Swift Protobuf And Verify Local Swift Packages

**Files:**
- Modify: `bindings/swift/README.md`
- Generated but ignored: `bindings/swift/auki-proto/`
- Test: `just generate-swift-proto`
- Test: `swift package describe --package-path bindings/swift/auki-proto`

- [ ] **Step 1: Generate Swift protobuf bindings**

Run:

```bash
just generate-swift-proto
```

Expected: `bindings/swift/auki-proto/Package.swift` exists locally. If the command fails with `protoc-gen-swift is required`, install SwiftProtobuf's protoc plugin and rerun the same command.

- [ ] **Step 2: Verify the generated SwiftProtobuf package**

Run:

```bash
swift package describe --package-path bindings/swift/auki-proto
```

Expected: output includes `Name: auki-proto` and product `AukiProto`.

- [ ] **Step 3: Document the required local generation order**

In `bindings/swift/README.md`, replace the body with:

```markdown
# Swift Bindings

Swift-facing SDK packages live here.

Generated UniFFI packages are committed when they are crate-owned Swift SDK surfaces, such as `auki-identity` and `auki-network`.

Generated protobuf packages are local data-conversion artifacts and are ignored by git. Run:

```bash
just generate-swift-proto
```

This creates `bindings/swift/auki-proto/`, a SwiftProtobuf package named `AukiProto`.

For the iOS network test app, generate packages in this order:

```bash
just generate-swift-proto
just generate-swift-bindings auki-identity
just generate-swift-bindings auki-network
```
```

- [ ] **Step 4: Verify ignored proto output is not staged**

Run:

```bash
git status --short bindings/swift
```

Expected: generated `bindings/swift/auki-proto/` is not listed as an untracked directory.

- [ ] **Step 5: Commit**

Run:

```bash
git add bindings/swift/README.md
git commit -m "Document Swift binding generation order"
```

## Task 7: Add The iOS Test App Host

**Files:**
- Create: `examples/ios/AukiNetworkTestApp/README.md`
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj/project.pbxproj`
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/App.swift`
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/ContentView.swift`
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/MessageNodeViewModel.swift`
- Create: `examples/ios/AukiNetworkTestApp/AukiNetworkTestAppTests/GeneratedBindingsSmokeTests.swift`
- Modify: `examples/changelog.md`

- [ ] **Step 1: Create app folders**

Run:

```bash
mkdir -p examples/ios/AukiNetworkTestApp/AukiNetworkTestApp
mkdir -p examples/ios/AukiNetworkTestApp/AukiNetworkTestAppTests
mkdir -p examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj
```

- [ ] **Step 2: Add the app README**

Create `examples/ios/AukiNetworkTestApp/README.md`:

```markdown
# AukiNetworkTestApp

Minimal iOS host app for generated Auki Swift bindings.

The app imports generated SDK packages:

- `auki_identity`
- `auki_network`
- `AukiProto`

Before opening the project, generate local bindings from the repository root:

```bash
just generate-swift-proto
just generate-swift-bindings auki-identity
just generate-swift-bindings auki-network
```

Then build the app:

```bash
xcodebuild \
  -project examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj \
  -scheme AukiNetworkTestApp \
  -destination 'generic/platform=iOS Simulator' \
  build
```

This app is a host/test harness only. SDK networking behavior lives in Rust crates and generated Swift bindings.
```

- [ ] **Step 3: Add SwiftUI entry point**

Create `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/App.swift`:

```swift
import SwiftUI

@main
struct AukiNetworkTestApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
```

- [ ] **Step 4: Add the view model**

Create `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/MessageNodeViewModel.swift`:

```swift
import Foundation
import AukiProto
import auki_identity
import auki_network

@MainActor
final class MessageNodeViewModel: ObservableObject {
    @Published var walletSeedHex = String(repeating: "03", count: 32)
    @Published var peerId = ""
    @Published var listenAddrs = ""
    @Published var browserPeerId = ""
    @Published var browserAddrs = ""
    @Published var eventLog = ""

    private var node: AukiMessageNode?

    func start() {
        do {
            let seed = try Self.hexToData(walletSeedHex)
            let wallet = try Wallet.fromSeed(seed: seed)
            let node = try AukiMessageNode.fromWalletSeed(
                seed: try wallet.seed(),
                listenAddrs: ["/ip4/0.0.0.0/udp/0/webrtc-direct"],
                agentVersion: "auki-ios-test-app/0.1"
            )
            self.node = node
            peerId = node.peerId()
            listenAddrs = node.listenAddrs().joined(separator: "\n")
            append("started \(peerId)")
        } catch {
            append("start failed: \(error)")
        }
    }

    func refreshListenAddrs() {
        guard let node else {
            append("node is not started")
            return
        }
        listenAddrs = node.listenAddrs().joined(separator: "\n")
    }

    func dialBrowser() {
        guard let node else {
            append("node is not started")
            return
        }
        let addrs = browserAddrs
            .split(whereSeparator: \.isNewline)
            .map(String.init)
        do {
            try node.dial(peerId: browserPeerId, addrs: addrs)
            append("dial requested \(browserPeerId)")
        } catch {
            append("dial failed: \(error)")
        }
    }

    func sendPing() {
        guard let node else {
            append("node is not started")
            return
        }
        do {
            var envelope = Auki_Message_MessageEnvelope()
            envelope.typeURL = "auki.test/ping"
            envelope.body = Data("hello from ios".utf8)
            envelope.requestID = UUID().uuidString
            let ackBytes = try node.sendMessageEnvelopeBytes(
                peerId: browserPeerId,
                envelope: try envelope.serializedData()
            )
            let ack = try Auki_Message_MessageAck(serializedData: ackBytes)
            append("ack \(ack.requestID) accepted=\(ack.accepted) \(ack.detail)")
        } catch {
            append("send failed: \(error)")
        }
    }

    func pollEvent() {
        guard let node else {
            append("node is not started")
            return
        }
        do {
            guard let event = try node.nextEvent() else {
                append("no event")
                return
            }
            let envelope = try Auki_Message_MessageEnvelope(serializedData: event.envelope)
            append("message from \(event.peerID): \(envelope.typeURL) \(envelope.requestID)")
        } catch {
            append("poll failed: \(error)")
        }
    }

    func stop() {
        node?.shutdown()
        node = nil
        append("stopped")
    }

    private func append(_ line: String) {
        eventLog = eventLog.isEmpty ? line : "\(eventLog)\n\(line)"
    }

    private static func hexToData(_ hex: String) throws -> Data {
        let cleaned = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        guard cleaned.count % 2 == 0 else {
            throw HexError.invalidLength
        }
        var data = Data()
        var index = cleaned.startIndex
        while index < cleaned.endIndex {
            let next = cleaned.index(index, offsetBy: 2)
            guard let byte = UInt8(cleaned[index..<next], radix: 16) else {
                throw HexError.invalidByte
            }
            data.append(byte)
            index = next
        }
        return data
    }

    enum HexError: Error {
        case invalidLength
        case invalidByte
    }
}
```

- [ ] **Step 5: Add the SwiftUI view**

Create `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp/ContentView.swift`:

```swift
import SwiftUI

struct ContentView: View {
    @StateObject private var model = MessageNodeViewModel()

    var body: some View {
        NavigationStack {
            Form {
                Section("Local") {
                    TextField("Wallet seed hex", text: $model.walletSeedHex)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Start node") { model.start() }
                    Button("Refresh listen addresses") { model.refreshListenAddrs() }
                    Button("Stop node") { model.stop() }
                    Text(model.peerId).font(.footnote).textSelection(.enabled)
                    Text(model.listenAddrs).font(.footnote).textSelection(.enabled)
                }

                Section("Browser peer") {
                    TextField("Browser peer id", text: $model.browserPeerId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextEditor(text: $model.browserAddrs)
                        .frame(minHeight: 80)
                    Button("Dial browser") { model.dialBrowser() }
                    Button("Send ping") { model.sendPing() }
                    Button("Poll event") { model.pollEvent() }
                }

                Section("Log") {
                    Text(model.eventLog)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                }
            }
            .navigationTitle("Auki Network")
        }
    }
}
```

- [ ] **Step 6: Add generated binding smoke tests**

Create `examples/ios/AukiNetworkTestApp/AukiNetworkTestAppTests/GeneratedBindingsSmokeTests.swift`:

```swift
import XCTest
import AukiProto
import auki_identity
import auki_network

final class GeneratedBindingsSmokeTests: XCTestCase {
    func testGeneratedBindingsConstructMessageNode() throws {
        let seed = Data(repeating: 3, count: 32)
        let wallet = try Wallet.fromSeed(seed: seed)
        let node = try AukiMessageNode.fromWalletSeed(
            seed: try wallet.seed(),
            listenAddrs: [],
            agentVersion: "auki-ios-test-app-tests/0.1"
        )
        XCTAssertFalse(node.peerId().isEmpty)
        node.shutdown()
    }

    func testGeneratedProtoSerializesEnvelope() throws {
        var envelope = Auki_Message_MessageEnvelope()
        envelope.typeURL = "auki.test/ping"
        envelope.body = Data([1, 2, 3])
        envelope.requestID = "req-1"
        let bytes = try envelope.serializedData()
        let decoded = try Auki_Message_MessageEnvelope(serializedData: bytes)
        XCTAssertEqual(decoded.requestID, "req-1")
    }
}
```

- [ ] **Step 7: Add a minimal Xcode project**

Create `examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj/project.pbxproj` as a deterministic minimal project with:

- one iOS app target named `AukiNetworkTestApp`;
- one iOS unit test target named `AukiNetworkTestAppTests`;
- local package references to:
  - `../../../bindings/swift/auki-identity`;
  - `../../../bindings/swift/auki-network`;
  - `../../../bindings/swift/auki-proto`;
- app sources:
  - `AukiNetworkTestApp/App.swift`;
  - `AukiNetworkTestApp/ContentView.swift`;
  - `AukiNetworkTestApp/MessageNodeViewModel.swift`;
- test source:
  - `AukiNetworkTestAppTests/GeneratedBindingsSmokeTests.swift`.

Use `examples/ios/AukiNetworkTestApp/README.md` as the authority for build commands. Keep the project file stable by sorting file references alphabetically inside each group.

- [ ] **Step 8: Verify app build**

Run:

```bash
just generate-swift-proto
just generate-swift-bindings auki-network
xcodebuild \
  -project examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj \
  -scheme AukiNetworkTestApp \
  -destination 'generic/platform=iOS Simulator' \
  build
git diff --check
```

Expected: build succeeds and diff check passes.

- [ ] **Step 9: Update examples changelog**

Prepend to `examples/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

Added the iOS Auki network test app scaffold. The app imports generated Swift bindings from `auki-identity`, `auki-network`, and generated SwiftProtobuf `AukiProto`; SDK networking behavior remains in Rust crates.
```

- [ ] **Step 10: Commit**

Run:

```bash
git add examples/ios/AukiNetworkTestApp examples/changelog.md
git commit -m "Add iOS Auki network test app"
```

## Task 8: Add Browser-To-iOS Message Interop Harness

**Files:**
- Create: `examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs`
- Modify: `examples/ios/AukiNetworkTestApp/README.md`
- Test by command: `node examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs --help`

- [ ] **Step 1: Add browser smoke script**

Create `examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs`:

```js
#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import initAukiNetwork, {
  createAukiNetworkPeer,
  messageProtocol,
} from "../../../bindings/javascript/auki-network/index.js";

function usage() {
  console.log("Usage: node browser-message-smoke.mjs <ios-multiaddr>");
}

const target = process.argv[2];
if (target == null || target === "--help") {
  usage();
  process.exit(target === "--help" ? 0 : 1);
}

const wasmBytes = await readFile(new URL("../../../bindings/javascript/auki-network/auki_network_bg.wasm", import.meta.url));
await initAukiNetwork({ module_or_path: wasmBytes });

const walletSeed = new Uint8Array(32);
walletSeed.fill(4);
const peer = await createAukiNetworkPeer({ walletSeed });

const stream = await peer.dialProtocol(target, messageProtocol());
const requestId = `browser-${Date.now()}`;
const payload = encodeEnvelope({
  typeUrl: "auki.test/ping",
  body: new TextEncoder().encode("hello from browser"),
  requestId,
});

const len = new Uint8Array(4);
new DataView(len.buffer).setUint32(0, payload.length, false);
await stream.sink([len, payload]);

console.log(`sent ${requestId} to ${target}`);
await peer.stop();

function encodeEnvelope({ typeUrl, body, requestId }) {
  return concat([
    fieldBytes(1, new TextEncoder().encode(typeUrl)),
    fieldBytes(2, body),
    fieldBytes(3, new TextEncoder().encode(requestId)),
  ]);
}

function fieldBytes(fieldNo, bytes) {
  return concat([
    varint((fieldNo << 3) | 2),
    varint(bytes.length),
    bytes,
  ]);
}

function varint(value) {
  const out = [];
  let n = value >>> 0;
  while (n >= 0x80) {
    out.push((n & 0x7f) | 0x80);
    n >>>= 7;
  }
  out.push(n);
  return Uint8Array.from(out);
}

function concat(chunks) {
  const len = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(len);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}
```

This script verifies browser dialing and write-side framing against the iOS app's native message node.

- [ ] **Step 2: Document the interop flow**

Append to `examples/ios/AukiNetworkTestApp/README.md`:

```markdown
## Browser message smoke

Generate browser bindings:

```bash
just generate-javascript-bindings auki-network
```

Run the iOS app, tap **Start node**, then copy one full listen address including `/p2p/<peer-id>`.

From the repository root:

```bash
node examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs '<ios-multiaddr>'
```
```

- [ ] **Step 3: Verify script help**

Run:

```bash
node examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs --help
git diff --check
```

Expected: usage text prints and diff check passes.

- [ ] **Step 4: Commit**

Run:

```bash
git add examples/ios/AukiNetworkTestApp/browser-message-smoke.mjs examples/ios/AukiNetworkTestApp/README.md
git commit -m "Add browser message smoke for iOS test app"
```

## Task 9: Final Documentation And Verification

**Files:**
- Modify: `crates/auki-network/src/readme.md`
- Modify: `crates/auki-network/src/sprint.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `examples/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update `auki-network` source README**

In `crates/auki-network/src/readme.md`, add `message_node.rs` to the file list:

```markdown
- [`message_node.rs`](message_node.rs) - native message-node facade for generated Swift bindings and host-app smoke tests.
```

Add to the public surface section:

```rust
#[cfg(feature = "message_node")]
pub mod message_node;
```

- [ ] **Step 2: Update sprint status**

In `crates/auki-network/src/sprint.md`, add under "Now":

```markdown
- **iOS message-node binding, `message_node` feature.** The crate exposes a native message-node facade for generated Swift bindings. iOS test apps consume generated Swift packages; SDK networking behavior stays in Rust.
```

- [ ] **Step 3: Update crate changelogs**

Prepend to `crates/auki-network/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

Added the `message_node` feature for generated Swift bindings: WebRTC Direct-capable Rust message node, UniFFI `AukiMessageNode` surface, and `/auki/message/0.0.1` browser interop helpers. Swift consumes generated bindings; it does not implement libp2p.
```

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

`auki-network` gained the native Swift message-node binding surface for the iOS test app, keeping libp2p in Rust and exposing `/auki/message/0.0.1` through UniFFI.
```

- [ ] **Step 4: Update root changelog**

Prepend to `changelog.md`:

```markdown
### Nils's codex · May 22, HKT, 2026

**iOS Auki network test app path implemented.** `auki-network` now exposes a generated Swift message-node surface over Rust libp2p, the SDK includes an iOS test host consuming generated Swift packages, and browser interop uses `/auki/message/0.0.1`.
```

- [ ] **Step 5: Run final verification**

Run:

```bash
cargo test -p auki-network --features message_node
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-network
just generate-swift-proto
just generate-swift-bindings auki-network
xcodebuild \
  -project examples/ios/AukiNetworkTestApp/AukiNetworkTestApp.xcodeproj \
  -scheme AukiNetworkTestApp \
  -destination 'generic/platform=iOS Simulator' \
  build
git diff --check
```

Expected: all commands exit with status `0`.

- [ ] **Step 6: Commit**

Run:

```bash
git add crates/auki-network/src/readme.md crates/auki-network/src/sprint.md crates/auki-network/changelog.md crates/changelog.md examples/changelog.md changelog.md
git commit -m "Document iOS message node binding path"
```

## Self-Review Checklist

- Spec coverage:
  - No swift-libp2p: covered by Tasks 3, 4, 5, and 7.
  - iOS test app consumes generated Swift packages: covered by Tasks 6 and 7.
  - Rust `auki-network` owns transport/runtime: covered by Tasks 3, 4, and 5.
  - SwiftProtobuf owns data conversion only: covered by Tasks 2, 6, and 7.
  - Browser-to-iOS `/auki/message/0.0.1`: covered by Tasks 2, 4, and 8.
  - `auki-domain` excluded from first milestone: no task touches `auki-domain`.
- Placeholder scan: the plan has no empty implementation or test steps.
- Type consistency:
  - Rust facade is `MessageNode`.
  - UniFFI object is `AukiMessageNode`.
  - Swift event record is `AukiMessageEvent`.
  - Protocol id is `MESSAGE_PROTOCOL` in Rust and `messageProtocol()` in JavaScript.
