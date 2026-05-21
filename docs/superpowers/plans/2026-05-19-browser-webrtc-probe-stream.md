# Browser WebRTC Probe Stream Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove a browser wasm Auki peer can dial a native SDK peer over WebRTC Direct and exchange one SDK-owned named probe protocol message.

**Architecture:** Keep `auki-domain-browser` fail-closed while this low-level proof runs. Add a native-only `auki-network` probe feature that listens on WebRTC Direct and serves `/auki/browser-probe/0.0.1` using libp2p request-response JSON. Extend `auki-network-browser-wasm` with a wasm `dialBrowserProbe(...)` export that derives the canonical PeerId from the supplied seed, builds a browser libp2p swarm with `webrtc-websys`, dials the native multiaddr, sends the probe request, and returns UI-friendly success/error details.

**Tech Stack:** Rust 2024, `auki-network`, `auki-network-browser-wasm`, rust-libp2p 0.56, `libp2p-webrtc` 0.9.0-alpha.1 native WebRTC Direct, `libp2p-webrtc-websys` browser transport, request-response JSON, `wasm-bindgen`, `wasm-pack`, Playwright-driven browser smoke test.

---

## File Structure

- `crates/auki-network/Cargo.toml` — add the native `browser_probe` feature and optional native WebRTC dependencies.
- `crates/auki-network/src/browser_probe_protocol.rs` — shared probe protocol id, request/response structs, and unit tests.
- `crates/auki-network/src/browser_probe.rs` — native WebRTC Direct request-response swarm builder and listener runner, compiled only with `browser_probe`.
- `crates/auki-network/examples/browser_probe_listener.rs` — command-line native probe listener that prints the dialable `/webrtc-direct/.../p2p/<peer>` multiaddr.
- `crates/auki-network/src/lib.rs` — export the probe protocol types and gate the native probe module.
- `crates/auki-network/src/README.md` and `crates/auki-network/src/sprint.md` — current implementation status and next steps.
- `crates/auki-network/changelog.md`, `crates/changelog.md`, `changelog.md` — changelog propagation.
- `crates/auki-network-browser-wasm/Cargo.toml` — add wasm-side async/serde dependencies under `browser_libp2p`.
- `crates/auki-network-browser-wasm/src/lib.rs` — add `dialBrowserProbe(seed, address, payload)` and its result/error structs.
- `crates/auki-network-browser-wasm/scripts/browser_probe_smoke.html` — browser harness that loads `pkg-web` and calls `dialBrowserProbe`.
- `crates/auki-network-browser-wasm/scripts/smoke_browser_probe.mjs` — starts a local static server, launches Chromium with Playwright, and verifies the browser result.
- `crates/auki-network-browser-wasm/src/README.md`, `src/sprint.md`, `changelog.md` — status/changelog updates for the wasm dialer.

This plan intentionally stops at one probe protocol. Domain join, participant metadata, sensor catalogs, media presence, and audio move only after this proof shows a browser peer can open an SDK-owned stream to a native SDK peer.

---

### Task 1: Shared Probe Protocol and Native Feature Gate

**Files:**
- Modify: `crates/auki-network/Cargo.toml`
- Create: `crates/auki-network/src/browser_probe_protocol.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/README.md`
- Modify: `crates/auki-network/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write the failing protocol unit test**

Create `crates/auki-network/src/browser_probe_protocol.rs` with the test first:

```rust
use serde::{Deserialize, Serialize};

