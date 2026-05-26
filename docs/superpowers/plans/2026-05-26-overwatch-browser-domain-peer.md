# Overwatch Browser Domain Peer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a stable SDK-owned browser Domain peer runtime and a web-only `examples/overwatch` app that proves browser peers can rendezvous through Discovery, connect over SDK networking, exchange peer/catalog/stream data, and render a Park-inspired operator surface without an app backend.

**Architecture:** Discovery remains a bootstrap directory: clusters, Manager dial hints, relay hints, and infrastructure nodes. Browser peer orchestration lives in generated SDK JavaScript packages, with Rust/WASM providing identity, protocol constants, DTO validation, membership helpers, and stream-message encoding. The wasm-pack generated bindings are the source of truth for Rust/WASM SDK exports; JS-owned browser runtime glue must generate its declarations from typed source instead of hand-maintaining SDK surface in `.d.ts` templates. Overwatch is only an example and acceptance surface over that SDK browser adapter.

**Tech Stack:** Rust 2024, wasm-bindgen, wasm-pack, generated JavaScript ESM packages, js-libp2p WebRTC/WebRTC-Direct transport, Discovery HTTP API, Vite, React, TypeScript, Tailwind CSS, Vitest, Playwright smoke testing, browser `localStorage` for example identity persistence.

---

## Decisions

- Discovery is allowed, but only as rendezvous/directory infrastructure.
- Do not add an Overwatch backend server. `just overwatch` may run Vite for static assets and HMR only.
- Do not implement app-specific signaling in Overwatch.
- First try libp2p browser transports plus Discovery relay hints. If that cannot establish browser-to-browser reachability, stop and add a generic SDK/Discovery signaling design instead of hiding the gap in Overwatch.
- Park is a UI/functionality reference only. Do not import Park server APIs, Park state polling, or Park app architecture.
- wasm-pack generated declarations are the authoritative SDK surface for Rust/WASM exports. Do not add new browser SDK API by manually typing `index.d.ts.tmpl`; JS-owned browser adapters must author typed source and emit declarations from that source.

## File Structure

Primary SDK changes:

- `crates/auki-network/src/discovery_client.rs` — include `relay_multiaddrs`, expose Discovery `/nodes`, and retain typed Rust parsing.
- `crates/auki-network/src/wasm.rs` — expose stream protocol constants and stream-message byte helpers.
- `crates/auki-network/bindings/javascript/index.js.tmpl` — add browser Discovery client, framed handler helpers, and stream helpers.
- `crates/auki-network/bindings/javascript/src/*.ts` — typed source for JS-owned browser runtime glue; emitted declarations supplement the wasm-pack declarations.
- `crates/auki-network/bindings/javascript/index.d.ts.tmpl` — keep only transitional re-export/glue declarations that cannot yet be emitted; do not define new SDK surface here.
- `crates/auki-network/bindings/javascript/test/*.tmpl` — generated package tests for Discovery and framed protocol helpers.
- `crates/auki-domain/src/wasm.rs` — add the wasm-pack generated browser Domain peer core/state surface.
- `crates/auki-domain/bindings/javascript/index.js.tmpl` — wire JS transport adapter code around the wasm-generated browser Domain peer core.
- `crates/auki-domain/bindings/javascript/src/*.ts` — typed source for JS-only transport adapter glue; emitted declarations define adapter methods only, not the semantic SDK core.
- `crates/auki-domain/bindings/javascript/index.d.ts.tmpl` — keep only transitional re-export/glue declarations that cannot yet be emitted; do not define the browser facade surface here.
- `crates/auki-domain/bindings/javascript/test/*.tmpl` — generated package tests for browser create/join, membership, catalogs, and debug state.

Example app:

- `examples/overwatch/README.md` — runbook and acceptance definition.
- `examples/overwatch/package.json` — Vite/React/Vitest/Playwright scripts.
- `examples/overwatch/index.html` — static entry.
- `examples/overwatch/vite.config.ts` and `vitest.config.ts`.
- `examples/overwatch/tailwind.config.ts` and `postcss.config.js` — Tailwind configuration.
- `examples/overwatch/scripts/stage-sdk.mjs` — verifies/stages generated SDK packages.
- `examples/overwatch/scripts/smoke-two-browser.mjs` — starts local Discovery and proves two browser peers.
- `examples/overwatch/src/sdk/createOverwatchPeer.ts` — imports the generated SDK browser adapter.
- `examples/overwatch/src/sdk/contract.ts` — app-facing type aliases inferred from generated SDK bindings.
- `examples/overwatch/src/state/*.ts` — app state, storage, derived view models.
- `examples/overwatch/src/App.tsx` and `src/main.tsx` — React entry and app shell.
- `examples/overwatch/src/components/**` — directory, peer detail, stage, tiles, sensor strip, quick search.
- `examples/overwatch/src/index.css` — Tailwind directives and Park-inspired dense operator theme.

Root/supporting files:

- `justfile` — add `overwatch` and `overwatch-smoke`.

---

## Task 0: Lock JavaScript SDK Type Ownership

**Files:**
- Modify: `scripts/bindings/generate_bindings.py`
- Modify: `crates/auki-network/bindings.toml`
- Modify: `crates/auki-network/bindings/javascript/index.d.ts.tmpl`
- Modify: `crates/auki-network/bindings/javascript/package.json.tmpl`
- Modify: `crates/auki-domain/bindings.toml`
- Modify: `crates/auki-domain/bindings/javascript/index.d.ts.tmpl`
- Modify: `crates/auki-domain/bindings/javascript/package.json.tmpl`
- Add: focused generator tests under `scripts/bindings/` if the existing generator tests do not cover declaration ownership.

- [ ] **Step 1: Add a failing declaration ownership test**

Add a generator or package test that proves generated JavaScript packages do not hand-declare wasm-pack exports. The check should fail if `index.d.ts.tmpl` manually declares functions/classes that are already present in the wasm-pack generated declaration file.

Expected generated package shape:

```ts
export * from "./$generated_js_file";
export { default } from "./$generated_js_file";
```

If the top-level wrapper still needs to export JS-only transport glue, those declarations must come from typed source emitted by the package build, not from manually maintained `.d.ts.tmpl` SDK declarations.

