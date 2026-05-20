# Browser Domain Peer Symmetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make browser Park instances true Auki Domain peers without creating browser forks of `auki-network` or `auki-domain`.

**Architecture:** Browser support is a platform target of the same Rust SDK logic. `auki-network` owns identity, reachability records, protocol IDs, wire types, framing, and stream semantics. `auki-domain` owns Domain creation, join, membership, Manager role, liveness, election, participant metadata, and resource/sensor semantics. Browser crates are packaging layers: wasm bindings for browser runtime primitives, and a TypeScript facade for Park. Only the concrete runtime seams forced by wasm/browser constraints may differ: transport construction, task spawning, timers, HTTP/fetch, and browser persistence.

**Tech Stack:** Rust 2024, rust-libp2p 0.56, `libp2p-webrtc`, `libp2p-webrtc-websys`, `libp2p-relay`, `libp2p-stream`, `wasm-bindgen`, TypeScript facade glue, Vitest for facade tests, `wasm-pack`, Chrome smoke via `playwright-core`.

---

## Non-Negotiable Constraints

- A peer is a peer is a peer. Browser, native, robot, and phone runtimes all use the same Domain roles.
- No Domain semantics may be reimplemented in TypeScript.
- No network protocol semantics may be reimplemented in `auki-network-browser-wasm`.
- Browser/native code may differ only where the runtime makes it necessary: transport builder, executor/spawn, timers, HTTP/fetch, and persistence.
- Discovery, reachability records, membership JSON, Manager election, liveness semantics, participant info, sensor catalogs, and stream protocol shapes are shared SDK logic.
- `auki-domain-browser` is a JS facade over wasm-backed SDK sessions, not a Domain engine.
- `auki-network-browser-wasm` is a wasm binding/packaging crate for `auki-network`, not a parallel browser implementation of `auki-network`.
- The browser path must prove both directions: browser creates/manages a Domain and another peer joins it; browser also joins a Domain currently managed by another peer.

---

## File Structure

- `docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md` - design contract locking peer symmetry and shared-SDK implementation rules.
- `crates/auki-network/src/` - shared network protocol and reachability logic. Browser work should add wasm-compatible feature paths here first.
- `crates/auki-network-browser-wasm/src/lib.rs` - wasm exports that call `auki-network`; no duplicate protocol structs or protocol logic.
- `crates/auki-domain/src/` - shared Domain state machine. Browser work should make this compile/usefully expose wasm-compatible sessions.
- `crates/auki-domain-browser/src/` - TypeScript facade over wasm exports; no create/join/election implementation.
- `crates/auki-*/tests` and `crates/auki-network-browser-wasm/scripts/` - shared conformance fixtures and browser smoke harnesses.
- `changelog.md`, `docs/changelog.md`, `docs/superpowers/changelog.md`, `docs/superpowers/plans/changelog.md`, and per-crate changelogs - propagated documentation and implementation history.

This plan replaces the earlier asymmetric-role plan and the rejected runtime-interface draft. Keep every runtime seam tied to a concrete wasm/browser constraint. Add tiny concrete runtime seams only after the shared Rust crate fails to compile or run on wasm for a specific reason.

---

### Task 1: Lock the Shared-SDK Peer Symmetry Spec

**Files:**
- Create: `docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md`
- Modify: `docs/superpowers/specs/changelog.md`
- Modify: `docs/superpowers/changelog.md`
- Modify: `docs/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write the design spec**

Create `docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md`:

```md
# Domain Peer Symmetry Design

## Principle

A peer is a peer is a peer. Browser, native, robot, and phone runtimes may have different runtime primitives, but they all participate in the same Domain role model and use the same SDK semantics.

## Shared Logic Rule

`auki-network` and `auki-domain` are the source of truth.

- Protocol IDs, wire types, framing, reachability records, and stream semantics live in `auki-network`.
- Domain creation, join, membership, Manager role, liveness, election, participant metadata, resource catalogs, and sensor semantics live in `auki-domain`.
- Browser crates expose those Rust crates to JavaScript; they do not fork the logic.