pub const BROWSER_PROBE_PROTOCOL: &str = "/auki/browser-probe/0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserProbeRequest {
    pub nonce: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserProbeResponse {
    pub nonce: String,
    pub payload: Vec<u8>,
    pub responder: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_preserves_nonce_payload_and_names_responder() {
        let request = BrowserProbeRequest {
            nonce: "probe-001".to_string(),
            payload: vec![1, 2, 3, 4],
        };

        let response = BrowserProbeResponse::from_request(&request, "native-probe");

        assert_eq!(response.nonce, "probe-001");
        assert_eq!(response.payload, vec![1, 2, 3, 4]);
        assert_eq!(response.responder, "native-probe");
    }
}
```

- [ ] **Step 2: Run the failing protocol test**

Run:

```bash
cargo test -p auki-network browser_probe_protocol::tests::response_preserves_nonce_payload_and_names_responder
```

Expected: compile failure because `BrowserProbeResponse::from_request` does not exist.

- [ ] **Step 3: Implement the protocol helper and exports**

Add this impl above the test module in `crates/auki-network/src/browser_probe_protocol.rs`:

```rust
impl BrowserProbeResponse {
    pub fn from_request(request: &BrowserProbeRequest, responder: impl Into<String>) -> Self {
        Self {
            nonce: request.nonce.clone(),
            payload: request.payload.clone(),
            responder: responder.into(),
        }
    }
}
```

Add exports to `crates/auki-network/src/lib.rs`:

```rust
pub mod browser_probe_protocol;
pub use browser_probe_protocol::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse,
};

#[cfg(feature = "browser_probe")]
pub mod browser_probe;
```

- [ ] **Step 4: Add the native probe feature gate**

Modify `crates/auki-network/Cargo.toml`:

```toml
browser_probe = [
    "swarm",
    "dep:libp2p-webrtc",
    "dep:rand",
]

libp2p-webrtc = { version = "0.9.0-alpha.1", default-features = false, features = ["tokio"], optional = true }
rand = { version = "0.8", optional = true }
```

Keep the existing `swarm` feature unchanged. The new `browser_probe` feature is a proof surface, not the default native runtime.

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test -p auki-network browser_probe_protocol::tests::response_preserves_nonce_payload_and_names_responder
cargo check -p auki-network --features browser_probe
```

Expected: both commands pass.

Update `crates/auki-network/src/README.md` to mention the shared `/auki/browser-probe/0.0.1` protocol types and native `browser_probe` feature. Prepend changelog entries at the leaf, `crates/changelog.md`, and root.

Commit:

```bash
git add Cargo.toml Cargo.lock crates/auki-network crates/changelog.md changelog.md
git commit -m "feat: add browser probe protocol"
```

---

### Task 2: Native WebRTC Direct Probe Listener

**Files:**
- Create: `crates/auki-network/src/browser_probe.rs`
- Create: `crates/auki-network/examples/browser_probe_listener.rs`
- Modify: `crates/auki-network/src/README.md`
- Modify: `crates/auki-network/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Write the failing native listener compile test**

Create `crates/auki-network/src/browser_probe.rs`:

```rust
use crate::{BrowserProbeRequest, BrowserProbeResponse, PeerIdentity};

pub fn responder_label(identity: &PeerIdentity) -> String {
    format!("native:{}", identity.peer_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responder_label_uses_native_peer_id() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        assert_eq!(
            responder_label(&identity),
            "native:12D3KooWJtD2C3gKUMYVEm5mgsYQC3dW7QWSiQVPbR64mFZaoj2Q"
        );
    }

    #[test]
    fn response_uses_native_responder_label() {
        let identity = PeerIdentity::from_seed(&[41u8; 32]);
        let request = BrowserProbeRequest {
            nonce: "n".to_string(),
            payload: vec![9],
        };
        let response = BrowserProbeResponse::from_request(&request, responder_label(&identity));
        assert_eq!(response.responder, responder_label(&identity));
    }
}
```

Run:

```bash
cargo test -p auki-network --features browser_probe browser_probe::tests::responder_label_uses_native_peer_id
```

Expected: if the hard-coded PeerId differs, replace the expected string with the actual SDK output from this failing assertion, then rerun and require it to pass. Do not derive the expected value outside `PeerIdentity`.

- [ ] **Step 2: Implement the native WebRTC swarm builder**

Extend `crates/auki-network/src/browser_probe.rs` with the request-response behaviour and builder:

```rust
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
    core::{muxing::StreamMuxerBox, transport::Boxed, Transport as _},
    request_response::{self, ProtocolSupport, json},
    swarm::{NetworkBehaviour, SwarmEvent},
};
use libp2p_webrtc as webrtc;
use rand::thread_rng;
use thiserror::Error;

use crate::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, PeerIdentity,
};