- [ ] **Step 2: Reduce `index.d.ts.tmpl` to generated re-exports**

Update both JavaScript binding templates so `index.d.ts.tmpl` re-exports the wasm-pack generated declaration file and contains no hand-written declarations for Rust/WASM exports such as `peerIdFromSeed`, protocol constants, DTO helpers, or Domain wasm helpers.

- [ ] **Step 3: Add a typed-source path for JS-only transport glue**

Teach the binding generator how to include optional `bindings/javascript/src/**/*.ts` source files and run declaration emission for JS-owned glue when a crate opts in. This path is only for browser transport adapter code that cannot be emitted by wasm-pack because it wraps jslibp2p/WebRTC objects.

The source-of-truth rule:

```text
Rust/WASM SDK API      -> wasm-bindgen source -> wasm-pack .d.ts
JS transport adapter   -> TypeScript source   -> tsc-emitted .d.ts
index.d.ts.tmpl        -> re-export bridge only
```

- [ ] **Step 4: Update package metadata**

Update generated package metadata so npm `types` points at a generated declaration entrypoint assembled from wasm-pack declarations plus any tsc-emitted JS adapter declarations. Keep this generated artifact out of the source templates when possible; the template may only wire exports together.

- [ ] **Step 5: Verify**

Run:

```bash
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
```

Expected: PASS. The generated packages still type-check, and the public wasm SDK exports are sourced from wasm-pack declarations.

- [ ] **Step 6: Commit**

```bash
git add scripts/bindings crates/auki-network crates/auki-domain
git commit -m "fix: make wasm-pack declarations authoritative for JavaScript bindings"
```

---

## Task 1: Complete Discovery Directory Exposure In `auki-network`

**Files:**
- Modify: `crates/auki-network/src/discovery_client.rs`
- Modify: `crates/auki-network/src/ffi.rs`
- Modify: `crates/auki-network/src/readme.md`
- Modify: `crates/auki-network/src/sprint.md`
- Modify: `crates/auki-network/README.md`

- [ ] **Step 1: Add failing Rust tests for relay hints and node directory**

In `crates/auki-network/src/discovery_client.rs`, extend `sample_wire_entry()` in the existing tests:

```rust
relay_multiaddrs: vec!["/dns4/relay.local/tcp/443/wss/p2p/12D3KooWRelay/p2p-circuit".to_string()],
```

Add assertions in `parse_wire_entry_round_trips_via_typed_form`:

```rust
assert_eq!(
    entry
        .relay_multiaddrs
        .iter()
        .map(|m| m.to_string())
        .collect::<Vec<_>>(),
    w.relay_multiaddrs
);
```

Add a node parser test:

```rust
#[test]
fn parse_wire_node_entry_round_trips_via_typed_form() {
    let wire = WireNodeEntry {
        peer_id: "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw".to_string(),
        node_type: "relay".to_string(),
        multiaddrs: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
        created_ns: 1,
        last_liveness_check_ns: 2,
    };

    let node = parse_wire_node_entry(wire.clone()).expect("valid wire node parses");

    assert_eq!(node.peer_id.to_string(), wire.peer_id);
    assert_eq!(node.node_type, "relay");
    assert_eq!(node.multiaddrs[0].to_string(), "/ip4/127.0.0.1/tcp/4001");
    assert_eq!(node.created_ns, 1);
    assert_eq!(node.last_liveness_check_ns, 2);
}
```

- [ ] **Step 2: Run the focused failing test**

Run:

```bash
cargo test -p auki-network -F discovery_client discovery_client::tests::parse_wire_entry_round_trips_via_typed_form discovery_client::tests::parse_wire_node_entry_round_trips_via_typed_form
```

Expected: FAIL because `ClusterEntry.relay_multiaddrs`, `WireNodeEntry`, and `parse_wire_node_entry` do not exist yet.

- [ ] **Step 3: Implement typed relay and node surfaces**

In `crates/auki-network/src/discovery_client.rs`, add:

```rust
pub struct ClusterEntry {
    pub name: String,
    pub manager_peer_id: PeerId,
    pub manager_multiaddrs: Vec<Multiaddr>,
    pub relay_multiaddrs: Vec<Multiaddr>,
    pub peer_count: u32,
    pub created_ns: i64,
    pub last_liveness_check_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEntry {
    pub peer_id: PeerId,
    pub node_type: String,
    pub multiaddrs: Vec<Multiaddr>,
    pub created_ns: i64,
    pub last_liveness_check_ns: i64,
}
```

Add methods:

```rust
pub async fn list_nodes(&self) -> Result<Vec<NodeEntry>, DiscoveryError>;
pub async fn list_nodes_by_type(&self, node_type: &str) -> Result<Vec<NodeEntry>, DiscoveryError>;
pub async fn create_cluster_with_relays(
    &self,
    name: &str,
    manager_peer_id: &PeerId,
    manager_multiaddrs: &[Multiaddr],
    relay_multiaddrs: &[Multiaddr],
) -> Result<CreateClusterOutcome, DiscoveryError>;
pub async fn rotate_manager_with_relays(
    &self,
    name: &str,
    manager_peer_id: &PeerId,
    manager_multiaddrs: &[Multiaddr],
    relay_multiaddrs: &[Multiaddr],
) -> Result<ClusterEntry, DiscoveryError>;
```

Keep existing `create_cluster` and `rotate_manager` as compatibility wrappers that pass an empty relay list.

- [ ] **Step 4: Expose relay and node data through native binding JSON**

In `crates/auki-network/src/ffi.rs`, update `discovery_entry_json` to include:

```rust
"relay_multiaddrs": entry
    .relay_multiaddrs
    .iter()
    .map(|addr| addr.to_string())
    .collect::<Vec<_>>(),
```

Update `register_peer_json` to parse optional `relay_multiaddrs` and call the new `*_with_relays` methods.

Add:

```rust
pub fn discover_nodes_json(
    &self,
    query_json: String,
    timeout_ms: u64,
) -> Result<String, BindingNetworkError>
```

The response JSON shape must be:

```json
{
  "nodes": [
    {
      "peer_id": "12D3KooWAfBVdmphtMFPVq3GEpcg3QMiRbrwD9mpd6D6fc4CswRw",
      "node_type": "relay",
      "multiaddrs": ["/ip4/127.0.0.1/tcp/4001"],
      "created_ns": 1,
      "last_liveness_check_ns": 2
    }
  ]
}
```