## Runtime Seam Rule

Browser/native code may differ only where the runtime forces it:

- libp2p transport construction
- task spawning and wakeups
- timers
- HTTP/fetch
- persistent identity storage
- media device capture/playback

Each seam must be concrete and justified by a compile/runtime constraint. No generic runtime layer is introduced ahead of need.

## Role Model

- Any peer can create a Domain.
- Any peer can be elected Manager.
- Any peer can publish participant metadata.
- Any peer can publish and consume sensors.
- Any peer can fail, recover, or be replaced.
- Discovery stores the current Manager by PeerId, not by platform class.

## Reachability Model

Reachability is shared SDK data. Runtime-specific code only obtains currently advertised SDK routes.

If a browser cannot accept a direct inbound UDP flow, the SDK must provide another SDK-owned route: relay, WebTransport, WebSocket relay, circuit relay, or another libp2p-compatible path. That route is a reachability fact, not a role rule.

## Milestone Shape

The first symmetric milestone is:

1. Browser peer creates a Domain and becomes Manager.
2. A second peer joins that browser-managed Domain through Discovery.
3. Browser Manager serves participant info through SDK protocols.
4. Browser Manager publishes liveness through Discovery.
5. Browser Manager failure triggers normal election.

The reciprocal path, browser joins another peer's Domain, is required too, but it is not allowed to be the only first implementation path.
```

- [ ] **Step 2: Self-review the spec**

Run:

```bash
rg "separate browser semantics|duplicated SDK semantics|placeholder language" docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md
```

Expected: no matches.

- [ ] **Step 3: Update changelogs and commit**

Prepend entries to the docs changelog chain and root changelog explaining that peer symmetry now also means shared Rust SDK logic.

```bash
git add docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md docs/superpowers/specs/changelog.md docs/superpowers/changelog.md docs/changelog.md changelog.md
git commit -m "docs: specify shared sdk peer symmetry"
```

---

### Task 2: Add Shared Cross-Target Conformance Fixtures

**Files:**
- Create: `crates/auki-network/tests/conformance_fixtures.rs`
- Create: `crates/auki-network-browser-wasm/src/conformance.rs`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/scripts/conformance_smoke.mjs`
- Modify: docs/changelogs

- [ ] **Step 1: Write native conformance fixture tests**

Create `crates/auki-network/tests/conformance_fixtures.rs`:

```rust
use auki_network::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, PeerIdentity,
    ReachabilityRecord,
};

#[test]
fn locked_seed_03_peer_id_is_shared_across_targets() {
    let identity = PeerIdentity::from_seed(&[3u8; 32]);
    assert_eq!(
        identity.peer_id().to_string(),
        "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
    );
}

#[test]
fn browser_probe_json_fixture_is_shared() {
    let request = BrowserProbeRequest {
        nonce: "browser-probe-1".to_string(),
        payload: vec![7, 8, 9],
    };
    let response = BrowserProbeResponse::from_request(&request, "native:peer");

    assert_eq!(BROWSER_PROBE_PROTOCOL, "/auki/browser-probe/0.0.1");
    assert_eq!(
        serde_json::to_string(&request).expect("request encodes"),
        r#"{"nonce":"browser-probe-1","payload":[7,8,9]}"#
    );
    assert_eq!(
        serde_json::to_string(&response).expect("response encodes"),
        r#"{"nonce":"browser-probe-1","payload":[7,8,9],"responder":"native:peer"}"#
    );
}

#[test]
fn reachability_record_json_fixture_is_shared() {
    let record = ReachabilityRecord {
        peer_id: PeerIdentity::from_seed(&[3u8; 32]).peer_id(),
        addresses: vec!["/p2p/relay/p2p-circuit/p2p/12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
            .parse()
            .expect("valid multiaddr")],
        capabilities: vec![],
        last_seen_ns: 42,
    };

    let json = serde_json::to_string(&record).expect("record encodes");
    assert!(json.contains("\"addresses\":[\"/p2p/relay/p2p-circuit/p2p/12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar\"]"));
}
```