#[derive(NetworkBehaviour)]
pub struct BrowserProbeBehaviour {
    pub probe: json::Behaviour<BrowserProbeRequest, BrowserProbeResponse>,
}

#[derive(Debug, Error)]
pub enum BrowserProbeError {
    #[error("webrtc certificate generation failed: {0}")]
    Certificate(String),
    #[error("transport setup failed: {0}")]
    Transport(String),
    #[error("listen failed for {addr}: {source}")]
    Listen {
        addr: Multiaddr,
        source: libp2p::TransportError<std::io::Error>,
    },
    #[error("listener did not produce a dialable address within {0:?}")]
    ListenTimeout(Duration),
}

pub fn webrtc_direct_transport(
    keypair: &libp2p::identity::Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, BrowserProbeError> {
    let certificate = webrtc::tokio::Certificate::generate(&mut thread_rng())
        .map_err(|err| BrowserProbeError::Certificate(err.to_string()))?;
    Ok(webrtc::tokio::Transport::new(keypair.clone(), certificate)
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .boxed())
}

pub fn build_browser_probe_swarm(
    identity: &PeerIdentity,
) -> Result<Swarm<BrowserProbeBehaviour>, BrowserProbeError> {
    SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_other_transport(webrtc_direct_transport)
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))?
        .with_behaviour(|_| BrowserProbeBehaviour {
            probe: json::Behaviour::new(
                [(
                    StreamProtocol::new(BROWSER_PROBE_PROTOCOL),
                    ProtocolSupport::Full,
                )],
                request_response::Config::default(),
            ),
        })
        .map_err(|err| BrowserProbeError::Transport(err.to_string()))
        .map(|builder| builder.build())
}
```

- [ ] **Step 3: Add the listener runner**

Add:

```rust
pub async fn listen_and_serve(
    identity: PeerIdentity,
    listen_addr: Multiaddr,
) -> Result<(), BrowserProbeError> {
    let mut swarm = build_browser_probe_swarm(&identity)?;
    swarm
        .listen_on(listen_addr.clone())
        .map_err(|source| BrowserProbeError::Listen {
            addr: listen_addr,
            source,
        })?;

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::Behaviour(BrowserProbeBehaviourEvent::Probe(
                request_response::Event::Message {
                    message:
                        request_response::Message::Request {
                            request, channel, ..
                        },
                    ..
                },
            )) => {
                let response =
                    BrowserProbeResponse::from_request(&request, responder_label(&identity));
                let _ = swarm.behaviour_mut().probe.send_response(channel, response);
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("PARK_BROWSER_PROBE_ADDR={address}/p2p/{}", identity.peer_id());
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Add the CLI example**

Create `crates/auki-network/examples/browser_probe_listener.rs`:

```rust
use auki_network::{PeerIdentity, browser_probe};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = [41u8; 32];
    let identity = PeerIdentity::from_seed(&seed);
    let listen_addr = "/ip4/0.0.0.0/udp/0/webrtc-direct".parse()?;

    eprintln!("peer_id={}", identity.peer_id());
    browser_probe::listen_and_serve(identity, listen_addr).await?;
    Ok(())
}
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo check -p auki-network --features browser_probe --example browser_probe_listener
cargo test -p auki-network --features browser_probe browser_probe
```

Expected: both commands pass. Then run the listener manually:

```bash
cargo run -p auki-network --features browser_probe --example browser_probe_listener
```

Expected: stdout eventually contains `PARK_BROWSER_PROBE_ADDR=/ip4/.../udp/.../webrtc-direct/certhash/.../p2p/<peer-id>`.

Update docs/changelogs. Commit:

```bash
git add crates/auki-network crates/changelog.md changelog.md
git commit -m "feat: add native browser probe listener"
```

---

### Task 3: Browser WASM Dial Export

**Files:**
- Modify: `crates/auki-network-browser-wasm/Cargo.toml`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Add a failing API test for result shaping**

Add this native unit test to `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(test)]
mod browser_probe_result_tests {
    use super::*;

    #[test]
    fn browser_probe_result_carries_peer_protocol_and_payload() {
        let result = BrowserProbeResult::ok(
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
            "/auki/browser-probe/0.0.1",
            vec![1, 2, 3],
        );

        assert!(result.ok);
        assert_eq!(result.local_peer_id, "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar");
        assert_eq!(result.protocol, "/auki/browser-probe/0.0.1");
        assert_eq!(result.payload, vec![1, 2, 3]);
        assert!(result.error.is_none());
    }
}
```

Run:

```bash
cargo test -p auki-network-browser-wasm browser_probe_result_tests::browser_probe_result_carries_peer_protocol_and_payload
```

Expected: compile failure because `BrowserProbeResult` does not exist.

- [ ] **Step 2: Add wasm-side dependencies**

Modify `crates/auki-network-browser-wasm/Cargo.toml`:

```toml
browser_libp2p = [
    "dep:futures",
    "dep:libp2p",
    "dep:serde",
    "dep:serde-wasm-bindgen",
    "dep:wasm-bindgen-futures",
]

futures = { version = "0.3", default-features = false, optional = true }
serde = { version = "1", features = ["derive"], optional = true }
serde-wasm-bindgen = { version = "0.6", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
```

- [ ] **Step 3: Implement result shaping**

Add:

```rust
#[cfg_attr(feature = "browser_libp2p", derive(serde::Serialize))]
pub struct BrowserProbeResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub protocol: String,
    pub payload: Vec<u8>,
    pub error: Option<String>,
}

impl BrowserProbeResult {
    pub fn ok(local_peer_id: impl Into<String>, protocol: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            protocol: protocol.into(),
            payload,
            error: None,
        }
    }

    pub fn err(local_peer_id: impl Into<String>, protocol: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            protocol: protocol.into(),
            payload: Vec::new(),
            error: Some(error.into()),
        }
    }
}
```

- [ ] **Step 4: Implement `dialBrowserProbe` behind `browser_libp2p`**

Add a wasm export:

```rust
#[cfg(feature = "browser_libp2p")]
#[wasm_bindgen(js_name = dialBrowserProbe)]
pub async fn dial_browser_probe(seed: &[u8], address: String, payload: &[u8]) -> Result<JsValue, JsValue> {
    let seed = seed_array(seed).map_err(|err| JsValue::from_str(&err))?;
    let local_peer_id = peer_id_from_seed_bytes(&seed).map_err(|err| JsValue::from_str(&err))?;
    let outcome = dial_browser_probe_inner(seed, address, payload.to_vec()).await;
    let result = match outcome {
        Ok(payload) => BrowserProbeResult::ok(local_peer_id, auki_network::BROWSER_PROBE_PROTOCOL, payload),
        Err(err) => BrowserProbeResult::err(local_peer_id, auki_network::BROWSER_PROBE_PROTOCOL, err),
    };
    serde_wasm_bindgen::to_value(&result).map_err(|err| JsValue::from_str(&err.to_string()))
}
```

Implement `dial_browser_probe_inner(...)` using:

```rust
libp2p::SwarmBuilder::with_existing_identity(identity.keypair().clone())
    .with_wasm_bindgen()
    .with_other_transport(|key| libp2p::webrtc_websys::Transport::new(libp2p::webrtc_websys::Config::new(key)).boxed())
```

and a `request_response::json::Behaviour<BrowserProbeRequest, BrowserProbeResponse>` configured with `StreamProtocol::new(auki_network::BROWSER_PROBE_PROTOCOL)` and `ProtocolSupport::Full`. The function must:

- parse `address` as `Multiaddr`
- dial the remote peer from the `/p2p/<peer-id>` suffix
- call `send_request` with a nonce such as `browser-probe-1`
- poll the swarm until `request_response::Message::Response`
- return `response.payload`
- return a string error for `DialFailure`, `OutboundFailure`, timeout, malformed multiaddr, or response nonce mismatch

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Expected: all commands pass.

Update docs/changelogs. Commit:

```bash
git add Cargo.lock crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "feat: add browser probe dial export"
```

---

### Task 4: Browser-to-Native Smoke Test

**Files:**
- Create: `crates/auki-network-browser-wasm/scripts/browser_probe_smoke.html`
- Create: `crates/auki-network-browser-wasm/scripts/smoke_browser_probe.mjs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/src/sprint.md`
- Modify: changelogs and parking lots only if the runtime smoke fails

- [ ] **Step 1: Add the browser smoke harness**

Create `crates/auki-network-browser-wasm/scripts/browser_probe_smoke.html`:

```html
<!doctype html>
<meta charset="utf-8" />
<script type="module">
  import init, { dialBrowserProbe } from "../pkg-web/auki_network_browser_wasm.js";

  const params = new URLSearchParams(location.search);
  const address = params.get("address");
  const payload = new Uint8Array([7, 8, 9]);
  const seed = new Uint8Array(32).fill(3);

  try {
    await init();
    const result = await dialBrowserProbe(seed, address, payload);
    document.body.dataset.result = JSON.stringify(result);
    document.body.textContent = JSON.stringify(result);
  } catch (err) {
    document.body.dataset.result = JSON.stringify({ ok: false, error: String(err) });
    document.body.textContent = String(err);
  }
</script>
```

- [ ] **Step 2: Add the Playwright smoke script**

Create `crates/auki-network-browser-wasm/scripts/smoke_browser_probe.mjs`:

```js
import http from "node:http";
import path from "node:path";
import { readFile } from "node:fs/promises";
import { chromium } from "playwright";

const address = process.argv[2];
if (!address) {
  throw new Error("usage: node scripts/smoke_browser_probe.mjs <webrtc-direct-multiaddr>");
}

const root = path.resolve("crates/auki-network-browser-wasm");
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  const filePath = path.join(root, url.pathname === "/" ? "scripts/browser_probe_smoke.html" : url.pathname);
  const body = await readFile(filePath);
  res.setHeader("content-type", filePath.endsWith(".wasm") ? "application/wasm" : "text/html");
  res.end(body);
});

await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${port}/scripts/browser_probe_smoke.html?address=${encodeURIComponent(address)}`);
  await page.waitForFunction(() => document.body.dataset.result);
  const result = JSON.parse(await page.locator("body").getAttribute("data-result"));
  if (!result.ok) throw new Error(result.error);
  if (result.protocol !== "/auki/browser-probe/0.0.1") throw new Error(`bad protocol: ${result.protocol}`);
  if (result.payload.join(",") !== "7,8,9") throw new Error(`bad payload: ${result.payload}`);
  console.log(`ok ${result.local_peer_id}`);
} finally {
  await browser.close();
  server.close();
}
```

- [ ] **Step 3: Run the end-to-end smoke**

Terminal A:

```bash
cargo run -p auki-network --features browser_probe --example browser_probe_listener
```

Copy the printed `PARK_BROWSER_PROBE_ADDR=...` value.

Terminal B:

```bash
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
node crates/auki-network-browser-wasm/scripts/smoke_browser_probe.mjs '<PARK_BROWSER_PROBE_ADDR value>'
```

Expected: `ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar`.

If the smoke fails after compile success, capture the exact runtime blocker in `crates/auki-network-browser-wasm/parking_lot.md` with classification `browser API`, `native listener`, `address advertisement`, `certificate`, or `SDK architecture`, then propagate the parking-lot summary upward.

- [ ] **Step 4: Commit**

Update status docs/changelogs. Commit:

```bash
git add crates/auki-network-browser-wasm crates/auki-network crates/changelog.md changelog.md
git commit -m "test: smoke browser webrtc probe stream"
```

---

## Follow-Up Plan Required

If the smoke passes, write the next plan to replace `auki-domain-browser`'s `transport_unavailable` shell with this wasm transport for Discovery-selected Domain join and participant metadata.

If the smoke fails because WebRTC Direct cannot produce a browser-to-native connection on local networks, write the next plan for the WebTransport candidate before falling back to Secure WebSocket.

## Self-Review Notes

- Spec coverage: The plan preserves the SDK-owned networking rule, keeps browser media/call shortcuts out of scope, and proves exactly one named protocol stream before Domain join or audio.
- Placeholder scan: No task contains empty placeholder markers. Runtime failure handling requires exact blocker capture rather than generic follow-up text.
- Type consistency: Protocol names, request/response structs, and exported `dialBrowserProbe` names are stable across native Rust, wasm Rust, and the browser smoke harness.