- [ ] **Step 5: Run tests**

Run:

```bash
cargo test -p auki-network -F discovery_client
cargo test -p auki-network --features discovery_client,swarm native_discovery_client_is_exposed
```

Expected: PASS. Existing Discovery client behavior remains unchanged when callers do not provide relay hints.

- [ ] **Step 6: Update docs**

Update `crates/auki-network/README.md`, `src/readme.md`, and `src/sprint.md` to state that Discovery exposes both cluster Manager hints and infrastructure node/relay hints.

Commit:

```bash
git add crates/auki-network
git commit -m "feat: expose Discovery relay and node hints"
```

---

## Task 2: Add Browser Discovery And Framed Protocol Helpers

**Files:**
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Add: `crates/auki-network/bindings/javascript/src/framed.ts`
- Add: `crates/auki-network/bindings/javascript/src/index.ts`
- Add: `crates/auki-network/bindings/javascript/tsconfig.json`
- Modify: `crates/auki-network/bindings/javascript/README.md.tmpl`
- Add: `crates/auki-network/bindings/javascript/test/discovery-directory-client.test.mjs.tmpl`
- Add: `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl`

- [ ] **Step 1: Add failing generated-package tests**

Create `crates/auki-network/bindings/javascript/test/discovery-directory-client.test.mjs.tmpl`:

```js
import assert from "node:assert/strict";
import test from "node:test";
import { DiscoveryDirectoryClient } from "../index.js";

test("DiscoveryDirectoryClient lists clusters and relay nodes through fetch", async () => {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    calls.push({ url: String(url), method: init.method ?? "GET" });
    if (String(url).endsWith("/clusters")) {
      return jsonResponse({
        clusters: [{
          name: "overwatch",
          manager_peer_id: "peer-manager",
          manager_multiaddrs: ["/webrtc/p2p/peer-manager"],
          relay_multiaddrs: ["/dns4/relay.local/tcp/443/wss/p2p/relay/p2p-circuit"],
          peer_count: 2,
          created_ns: 1,
          last_liveness_check_ns: 2,
        }],
      });
    }
    if (String(url).endsWith("/nodes?type=relay")) {
      return jsonResponse({
        nodes: [{
          peer_id: "relay",
          node_type: "relay",
          multiaddrs: ["/dns4/relay.local/tcp/443/wss/p2p/relay"],
          created_ns: 3,
          last_liveness_check_ns: 4,
        }],
      });
    }
    throw new Error(`unexpected fetch ${url}`);
  };

  globalThis.fetch = fetchImpl;
  const client = new DiscoveryDirectoryClient("http://discovery.local/");
  const clusters = JSON.parse(await client.discoverPeersJson("{}")).clusters;
  const relays = JSON.parse(await client.listNodesJson(JSON.stringify({ type: "relay" }))).nodes;

  assert.equal(clusters[0].relay_multiaddrs.length, 1);
  assert.equal(relays[0].node_type, "relay");
  assert.deepEqual(calls.map((call) => call.url), [
    "http://discovery.local/clusters",
    "http://discovery.local/nodes?type=relay",
  ]);
});

function jsonResponse(body) {
  return {
    ok: true,
    status: 200,
    async json() {
      return body;
    },
    async text() {
      return JSON.stringify(body);
    },
  };
}
```

Create `crates/auki-network/bindings/javascript/test/framed-handler.test.mjs.tmpl` with a fake inbound stream and assert `handleFramed(protocol, handler)` decodes one length-prefixed request and sends one length-prefixed response.

- [ ] **Step 2: Run generated package tests to confirm failure**

Run:

```bash
just generate-javascript-bindings auki-network
```

Expected: FAIL because wasm-pack does not yet export `DiscoveryDirectoryClient` and the JS transport wrapper does not yet export `handleFramed`.

- [ ] **Step 3: Add stream protocol constants**

In `crates/auki-network/src/wasm.rs`, import `crate::stream_protocol::STREAM_PROTOCOL` behind the `wasm` feature path and add:

```rust
#[wasm_bindgen(js_name = streamProtocol)]
pub fn stream_protocol() -> String {
    STREAM_PROTOCOL.to_string()
}
```

Add `"stream": STREAM_PROTOCOL` to `aukiNetworkProtocolsJson`.

- [ ] **Step 4: Add wasm browser Discovery client source**

Add `DiscoveryDirectoryClient` as a `#[wasm_bindgen]` type in `crates/auki-network/src/wasm.rs` or a wasm-only module it imports. Use browser `fetch` through `web-sys`/`wasm-bindgen-futures`, normalize JSON in Rust, and let wasm-pack emit its declaration. Do not implement this SDK API as a hand-typed declaration in `index.d.ts.tmpl`.

Generated wasm surface:

```rust
#[wasm_bindgen]
pub struct DiscoveryDirectoryClient { /* base_url */ }

#[wasm_bindgen]
impl DiscoveryDirectoryClient {
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String) -> Result<DiscoveryDirectoryClient, JsValue>;

    #[wasm_bindgen(js_name = registerPeerJson)]
    pub async fn register_peer_json(&self, registration_json: String) -> Result<String, JsValue>;

    #[wasm_bindgen(js_name = discoverPeersJson)]
    pub async fn discover_peers_json(&self, query_json: String) -> Result<String, JsValue>;

    #[wasm_bindgen(js_name = listNodesJson)]
    pub async fn list_nodes_json(&self, query_json: String) -> Result<String, JsValue>;
}
```

The JSON method names intentionally mirror the native binding style. Generated package tests can provide a fake `globalThis.fetch`; implementation errors should include `Discovery HTTP <status>: <body>` for non-2xx responses.

- [ ] **Step 5: Add typed framed handler helpers**

Add typed helpers in `crates/auki-network/bindings/javascript/src/framed.ts` and wire them into the generated `AukiNetworkPeer` wrapper:

```js
async handleFramed(protocol, handler, options) {
  return this.handle(protocol, async ({ stream, connection }) => {
    const request = await readLengthPrefixed(stream);
    const response = await handler(request, { protocol, connection });
    if (response != null) {
      await writeLengthPrefixed(stream, response);
    }
  }, options);
}

async requestFramedJson(peerMultiaddr, protocol, requestJson = {}) {
  const payload = new TextEncoder().encode(JSON.stringify(requestJson ?? {}));
  const response = await this.requestFramed(peerMultiaddr, protocol, payload);
  return JSON.parse(new TextDecoder().decode(response));
}
```

Export `writeLengthPrefixed` and `readLengthPrefixed` only if tests need them; otherwise keep them private.

- [ ] **Step 6: Generate declarations and document the new surface**

Verify the generated package declaration output includes wasm-pack generated `DiscoveryDirectoryClient`, `registerPeerJson`, `discoverPeersJson`, `listNodesJson`, and `streamProtocol`. Verify JS transport adapter declarations include `handleFramed` and `requestFramedJson` from typed source emission. `index.d.ts.tmpl` may only re-export generated declarations during transition; it must not manually define this SDK surface.

Update `README.md.tmpl` with a browser rendezvous example that calls:

```js
const discovery = new DiscoveryDirectoryClient("http://127.0.0.1:8080");
const relays = JSON.parse(await discovery.listNodesJson(JSON.stringify({ type: "relay" }))).nodes;
```

- [ ] **Step 7: Verify**

Run:

```bash
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-network
npm --prefix bindings/javascript/auki-network test
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/auki-network bindings/javascript/auki-network
git commit -m "feat: add browser Discovery and framed protocol helpers"
```

---

## Task 3: Add Browser Domain Peer Core And Transport Adapter

**Files:**
- Modify: `crates/auki-domain/src/wasm.rs`
- Modify: `crates/auki-domain/bindings/javascript/index.js.tmpl`
- Add: `crates/auki-domain/bindings/javascript/src/browser-domain-peer.ts`
- Add: `crates/auki-domain/bindings/javascript/src/index.ts`
- Add: `crates/auki-domain/bindings/javascript/tsconfig.json`
- Modify: `crates/auki-domain/bindings/javascript/README.md.tmpl`
- Add: `crates/auki-domain/bindings/javascript/test/browser-domain-peer-state.test.mjs.tmpl`
- Add: `crates/auki-domain/bindings/javascript/test/browser-domain-peer-join.test.mjs.tmpl`

- [ ] **Step 1: Add failing state tests**

Create `browser-domain-peer-state.test.mjs.tmpl` proving that the wasm-pack generated core owns the state shape, and the JS transport adapter delegates to it:

```js
const core = new AukiBrowserDomainPeerCore(JSON.stringify({
  peer_id: "peer-a",
  app_id: "overwatch",
  display_name: "Browser A"
}));

assert.equal(JSON.parse(core.snapshotJson()).selfPeerId, "peer-a");

const peer = new AukiBrowserDomainPeer({
  networkPeer: fakeNetworkPeer("peer-a", ["/webrtc/p2p/peer-a"]),
  discovery: fakeDiscovery(),
  core,
  storage: new MapStorage(),
  appId: "overwatch",
  displayName: "Browser A",
});

assert.equal(peer.peerId, "peer-a");
assert.deepEqual(peer.debugState().advertisedMultiaddrs, ["/webrtc/p2p/peer-a"]);
assert.equal(peer.snapshot().participants[0].peer_id, "peer-a");
```

Expected failure: `AukiBrowserDomainPeerCore` is not emitted by wasm-pack yet and `AukiBrowserDomainPeer` does not delegate to it.

- [ ] **Step 2: Add failing create/join tests**

Create `browser-domain-peer-join.test.mjs.tmpl` with two fake network peers connected by in-memory framed protocol handlers:

```js
await managerPeer.createOrJoin({ discoveryUrl: "http://discovery", clusterName: "overwatch" });
await joinerPeer.createOrJoin({ discoveryUrl: "http://discovery", clusterName: "overwatch" });

assert.equal(managerPeer.snapshot().participants.length, 2);
assert.equal(joinerPeer.snapshot().participants.length, 2);
assert.equal(managerPeer.snapshot().managerPeerId, managerPeer.peerId);
assert.equal(joinerPeer.snapshot().managerPeerId, managerPeer.peerId);
```

The fake Discovery returns `201` for the first create and `409` plus the Manager cluster entry for the second.

- [ ] **Step 3: Implement wasm core plus typed transport constructor**

In `crates/auki-domain/src/wasm.rs`, add a `#[wasm_bindgen]` `AukiBrowserDomainPeerCore` for browser peer state transitions that can be shared across JS, Swift, Python, and future wasm consumers where practical.

Required generated methods:

```rust
#[wasm_bindgen]
impl AukiBrowserDomainPeerCore {
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: String) -> Result<AukiBrowserDomainPeerCore, JsValue>;

    #[wasm_bindgen(js_name = snapshotJson)]
    pub fn snapshot_json(&self) -> Result<String, JsValue>;

    #[wasm_bindgen(js_name = debugStateJson)]
    pub fn debug_state_json(&self) -> Result<String, JsValue>;

    #[wasm_bindgen(js_name = declareSensorsJson)]
    pub fn declare_sensors_json(&mut self, sensors_json: String) -> Result<String, JsValue>;

    #[wasm_bindgen(js_name = handleJoinRequestJson)]
    pub fn handle_join_request_json(&mut self, request_json: String) -> Result<String, JsValue>;
}
```

In `crates/auki-domain/bindings/javascript/src/browser-domain-peer.ts`, add a thin transport adapter that owns libp2p/discovery side effects and delegates state mutations to `AukiBrowserDomainPeerCore`:

```js
export class AukiBrowserDomainPeer {
  constructor({ networkPeer, discovery, core, storage, appId = "overwatch", displayName = "Overwatch Browser" } = {}) {
    if (!networkPeer) throw new TypeError("AukiBrowserDomainPeer requires networkPeer");
    if (!discovery) throw new TypeError("AukiBrowserDomainPeer requires discovery");
    this.networkPeer = networkPeer;
    this.discovery = discovery;
    this.core = core ?? new AukiBrowserDomainPeerCore(JSON.stringify({
      peer_id: networkPeer.peerId,
      app_id: appId,
      display_name: displayName,
      multiaddrs: networkPeer.multiaddrs ?? [],
    }));
    this.storage = storage ?? globalThis.localStorage;
    this.observers = new Set();
  }

  snapshot() {
    return JSON.parse(this.core.snapshotJson());
  }

  debugState() {
    return JSON.parse(this.core.debugStateJson());
  }
}
```