- [ ] **Step 2: Run the native fixtures**

```bash
cargo test -p auki-network --test conformance_fixtures
```

Expected: pass. If this fails, fix the fixture against existing shared SDK types, not by creating browser-only copies.

- [ ] **Step 3: Expose wasm conformance values from `auki-network-browser-wasm`**

Create `crates/auki-network-browser-wasm/src/conformance.rs`:

```rust
use auki_network::{
    BROWSER_PROBE_PROTOCOL, BrowserProbeRequest, BrowserProbeResponse, PeerIdentity,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct NetworkConformance {
    pub peer_id_seed_03: String,
    pub browser_probe_protocol: String,
    pub browser_probe_request_json: String,
    pub browser_probe_response_json: String,
}

pub fn network_conformance() -> NetworkConformance {
    let request = BrowserProbeRequest {
        nonce: "browser-probe-1".to_string(),
        payload: vec![7, 8, 9],
    };
    let response = BrowserProbeResponse::from_request(&request, "native:peer");

    NetworkConformance {
        peer_id_seed_03: PeerIdentity::from_seed(&[3u8; 32]).peer_id().to_string(),
        browser_probe_protocol: BROWSER_PROBE_PROTOCOL.to_string(),
        browser_probe_request_json: serde_json::to_string(&request).expect("request encodes"),
        browser_probe_response_json: serde_json::to_string(&response).expect("response encodes"),
    }
}
```

Modify `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(feature = "browser_libp2p")]
mod conformance;

#[cfg(feature = "browser_libp2p")]
#[wasm_bindgen(js_name = networkConformance)]
pub fn network_conformance() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&conformance::network_conformance())
        .map_err(|err| JsValue::from_str(&err.to_string()))
}
```

- [ ] **Step 4: Add the browser conformance smoke**

Create `crates/auki-network-browser-wasm/scripts/conformance_smoke.mjs`:

```js
import init, { networkConformance } from "../pkg-node/auki_network_browser_wasm.js";

await init();
const result = networkConformance();

if (result.peer_id_seed_03 !== "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar") {
  throw new Error(`bad peer id: ${result.peer_id_seed_03}`);
}
if (result.browser_probe_protocol !== "/auki/browser-probe/0.0.1") {
  throw new Error(`bad protocol: ${result.browser_probe_protocol}`);
}
if (result.browser_probe_request_json !== "{\"nonce\":\"browser-probe-1\",\"payload\":[7,8,9]}") {
  throw new Error(`bad request fixture: ${result.browser_probe_request_json}`);
}
if (
  result.browser_probe_response_json !==
  "{\"nonce\":\"browser-probe-1\",\"payload\":[7,8,9],\"responder\":\"native:peer\"}"
) {
  throw new Error(`bad response fixture: ${result.browser_probe_response_json}`);
}

console.log(`ok ${result.peer_id_seed_03}`);
```

- [ ] **Step 5: Verify and commit**

```bash
cargo test -p auki-network --test conformance_fixtures
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target nodejs --out-dir pkg-node -- --features browser_libp2p
node crates/auki-network-browser-wasm/scripts/conformance_smoke.mjs
```

Expected: all pass. The Node smoke prints:

```text
ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar
```

Update docs/changelogs, then commit:

```bash
git add Cargo.lock crates/auki-network crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "test: add network cross-target conformance"
```

---

### Task 3: Move Browser Probe Logic Back Behind Shared `auki-network`

**Files:**
- Modify: `crates/auki-network/src/browser_probe.rs`
- Modify: `crates/auki-network/src/browser_probe_protocol.rs`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: changelogs

- [ ] **Step 1: Add a shared probe outcome type to `auki-network`**

Add to `crates/auki-network/src/browser_probe_protocol.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserProbeOutcome {
    pub ok: bool,
    pub local_peer_id: String,
    pub protocol: String,
    pub payload: Vec<u8>,
    pub error: Option<String>,
}

impl BrowserProbeOutcome {
    pub fn ok(local_peer_id: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            protocol: BROWSER_PROBE_PROTOCOL.to_string(),
            payload,
            error: None,
        }
    }

    pub fn err(local_peer_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            protocol: BROWSER_PROBE_PROTOCOL.to_string(),
            payload: Vec::new(),
            error: Some(error.into()),
        }
    }
}
```

- [ ] **Step 2: Write the failing shared outcome test**

Add to the existing test module:

```rust
#[test]
fn browser_probe_outcome_uses_shared_protocol_id() {
    let outcome = BrowserProbeOutcome::ok("peer", vec![1, 2, 3]);

    assert!(outcome.ok);
    assert_eq!(outcome.local_peer_id, "peer");
    assert_eq!(outcome.protocol, BROWSER_PROBE_PROTOCOL);
    assert_eq!(outcome.payload, vec![1, 2, 3]);
    assert!(outcome.error.is_none());
}
```

Run:

```bash
cargo test -p auki-network browser_probe_protocol::tests::browser_probe_outcome_uses_shared_protocol_id
```

Expected: pass after the type is added.

- [ ] **Step 3: Delete the duplicate wasm outcome type**

In `crates/auki-network-browser-wasm/src/lib.rs`, remove the local `BrowserProbeResult` struct and its native tests. In `dial_browser_probe`, build `auki_network::BrowserProbeOutcome::ok(...)` or `::err(...)`, then serialize it through `serde_wasm_bindgen`.

- [ ] **Step 4: Verify no duplicate probe result remains**

```bash
rg "struct BrowserProbeResult|BrowserProbeResult" crates/auki-network-browser-wasm/src
```

Expected: no matches.

- [ ] **Step 5: Verify and commit**

```bash
cargo test -p auki-network browser_probe_protocol::tests::browser_probe_outcome_uses_shared_protocol_id
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Update docs/changelogs, then commit:

```bash
git add Cargo.lock crates/auki-network crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "refactor: share browser probe outcome"
```

---

### Task 4: Make `auki-domain` the Browser Domain Engine

**Files:**
- Modify: `crates/auki-domain/Cargo.toml`
- Modify: `crates/auki-domain/src/lib.rs`
- Create: `crates/auki-domain/src/browser_session.rs`
- Modify: `crates/auki-domain/src/readme.md`
- Modify: `crates/auki-domain/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Add a wasm compile target check for `auki-domain`**

Run:

```bash
cargo check -p auki-domain --target wasm32-unknown-unknown
```

Expected: likely failure. Record the exact first native-only dependency or Tokio/runtime assumption in the task notes before changing code.

- [ ] **Step 2: Add the smallest feature gate for shared Domain logic**

Modify `crates/auki-domain/Cargo.toml` so the default/shared pieces compile without native-only runtime features:

```toml
[features]
default = []
native_runtime = ["dep:tokio"]
browser_runtime = ["dep:wasm-bindgen", "dep:wasm-bindgen-futures"]
```

Use the existing dependency names if they already exist. Do not add a broad runtime abstraction layer. Move only code that actually fails wasm compilation behind `native_runtime`.

- [ ] **Step 3: Add a shared browser session facade in Rust**

Create `crates/auki-domain/src/browser_session.rs`:

```rust
use auki_network::PeerIdentity;

pub struct BrowserDomainSession {
    identity: PeerIdentity,
}

impl BrowserDomainSession {
    pub fn new(identity: PeerIdentity) -> Self {
        Self { identity }
    }

    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }
}
```

Export it from `crates/auki-domain/src/lib.rs` behind `browser_runtime`:

```rust
#[cfg(feature = "browser_runtime")]
pub mod browser_session;
```

This is deliberately tiny. It establishes that browser sessions live in `auki-domain`, then later tasks move create/join/election into this module by reusing existing `ClusterManager` pieces.

- [ ] **Step 4: Verify**

```bash
cargo check -p auki-domain --target wasm32-unknown-unknown --features browser_runtime
cargo check -p auki-domain
```

Expected: both pass. If native default previously required runtime features, update the native consumers to request `native_runtime` explicitly.