Snapshot shape:

```js
{
  selfPeerId: "peer-a",
  domainName: "overwatch",
  managerPeerId: "peer-a",
  role: "manager",
  participants: [
    {
      peer_id: "peer-a",
      app: "overwatch",
      name: "Browser A",
      is_self: true,
      is_manager: true,
      connected: true,
      sensors: []
    }
  ]
}
```

- [ ] **Step 4: Implement create-or-join**

`createOrJoin({ discoveryUrl, clusterName })` must:

1. Read `networkPeer.peerId` and `networkPeer.multiaddrs`.
2. POST `createCluster(clusterName, { manager_peer_id, manager_multiaddrs, relay_multiaddrs })`.
3. If created, initialize membership with `clusterMembershipNewJson(clusterName)` and admit self with `clusterMembershipAdmitMemberJson`.
4. If already exists, list clusters, find the matching name, request join from `manager_multiaddrs[0]`, validate returned membership, and refresh participant info/catalogs.
5. Register inbound handlers for join/info/sensors/resources/registries before announcing readiness.

- [ ] **Step 5: Implement Manager-side join handler**

Handle `/auki/join/0.0.1` through `networkPeer.handleFramed`. For a valid join request:

```json
{
  "peer_id": "peer-b",
  "multiaddrs": ["/webrtc/p2p/peer-b"],
  "participant_info_json": "{\"app\":\"overwatch\",\"name\":\"Browser B\",\"session_id\":\"session-b\",\"session_clock_id\":\"peer-b/session-b/monotonic\",\"session_clock_hash\":\"hash-b\",\"session_now_ns\":1,\"cluster_joined_at_ns\":1,\"peer_id\":\"peer-b\",\"app_instance\":\"browser-b\",\"is_manager\":false,\"manager_peer_id\":\"peer-a\"}"
}
```

Return:

```json
{
  "kind": "accept",
  "membership_json": "{\"cluster_name\":\"overwatch\",\"peers\":[{\"peer_id\":\"peer-a\",\"multiaddrs\":[\"/webrtc/p2p/peer-a\"],\"join_ts_ns\":1},{\"peer_id\":\"peer-b\",\"multiaddrs\":[\"/webrtc/p2p/peer-b\"],\"join_ts_ns\":2}]}",
  "manager_peer_id": "peer-a"
}
```

Reject with `{ "kind": "reject", "reason": "not_manager" }` when the local browser is not Manager.

- [ ] **Step 6: Verify generated package tests**

Run:

```bash
cargo check -p auki-domain --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-domain test
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/auki-domain bindings/javascript/auki-domain
git commit -m "feat: add browser Domain peer core"
```

---

## Task 4: Add Browser Catalog And Registry Protocol Handling

**Files:**
- Modify: `crates/auki-domain/src/wasm.rs`
- Modify: `crates/auki-domain/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-domain/bindings/javascript/src/browser-domain-peer.ts`
- Add: `crates/auki-domain/bindings/javascript/test/browser-domain-peer-catalogs.test.mjs.tmpl`

- [ ] **Step 1: Add failing catalog tests**

Add a generated-package test where a browser peer declares:

```js
await peer.declareSensors([{
  sensor_id: "browser-a/audio",
  sensor_hash: "audio-hash",
  kind: "audio",
  label: "Microphone",
}]);
```

Assert remote fetches return:

```json
{
  "sensors": [
    {
      "sensor_id": "browser-a/audio",
      "sensor_hash": "audio-hash",
      "kind": "audio"
    }
  ]
}
```

- [ ] **Step 2: Implement local catalog state in the wasm core**

Add or extend `AukiBrowserDomainPeerCore` methods:

```rust
#[wasm_bindgen(js_name = sensorCatalogJson)]
pub fn sensor_catalog_json(&self) -> Result<String, JsValue>;

#[wasm_bindgen(js_name = resourceCatalogJson)]
pub fn resource_catalog_json(&self) -> Result<String, JsValue>;
```

The JS adapter should only normalize app inputs and delegate state changes:

```js
declareSensors(sensors) {
  this.core.declareSensorsJson(JSON.stringify(sensors.map(normalizeSensor)));
  this.#emit();
}

sensorCatalogJson() {
  return this.core.sensorCatalogJson();
}
```

- [ ] **Step 3: Implement inbound catalog/resource handlers**

Register:

- `/auki/info/0.0.1` returns `{ participant_info_json }`.
- `/auki/sensors/0.0.1` returns `sensorCatalogJson()`.
- `/auki/resources/0.0.1` returns one `sensor_stream` resource per declared sensor.
- `/auki/registries/0.0.1` returns a registry entry only when the browser has a matching local registry JSON.

- [ ] **Step 4: Implement remote refresh**

After join/admit, fetch info and sensors from each member multiaddr. Merge participant display names and sensors into `participants`. Emit one observer snapshot per merge batch.

- [ ] **Step 5: Verify**

Run:

```bash
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-domain test
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-domain bindings/javascript/auki-domain
git commit -m "feat: serve browser peer catalogs"
```

---

## Task 5: Add Browser Stream MVP

**Files:**
- Modify: `crates/auki-network/src/wasm.rs`
- Modify: `crates/auki-network/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-network/bindings/javascript/src/framed.ts`
- Add: `crates/auki-network/bindings/javascript/test/stream-message-helpers.test.mjs.tmpl`
- Modify: `crates/auki-domain/src/wasm.rs`
- Modify: `crates/auki-domain/bindings/javascript/index.js.tmpl`
- Modify: `crates/auki-domain/bindings/javascript/src/browser-domain-peer.ts`
- Add: `crates/auki-domain/bindings/javascript/test/browser-domain-peer-stream.test.mjs.tmpl`

- [ ] **Step 1: Add failing stream helper tests**

Test expected exports:

```js
import {
  streamProtocol,
  encodeStreamRequestBytes,
  decodeStreamMessageJson,
} from "../index.js";

assert.equal(streamProtocol(), "/auki/stream/0.1.0");
const request = encodeStreamRequestBytes(JSON.stringify({ sensor_id: "browser-a/audio" }));
assert.equal(JSON.parse(decodeStreamMessageJson(request)).request.sensor_id, "browser-a/audio");
```