- [ ] **Step 5: Commit**

Update docs/changelogs, then commit:

```bash
git add Cargo.toml Cargo.lock crates/auki-domain crates/changelog.md changelog.md
git commit -m "refactor: make domain browser session live in auki-domain"
```

---

### Task 5: Make `auki-domain-browser` a Thin WASM Facade

**Files:**
- Modify: `crates/auki-domain-browser/src/peer.ts`
- Modify: `crates/auki-domain-browser/src/peer.test.ts`
- Modify: `crates/auki-domain-browser/src/README.md`
- Modify: `crates/auki-domain-browser/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Write facade-boundary tests**

Add to `crates/auki-domain-browser/src/peer.test.ts`:

```ts
it("delegates createDomain to the wasm SDK session", async () => {
  const session = {
    peerId: () => "browser-peer",
    listDomains: vi.fn(),
    createDomain: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    joinDomain: vi.fn(),
    leaveDomain: vi.fn(),
    observeParticipants: vi.fn(),
    setParticipantMetadata: vi.fn(),
    declareLocalSensors: vi.fn(),
    setSensorPublication: vi.fn(),
    subscribeToSensor: vi.fn(),
    unsubscribeFromSensor: vi.fn(),
  };

  const peer = await createBrowserDomainPeer({ peerId: "browser-peer", sdkSession: session });

  await expect(peer.createDomain("http://discovery.example", "demo")).resolves.toEqual({
    ok: true,
    value: undefined,
  });
  expect(session.createDomain).toHaveBeenCalledWith("http://discovery.example", "demo");
});

it("does not implement join semantics in TypeScript", async () => {
  const source = await import("node:fs/promises").then((fs) =>
    fs.readFile(new URL("./peer.ts", import.meta.url), "utf8"),
  );

  expect(source).not.toContain("membershipJson");
  expect(source).not.toContain("managerReachability");
  expect(source).not.toContain("elect");
});
```

- [ ] **Step 2: Run the failing tests**

```bash
npm --prefix crates/auki-domain-browser test -- peer.test.ts
```

Expected: failure because `sdkSession` is not accepted and peer methods still contain local shell behavior.

- [ ] **Step 3: Replace local shell methods with session delegation**

Modify `crates/auki-domain-browser/src/peer.ts`:

```ts
import type { BrowserDomainPeer, PeerId } from "./contract.js";
import { transportUnavailable } from "./errors.js";

type SdkBrowserDomainSession = BrowserDomainPeer & {
  peerId?: () => PeerId;
};

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  sdkSession?: SdkBrowserDomainSession;
};

export async function createBrowserDomainPeer(
  options: CreateBrowserDomainPeerOptions,
): Promise<BrowserDomainPeer> {
  if (!options.sdkSession) {
    return failClosedPeer(options.peerId);
  }

  return options.sdkSession;
}
```

Keep `failClosedPeer(peerId)` only as a temporary shell for package import tests. It must contain no Domain create/join/election behavior.

- [ ] **Step 4: Verify TypeScript contains no Domain semantics**

```bash
rg "membershipJson|managerReachability|election|liveness|successor|manager_multiaddrs|JoinResponse" crates/auki-domain-browser/src
```

Expected: no matches outside type names imported directly from generated wasm declarations. If this command finds local logic in `.ts` files, move that logic into Rust/wasm first.

- [ ] **Step 5: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test
npm --prefix crates/auki-domain-browser run build
```

Update docs/changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "refactor: make browser domain package a wasm facade"
```

---

### Task 6: Implement Symmetric Browser Manager in Shared Rust Logic

**Files:**
- Modify: `crates/auki-domain/src/browser_session.rs`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/scripts/browser_manager_smoke.html`
- Modify: `crates/auki-network-browser-wasm/scripts/smoke_browser_manager.mjs`
- Modify: docs/changelogs and parking lots if reachability is blocked

- [ ] **Step 1: Write the Rust session result tests in `auki-domain`**

Add to `crates/auki-domain/src/browser_session.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use auki_network::PeerIdentity;

    #[test]
    fn browser_session_peer_id_uses_shared_identity() {
        let session = BrowserDomainSession::new(PeerIdentity::from_seed(&[3u8; 32]));
        assert_eq!(
            session.peer_id(),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
    }
}
```

- [ ] **Step 2: Add create-domain API in `auki-domain`**

Extend `BrowserDomainSession` with:

```rust
pub async fn create_domain(
    &mut self,
    discovery_url: &str,
    domain_name: &str,
) -> Result<CreateDomainOutcome, BrowserDomainError> {
    // Reuse existing ClusterManager create semantics.
    // Use the browser-compatible network runtime and browser-compatible Discovery HTTP path.
    // Do not implement separate membership or Manager election logic here.
}
```

If existing `ClusterManager` cannot be reused because of native runtime assumptions, refactor the smallest blocking piece into `auki-domain` shared code first. Do not copy the algorithm.

- [ ] **Step 3: Expose the Rust session through wasm**

In `crates/auki-network-browser-wasm/src/lib.rs`, export a wasm class whose methods call `auki_domain::browser_session::BrowserDomainSession`:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
pub struct BrowserDomainSession {
    inner: auki_domain::browser_session::BrowserDomainSession,
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
impl BrowserDomainSession {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: &[u8]) -> Result<Self, JsValue> {
        let seed = seed_array(seed).map_err(|err| JsValue::from_str(&err))?;
        let identity = peer_identity_from_seed_bytes(&seed);
        Ok(Self {
            inner: auki_domain::browser_session::BrowserDomainSession::new(identity),
        })
    }

    #[wasm_bindgen(js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.inner.peer_id()
    }
}
```

- [ ] **Step 4: Add browser-as-Manager smoke**

Create `crates/auki-network-browser-wasm/scripts/browser_manager_smoke.html`:

```html
<!doctype html>
<meta charset="utf-8" />
<script type="module">
  import init, { BrowserDomainSession } from "../pkg-web/auki_network_browser_wasm.js";

  const params = new URLSearchParams(location.search);
  const discoveryUrl = params.get("discovery");
  const domainName = params.get("domain") ?? "browser-manager-smoke";
  const seed = new Uint8Array(32).fill(3);

  try {
    await init();
    const session = new BrowserDomainSession(seed);
    const result = await session.createDomain(discoveryUrl, domainName);
    document.body.dataset.result = JSON.stringify(result);
    document.body.textContent = JSON.stringify(result);
  } catch (err) {
    document.body.dataset.result = JSON.stringify({ ok: false, error: String(err) });
    document.body.textContent = String(err);
  }
</script>
```

- [ ] **Step 5: Verify**

```bash
cargo test -p auki-domain browser_session
cargo check -p auki-domain --target wasm32-unknown-unknown --features browser_runtime
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

- [ ] **Step 6: Handle reachability blockers as SDK blockers**

If browser Manager reachability cannot be advertised yet, add this item to `crates/auki-network-browser-wasm/parking_lot.md` and propagate upward:

```md
- **2026-05-20 — Browser Manager SDK reachability route.** Browser peers are role-symmetric and use shared `auki-domain` logic, but the current wasm transport cannot yet advertise an SDK-owned route for Manager duties. Evaluate wasm relay/circuit support first, then SDK WebTransport/WebSocket relay if relay is unavailable.
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.lock crates/auki-domain crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "feat: expose shared browser domain session"
```

---

## Self-Review Notes

- Spec coverage: The plan now addresses the drift problem directly. Shared Rust crates own protocol and Domain logic; browser crates expose them to JavaScript.
- Placeholder scan: The plan avoids placeholder instructions and names the known reachability risk explicitly.
- Type consistency: `BrowserDomainSession`, `ReachabilityRecord`, `BrowserProbeOutcome`, and shared `auki-domain` session naming are consistent across tasks.
- Known risk: compiling the existing native Domain runtime to wasm may expose Tokio, HTTP, or libp2p assumptions. The rule is to refactor the smallest concrete blocker, not to create a second browser Domain implementation.