- [ ] **Step 2: Add wasm stream-message helpers**

In `crates/auki-network/src/wasm.rs`, use `auki_proto::stream::{StreamMessage, StreamRequest, StreamManifest, StreamEntry}` to add:

```rust
encodeStreamRequestBytes(json) -> Uint8Array
encodeStreamAcceptBytes(json) -> Uint8Array
encodeStreamEntryBytes(json) -> Uint8Array
decodeStreamMessageJson(bytes) -> string
streamProtocol() -> string
```

JSON byte fields must use integer arrays because existing wasm helpers already use that convention for `Vec<u8>`.

- [ ] **Step 3: Add browser stream methods on `AukiNetworkPeer`**

Add:

```js
async openStream(peerMultiaddr, requestJson) {
  const stream = await this.dialProtocol(await toMultiaddr(peerMultiaddr), wasm.streamProtocol());
  await writeLengthPrefixed(stream, wasm.encodeStreamRequestBytes(JSON.stringify(requestJson)));
  return new AukiBrowserStream(stream);
}
```

`AukiBrowserStream.nextMessage()` reads one framed `StreamMessage` and returns decoded JSON.

- [ ] **Step 4: Add Domain peer publish/subscribe MVP**

Keep publication metadata in `AukiBrowserDomainPeerCore` when it affects snapshots/catalogs, and keep live byte-source scheduling in the JS adapter because it is browser runtime behavior. In `AukiBrowserDomainPeer`, implement:

```js
publishSensor(sensorId, source) {
  this.publications.set(sensorId, source);
  this.#emit();
}

async subscribeToSensor(peerId, sensorId) {
  const participant = this.participants.get(peerId);
  const address = participant?.multiaddrs?.[0];
  if (!address) throw new Error(`no multiaddr for peer ${peerId}`);
  const stream = await this.networkPeer.openStream(address, { sensor_id: sensorId });
  return stream;
}
```

The first source type is a generated byte source for tests:

```js
{ kind: "generated-bytes", frames: [[1, 2, 3], [4, 5, 6]], interval_ms: 20 }
```

- [ ] **Step 5: Verify**

Run:

```bash
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network crates/auki-domain bindings/javascript/auki-network bindings/javascript/auki-domain
git commit -m "feat: add browser stream helpers"
```

---

## Task 6: Scaffold `examples/overwatch`

**Files:**
- Create: `examples/overwatch/README.md`
- Create: `examples/overwatch/package.json`
- Create: `examples/overwatch/index.html`
- Create: `examples/overwatch/tsconfig.json`
- Create: `examples/overwatch/vite.config.ts`
- Create: `examples/overwatch/vitest.config.ts`
- Create: `examples/overwatch/tailwind.config.ts`
- Create: `examples/overwatch/postcss.config.js`
- Create: `examples/overwatch/scripts/stage-sdk.mjs`
- Create: `examples/overwatch/src/main.tsx`
- Create: `examples/overwatch/src/App.tsx`
- Create: `examples/overwatch/src/index.css`
- Create: `examples/overwatch/src/sdk/createOverwatchPeer.ts`
- Create: `examples/overwatch/src/sdk/contract.ts`
- Create: `examples/overwatch/src/state/appState.ts`

- [ ] **Step 1: Create package metadata**

`examples/overwatch/package.json`:

```json
{
  "name": "auki-overwatch-example",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite --host 0.0.0.0 --port 7880",
    "build": "vite build",
    "test": "node scripts/stage-sdk.mjs && vitest run",
    "smoke": "node scripts/smoke-two-browser.mjs"
  },
  "dependencies": {
    "@aukilabs/auki-domain": "file:./sdk-generated/auki-domain",
    "@aukilabs/auki-network": "file:./sdk-generated/auki-network",
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.0.0",
    "@testing-library/user-event": "^14.5.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^4.3.0",
    "vite": "^5.4.0",
    "vitest": "^2.1.0",
    "happy-dom": "^20.9.0",
    "playwright": "^1.53.0",
    "tailwindcss": "^3.4.0",
    "postcss": "^8.4.0",
    "autoprefixer": "^10.4.0",
    "typescript": "^5.6.0"
  }
}
```

- [ ] **Step 2: Add React, Vite, Tailwind config**

`examples/overwatch/vite.config.ts`:

```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: { host: "0.0.0.0", port: 7880 },
});
```

`examples/overwatch/tailwind.config.ts`:

```ts
import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#101418",
        panel: "#171d22",
        line: "#29323a",
        signal: "#f97316",
      },
      borderRadius: {
        control: "8px",
      },
    },
  },
} satisfies Config;
```

`examples/overwatch/postcss.config.js`:

```js
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
```

- [ ] **Step 3: Add SDK staging script**

`examples/overwatch/scripts/stage-sdk.mjs` must:

1. Verify `bindings/javascript/auki-network/index.js` exists.
2. Verify `bindings/javascript/auki-domain/index.js` exists.
3. If missing, print:

```text
Run:
  just generate-javascript-bindings auki-network
  just generate-javascript-bindings auki-domain
```

4. Create `examples/overwatch/sdk-generated/auki-network` and `examples/overwatch/sdk-generated/auki-domain`.
5. Copy generated package files into those directories.

- [ ] **Step 4: Create docs**

`examples/overwatch/README.md` must say:

```markdown
# Overwatch

Web-only Auki SDK example.

Overwatch proves that a browser can act as an Auki Domain peer through generated SDK JavaScript/WASM bindings. It uses Discovery only for rendezvous and relay hints. It does not run an app backend.

Run from the repository root:

```bash
just overwatch
```
```

- [ ] **Step 5: Create direct generated-SDK peer entry**

`src/sdk/contract.ts` defines `OverwatchPeer` as an app-facing type over the generated SDK browser adapter. It must not introduce a fake peer implementation.

```ts
export type SensorSummary = {
  sensor_id: string;
  sensor_hash: string;
  kind: "audio" | "camera" | "point_cloud" | "joint_encoders" | "detection";
  label?: string;
};

export type SensorSource = {
  kind: "generated-bytes";
  frames: number[][];
  interval_ms: number;
};

export type StreamHandle = {
  nextMessage(): Promise<unknown>;
  close?(): Promise<void> | void;
};

export type PeerDebugState = Record<string, unknown>;

export type PeerSnapshot = {
  selfPeerId: string;
  domainName: string | null;
  managerPeerId: string | null;
  role: "manager" | "member" | "idle";
  participants: Array<{
    peer_id: string;
    name?: string;
    app?: string;
    is_self?: boolean;
    is_manager?: boolean;
    connected?: boolean;
    sensors?: SensorSummary[];
  }>;
};

export type OverwatchPeer = {
  readonly peerId: string;
  createOrJoin(input: { discoveryUrl: string; clusterName: string }): Promise<void>;
  observeParticipants(cb: (snapshot: PeerSnapshot) => void): () => void;
  declareSensors(sensors: SensorSummary[]): Promise<void>;
  publishSensor(sensorId: string, source: SensorSource): Promise<void>;
  subscribeToSensor(peerId: string, sensorId: string): Promise<StreamHandle>;
  debugState(): PeerDebugState;
};
```

`src/sdk/createOverwatchPeer.ts` imports generated SDK packages directly:

```ts
import initAukiNetwork, { createAukiNetworkPeer, DiscoveryDirectoryClient } from "@aukilabs/auki-network";
import initAukiDomain, { AukiBrowserDomainPeer } from "@aukilabs/auki-domain";
import type { OverwatchPeer } from "./contract";

export async function createOverwatchPeer(): Promise<OverwatchPeer> {
  await initAukiNetwork();
  await initAukiDomain();
  const walletSeed = loadOrMintWalletSeed();
  const networkPeer = await createAukiNetworkPeer({ walletSeed });
  return new AukiBrowserDomainPeer({
    networkPeer,
    discoveryFactory: (url: string) => new DiscoveryDirectoryClient(url),
    appId: "overwatch",
    displayName: browserDisplayName(networkPeer.peerId),
  }) as OverwatchPeer;
}

function loadOrMintWalletSeed(): Uint8Array {
  const key = "auki:overwatch:wallet-seed:v1";
  const existing = globalThis.localStorage?.getItem(key);
  if (existing) return Uint8Array.from(existing.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16));
  const seed = globalThis.crypto.getRandomValues(new Uint8Array(32));
  globalThis.localStorage?.setItem(key, Array.from(seed, (byte) => byte.toString(16).padStart(2, "0")).join(""));
  return seed;
}

function browserDisplayName(peerId: string): string {
  return `Browser ${peerId.slice(-6)}`;
}
```

- [ ] **Step 6: Create the React shell UI**

The first screen must be the usable app, not a landing page:

- top bar: Auki wordmark text, Domain chip, peer id, SDK status
- join/create form: Discovery URL and Domain name
- directory: self card plus remote peer cards
- empty state: "No remote peers"

Use React components and Tailwind utility classes. Styling should be restrained operator UI: dark ink surface, off-white text, orange accent, dense spacing, 8px or smaller radii.

- [ ] **Step 7: Verify**

Run:

```bash
node examples/overwatch/scripts/stage-sdk.mjs
npm --prefix examples/overwatch install
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add examples/overwatch
git commit -m "feat: scaffold Overwatch browser example"
```

---

## Task 7: Build The Park-Inspired Operator Surface

**Files:**
- Create: `examples/overwatch/src/components/Directory.tsx`
- Create: `examples/overwatch/src/components/PeerDetail.tsx`
- Create: `examples/overwatch/src/components/Stage.tsx`
- Create: `examples/overwatch/src/components/SensorStrip.tsx`
- Create: `examples/overwatch/src/components/Tiles.tsx`
- Create: `examples/overwatch/src/components/QuickSearch.tsx`
- Modify: `examples/overwatch/src/App.tsx`
- Modify: `examples/overwatch/src/index.css`
- Add: `examples/overwatch/src/components/Stage.test.tsx`
- Add: `examples/overwatch/src/components/Directory.test.tsx`

- [ ] **Step 1: Add failing UI tests**

Test stage layout with React Testing Library:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Stage } from "./Stage";

it("renders one tile per toggled sensor and closes tiles independently", async () => {
  render(<Stage tiles={[
    { kind: "camera", peerId: "peer-a", sensorId: "camera" },
    { kind: "audio", peerId: "peer-a", sensorId: "audio" },
  ]} />);

  expect(screen.getAllByTestId("stage-tile")).toHaveLength(2);
  await userEvent.click(screen.getByRole("button", { name: /close audio/i }));
  expect(screen.getAllByTestId("stage-tile")).toHaveLength(1);
});
```

Test directory cards:

```tsx
import { render, screen } from "@testing-library/react";
import { Directory } from "./Directory";

it("shows self, remotes, manager state, and stream health", () => {
  render(<Directory snapshot={snapshotWithTwoPeers()} />);
  expect(screen.getByText("you")).toBeInTheDocument();
  expect(screen.getByText("Manager")).toBeInTheDocument();
  expect(screen.getByText("browser-a/audio")).toBeInTheDocument();
});
```

- [ ] **Step 2: Implement directory view**

Cards show:

- display name
- app id
- short peer id
- connected/offline
- Manager pill
- sensor count
- last stream frame state

- [ ] **Step 3: Implement peer detail view**

Layout:

```text
topbar
sidebar | stage
sensors strip across bottom
```

Sidebar shows peer identity, role, connection state, advertised addresses, and local/remote sensor catalog facts.

- [ ] **Step 4: Implement stage and tiles**

Stage layout rules:

- 0 tiles: centered empty state
- 1 tile: full stage
- 2 tiles: two columns
- 3-4 tiles: 2x2 grid
- 5+ tiles: auto-fit minmax grid

Tile controls:

- pause/resume
- close
- snapshot button that downloads a JSON/text debug snapshot for non-visual tiles

- [ ] **Step 5: Implement sensor strip**

Each sensor is a toggle. The first visual sensor toggles on automatically when opening a peer for the first time. Persist toggles under key:

```text
auki:overwatch:toggled:v1:<peer-id>
```

- [ ] **Step 6: Implement quick search**

`Ctrl+K` / `Cmd+K` opens a modal listing peers and static actions. Selecting a peer opens peer detail.

- [ ] **Step 7: Verify**

Run:

```bash
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add examples/overwatch
git commit -m "feat: add Overwatch operator UI"
```

---

## Task 8: Add `just overwatch` Commands

**Files:**
- Modify: `examples/overwatch/README.md`
- Modify: `justfile`

- [ ] **Step 1: Add root commands**

Add to `justfile`:

```just
overwatch:
    node examples/overwatch/scripts/stage-sdk.mjs
    npm --prefix examples/overwatch install
    npm --prefix examples/overwatch run dev

overwatch-smoke:
    node examples/overwatch/scripts/stage-sdk.mjs
    npm --prefix examples/overwatch install
    npm --prefix examples/overwatch run smoke
```

- [ ] **Step 2: Update README runbook**

`examples/overwatch/README.md` must state that `just overwatch` stages the generated SDK packages, installs the React app dependencies, and starts the Vite dev server. It must also state that the app imports generated SDK bindings directly and does not use a fake peer.

- [ ] **Step 3: Verify**

Run:

```bash
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
node examples/overwatch/scripts/stage-sdk.mjs
npm --prefix examples/overwatch install
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add examples/overwatch/README.md justfile
git commit -m "feat: add Overwatch just commands"
```

---

## Task 9: Add Two-Browser Acceptance Smoke

**Files:**
- Add: `examples/overwatch/scripts/smoke-two-browser.mjs`
- Modify: `examples/overwatch/package.json`
- Modify: `examples/overwatch/README.md`

- [ ] **Step 1: Write failing smoke script**

`smoke-two-browser.mjs` must:

1. Start Discovery from `/Users/jb/Developer/Aukilabs/repos/discovery` on `127.0.0.1:8091`.
2. Start Vite on `127.0.0.1:7880`.
3. Open two Playwright Chromium browser contexts.
4. Join both browsers to Domain `overwatch-smoke`.
5. Wait until each sees two participants.
6. Declare generated byte sensor on peer A.
7. Subscribe peer B to peer A.
8. Assert peer B receives at least one frame.
9. Assert no request URL contains `/api/`.
10. Stop Vite and Discovery.

The failure message for reachability must include:

```text
Browser peer reachability failed through SDK networking. Do not add an Overwatch backend; fix SDK transport or add a generic Discovery signaling design.
```

- [ ] **Step 2: Run and classify failure**

Run:

```bash
just overwatch-smoke
```

Expected at this point: either PASS if libp2p browser reachability is sufficient, or FAIL with the explicit reachability message.

If the failure is missing SDK methods, return to Tasks 2-5.

If the failure is browser reachability after SDK methods exist, stop this plan and write a separate Discovery signaling plan. Do not implement app-specific signaling in Overwatch.

- [ ] **Step 3: Commit passing smoke**

Only commit this task when the smoke passes.

```bash
git add examples/overwatch
git commit -m "test: add Overwatch two-browser smoke"
```

---

## Task 10: Documentation And Stability Contract

**Files:**
- Modify: `README.md`
- Modify: `examples/overwatch/README.md`
- Modify: `crates/auki-network/README.md`
- Modify: `crates/auki-domain/README.md`
- Modify: `crates/auki-network/src/sprint.md`
- Modify: `crates/auki-domain/src/sprint.md`

- [ ] **Step 1: Document the public SDK contract**

In root `README.md`, add Overwatch to the example index and state:

```markdown
Overwatch is the browser-only acceptance example for the generated JavaScript/WASM Domain peer surface. It must not depend on app-specific backend APIs.
```

In `crates/auki-domain/README.md`, document `AukiBrowserDomainPeerCore` as the wasm-pack generated browser Domain peer state surface and `AukiBrowserDomainPeer` as the JS transport adapter.

In `crates/auki-network/README.md`, document `DiscoveryDirectoryClient`, `AukiNetworkPeer.handleFramed`, and stream helpers.

- [ ] **Step 2: Document acceptance commands**

Add final verification list:

```bash
cargo test -p auki-network -F discovery_client
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p auki-domain --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
just overwatch-smoke
```

- [ ] **Step 3: Update sprint files**

`auki-network/src/sprint.md` should name the next transport hardening item after Overwatch:

```markdown
Browser reachability hardening: if Overwatch smoke required a relay or signaling fallback, move that work into SDK transport/Discovery, not app code.
```

`auki-domain/src/sprint.md` should name the browser peer core plus JS transport adapter as current implementation status.

- [ ] **Step 4: Commit**

```bash
git add README.md crates/auki-network crates/auki-domain examples/overwatch
git commit -m "docs: document Overwatch browser peer contract"
```

---

## Verification Matrix

Required before calling the work complete:

```bash
cargo test -p auki-network -F discovery_client
cargo check -p auki-network --target wasm32-unknown-unknown --no-default-features --features wasm
cargo check -p auki-domain --target wasm32-unknown-unknown --no-default-features --features wasm
just generate-javascript-bindings auki-network
just generate-javascript-bindings auki-domain
npm --prefix bindings/javascript/auki-network test
npm --prefix bindings/javascript/auki-domain test
npm --prefix examples/overwatch test
npm --prefix examples/overwatch run build
just overwatch-smoke
```

Expected final result: all commands pass. `just overwatch-smoke` proves two browser contexts join through Discovery, see each other through the SDK browser Domain peer core/adapter, exchange at least one SDK stream frame, and make no `/api` backend calls.

## Self-Review

- Spec coverage: The plan covers Discovery as rendezvous, relay/node hint exposure, wasm-pack SDK surface ownership, JS transport adapter ownership, stream MVP, Overwatch UI, `just overwatch`, and two-browser acceptance.
- Placeholder scan: No step uses open-ended `TBD`, `TODO`, or future fill-in language. The only conditional path is explicit: if SDK browser reachability fails after methods exist, stop Overwatch work and write a separate generic Discovery signaling plan.
- Type consistency: `AukiNetworkPeer`, `DiscoveryDirectoryClient`, `AukiBrowserDomainPeerCore`, `AukiBrowserDomainPeer`, `OverwatchPeer`, `PeerSnapshot`, and `SensorSummary` are named consistently across SDK and example tasks.
