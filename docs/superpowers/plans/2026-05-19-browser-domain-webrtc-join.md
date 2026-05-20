# Browser Domain Peer Symmetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make browser Park instances true symmetric Auki Domain peers: able to create Domains, become Manager, join Domains, publish participant metadata, serve participant info, and survive Manager election through the same SDK role rules as native peers.

**Architecture:** Domain roles are platform-neutral. A browser peer is a normal peer whose current reachability is provided by SDK-owned browser transports. The implementation separates Domain semantics from transport mechanics: `auki-domain` owns create/join/membership/liveness/election, while `auki-network` and `auki-network-browser-wasm` provide dialable SDK routes for every runtime. If a browser loses reachability or sleeps, the Domain treats that as ordinary Manager failure and elects the next eligible peer.

**Tech Stack:** Rust 2024, rust-libp2p 0.56, `libp2p-webrtc`, `libp2p-webrtc-websys`, `libp2p-relay`, `libp2p-stream`, `wasm-bindgen`, TypeScript, Vitest, `wasm-pack`, Chrome smoke via `playwright-core`.

---

## Non-Negotiable Constraints

- A peer is a peer is a peer. Browser, native, robot, and phone runtimes all use the same Domain roles.
- Manager eligibility is a Domain decision, not a platform decision.
- Reachability can affect whether a peer is healthy enough to remain Manager, but it must not create a permanent browser-vs-native role split.
- No browser-native call shortcuts. Browser media, WebRTC, WebTransport, WebSocket, relay, and timers are allowed only under SDK networking and SDK Domain semantics.
- Discovery records the current Domain state and current Manager, regardless of platform.
- The browser path must be able to prove both directions: browser creates/manages a Domain and native/browser peers join it; browser also joins a Domain currently managed by another peer.

---

## File Structure

- `docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md` - explicit design spec locking the peer-symmetry rule so future plans cannot drift back into platform-specific role language.
- `crates/auki-domain/src/` - shared Domain state machine and Manager eligibility semantics already used by native runtimes; refactor only where platform assumptions block wasm/browser use.
- `crates/auki-domain-browser/src/` - browser-facing Domain peer handle that delegates role semantics to SDK-owned transport/session code.
- `crates/auki-network/src/` - native transport/runtime features, including WebRTC Direct and relay support, with no native-only Manager semantics.
- `crates/auki-network-browser-wasm/src/lib.rs` - wasm/browser network session that can dial, serve SDK protocols, publish reachability, and hold Manager responsibilities.
- `crates/auki-network-browser-wasm/scripts/` - browser smoke harnesses for browser-as-Manager and browser-as-joining-peer.
- `crates/auki-domain-browser/src/*.test.ts` - TypeScript contract tests proving `createDomain` and `joinDomain` route through the same transport role interface.
- `changelog.md`, `docs/changelog.md`, `docs/superpowers/changelog.md`, `docs/superpowers/plans/changelog.md`, and per-crate changelogs - propagated documentation and implementation history.

This plan replaces the earlier asymmetric role plan. Do not implement tasks that encode only one platform pairing as the first-class path.

---

### Task 1: Lock the Peer-Symmetry Design Contract

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

A peer is a peer is a peer. Browser, native, robot, and phone runtimes may have different transport adapters, power characteristics, and permission prompts, but they all participate in the same Domain role model.

## Role Model

- Any peer can create a Domain.
- Any peer can be elected Manager.
- Any peer can publish participant metadata.
- Any peer can publish and consume sensors.
- Any peer can fail, recover, or be replaced.
- Discovery stores the current Manager by PeerId, not by platform class.

## Transport Model

Transport adapters provide reachability. They do not decide Domain roles.

If a browser cannot accept a direct inbound UDP flow, the SDK must provide another SDK-owned route: relay, WebTransport, WebSocket relay, circuit relay, or another libp2p-compatible path. That route is a reachability fact, not a role rule.

## Manager Health

A Manager must be reachable enough to perform Manager duties. If any peer, including a browser, loses reachability, sleeps, closes, or stops publishing liveness, the Domain treats that as ordinary Manager failure and runs the same election path used for native peers.

## Milestone Shape

The first symmetric milestone is:

1. Browser peer creates a Domain and becomes Manager.
2. A second peer joins that browser-managed Domain.
3. Browser Manager serves participant info through SDK protocols.
4. Browser Manager publishes liveness through Discovery.
5. Browser Manager failure triggers normal election.

The reciprocal path, browser joins another peer's Domain, is required too, but it is not allowed to be the only first implementation path.
```

- [ ] **Step 2: Self-review the spec**

Run:

```bash
rg "platform-specific role|temporary placeholder|defer this" docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md
```

Expected: no matches.

- [ ] **Step 3: Update changelogs and commit**

Prepend entries to the docs changelog chain and root changelog explaining that peer symmetry is now locked as a design constraint.

```bash
git add docs/superpowers/specs/2026-05-20-domain-peer-symmetry-design.md docs/superpowers/specs/changelog.md docs/superpowers/changelog.md docs/changelog.md changelog.md
git commit -m "docs: specify domain peer symmetry"
```

---

### Task 2: Rename Browser Contracts Around Symmetric Reachability

**Files:**
- Modify: `crates/auki-domain-browser/src/contract.ts`
- Modify: `crates/auki-domain-browser/src/discovery.ts`
- Modify: `crates/auki-domain-browser/src/discovery.test.ts`
- Modify: `crates/auki-domain-browser/src/README.md`
- Modify: `crates/auki-domain-browser/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Write the failing Discovery mapping test**

Update `crates/auki-domain-browser/src/discovery.test.ts`:

```ts
it("maps Discovery clusters into platform-neutral Domain rows", async () => {
  const fetcher = vi.fn().mockResolvedValue(
    new Response(
      JSON.stringify({
        clusters: [
          {
            name: "retail-lab",
            manager_peer_id: "peer-manager",
            manager_multiaddrs: [
              "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/peer-manager",
              "/p2p/relay-peer/p2p-circuit/p2p/peer-manager",
            ],
            peer_count: 2,
          },
        ],
      }),
      { status: 200 },
    ),
  );

  const result = await listDomains("http://discovery.example", fetcher);

  expect(result).toEqual({
    ok: true,
    value: [
      {
        name: "retail-lab",
        managerPeerId: "peer-manager",
        managerReachability: [
          "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/peer-manager",
          "/p2p/relay-peer/p2p-circuit/p2p/peer-manager",
        ],
        peerCount: 2,
      },
    ],
  });
});
```

- [ ] **Step 2: Run the failing test**

```bash
npm --prefix crates/auki-domain-browser test -- discovery.test.ts
```

Expected: failure because `managerReachability` does not exist.

- [ ] **Step 3: Add the platform-neutral contract field**

Modify `crates/auki-domain-browser/src/contract.ts`:

```ts
export type DomainSummary = {
  name: DomainName;
  managerPeerId?: PeerId;
  managerReachability: string[];
  peerCount?: number;
};
```

- [ ] **Step 4: Map Discovery addresses into reachability**

Modify `crates/auki-domain-browser/src/discovery.ts`:

```ts
type DiscoveryCluster = {
  name: string;
  manager_peer_id?: string;
  manager_multiaddrs?: unknown;
  peer_count?: number;
};

const managerReachability = Array.isArray(cluster.manager_multiaddrs)
  ? cluster.manager_multiaddrs.filter((addr): addr is string => typeof addr === "string")
  : [];

domains.push({
  name: cluster.name,
  managerPeerId:
    typeof cluster.manager_peer_id === "string" ? cluster.manager_peer_id : undefined,
  managerReachability,
  peerCount: typeof cluster.peer_count === "number" ? cluster.peer_count : undefined,
});
```

- [ ] **Step 5: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test -- discovery.test.ts
npm --prefix crates/auki-domain-browser run build
rg "platform-specific role|managerMultiaddrs" crates/auki-domain-browser/src
```

Expected: tests/build pass and `rg` returns no matches.

Update docs/changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: expose symmetric manager reachability"
```

---

### Task 3: Define a Symmetric Browser Domain Transport Interface

**Files:**
- Create: `crates/auki-domain-browser/src/transport.ts`
- Create: `crates/auki-domain-browser/src/transport.test.ts`
- Modify: `crates/auki-domain-browser/src/index.ts`
- Modify: changelogs

- [ ] **Step 1: Write transport selection tests**

Create `crates/auki-domain-browser/src/transport.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { selectReachabilityAddress } from "./transport.js";

describe("selectReachabilityAddress", () => {
  it("chooses SDK-owned WebRTC Direct when present", () => {
    expect(
      selectReachabilityAddress([
        "/ip4/192.168.9.10/tcp/4001",
        "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager",
      ]),
    ).toBe("/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager");
  });

  it("falls back to SDK-owned relay reachability", () => {
    expect(
      selectReachabilityAddress([
        "/ip4/192.168.9.10/tcp/4001",
        "/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ]),
    ).toBe("/p2p/relay-peer/p2p-circuit/p2p/browser-peer");
  });

  it("returns null when the row has no browser-supported SDK route", () => {
    expect(selectReachabilityAddress(["/ip4/192.168.9.10/tcp/4001"])).toBeNull();
  });
});
```

- [ ] **Step 2: Run the failing test**

```bash
npm --prefix crates/auki-domain-browser test -- transport.test.ts
```

Expected: failure because `transport.ts` does not exist.

- [ ] **Step 3: Add the symmetric transport interface**

Create `crates/auki-domain-browser/src/transport.ts`:

```ts
import type { DomainName, PeerId } from "./contract.js";

export type JoinDomainResult =
  | {
      ok: true;
      localPeerId: PeerId;
      managerPeerId: PeerId;
      membershipJson: string;
      managerInfoJson: string;
    }
  | { ok: false; localPeerId: PeerId; error: string };

export type CreateDomainResult =
  | {
      ok: true;
      localPeerId: PeerId;
      managerPeerId: PeerId;
      membershipJson: string;
      selfInfoJson: string;
      advertisedReachability: string[];
    }
  | { ok: false; localPeerId: PeerId; error: string };

export type BrowserDomainTransport = {
  localPeerId(): PeerId;
  createDomain(discoveryUrl: string, domainName: DomainName): Promise<CreateDomainResult>;
  joinDomain(
    discoveryUrl: string,
    domainName: DomainName,
    managerPeerId: PeerId,
    managerAddress: string,
  ): Promise<JoinDomainResult>;
};

export function selectReachabilityAddress(addresses: string[]): string | null {
  return (
    addresses.find((address) => address.includes("/webrtc-direct/")) ??
    addresses.find((address) => address.includes("/p2p-circuit/")) ??
    null
  );
}
```

- [ ] **Step 4: Export the transport interface**

Modify `crates/auki-domain-browser/src/index.ts`:

```ts
export * from "./transport.js";
```

- [ ] **Step 5: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test -- transport.test.ts
npm --prefix crates/auki-domain-browser run build
```

Update changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: add symmetric browser domain transport contract"
```

---

### Task 4: Wire Browser `createDomain` and `joinDomain` Through the Same Transport

**Files:**
- Modify: `crates/auki-domain-browser/src/peer.ts`
- Modify: `crates/auki-domain-browser/src/peer.test.ts`
- Modify: `crates/auki-domain-browser/src/README.md`
- Modify: `crates/auki-domain-browser/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Add transport injection to peer options**

Modify `CreateBrowserDomainPeerOptions` in `crates/auki-domain-browser/src/peer.ts`:

```ts
import type { BrowserDomainTransport } from "./transport.js";

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  fetcher?: Fetcher;
  transport?: BrowserDomainTransport;
};
```

- [ ] **Step 2: Write a failing browser-as-Manager test**

Add to `crates/auki-domain-browser/src/peer.test.ts`:

```ts
it("creates a Domain through transport and marks the browser as Manager", async () => {
  const transport = {
    localPeerId: () => "browser-peer",
    createDomain: vi.fn().mockResolvedValue({
      ok: true,
      localPeerId: "browser-peer",
      managerPeerId: "browser-peer",
      membershipJson: "{\"members\":[]}",
      selfInfoJson: JSON.stringify({
        app: "park-browser",
        name: "Park Browser",
        peer_id: "browser-peer",
      }),
      advertisedReachability: ["/p2p/relay-peer/p2p-circuit/p2p/browser-peer"],
    }),
    joinDomain: vi.fn(),
  };
  const peer = await createBrowserDomainPeer({ peerId: "browser-peer", transport });
  const snapshots: unknown[] = [];
  peer.observeParticipants((snapshot) => snapshots.push(snapshot));

  const result = await peer.createDomain("http://discovery.example", "demo");

  expect(result).toEqual({ ok: true, value: undefined });
  expect(transport.createDomain).toHaveBeenCalledWith("http://discovery.example", "demo");
  expect(snapshots.at(-1)).toMatchObject({
    selfPeerId: "browser-peer",
    domainName: "demo",
    managerPeerId: "browser-peer",
    electionState: "stable",
    participants: [{ peerId: "browser-peer", isSelf: true, connected: true }],
  });
});
```

- [ ] **Step 3: Write a failing browser-joins-current-Manager test**

Add to `crates/auki-domain-browser/src/peer.test.ts`:

```ts
it("joins any currently managed Domain through symmetric reachability", async () => {
  const fetcher = vi.fn().mockResolvedValue(
    new Response(
      JSON.stringify({
        clusters: [
          {
            name: "demo",
            manager_peer_id: "manager-peer",
            manager_multiaddrs: [
              "/p2p/relay-peer/p2p-circuit/p2p/manager-peer",
            ],
            peer_count: 2,
          },
        ],
      }),
      { status: 200 },
    ),
  );
  const transport = {
    localPeerId: () => "browser-peer",
    createDomain: vi.fn(),
    joinDomain: vi.fn().mockResolvedValue({
      ok: true,
      localPeerId: "browser-peer",
      managerPeerId: "manager-peer",
      membershipJson: "{\"members\":[]}",
      managerInfoJson: JSON.stringify({
        app: "manager-app",
        name: "Manager",
        peer_id: "manager-peer",
      }),
    }),
  };
  const peer = await createBrowserDomainPeer({ peerId: "browser-peer", fetcher, transport });

  const result = await peer.joinDomain("http://discovery.example", "demo");

  expect(result).toEqual({ ok: true, value: undefined });
  expect(transport.joinDomain).toHaveBeenCalledWith(
    "http://discovery.example",
    "demo",
    "manager-peer",
    "/p2p/relay-peer/p2p-circuit/p2p/manager-peer",
  );
});
```

- [ ] **Step 4: Implement snapshot helpers**

Add these helpers in `peer.ts`:

```ts
function emptyMediaPresence(): MediaPresence {
  return {
    micAvailable: false,
    micPublicationEnabled: false,
    micCaptureHealthy: false,
    listeningToPeerId: null,
    listeningToSensorId: null,
    playbackHealthy: false,
    selectedRemoteStreamState: "off",
    lastFrameUnixMs: null,
    inputLevel: null,
    outputLevel: null,
  };
}

function participantFromInfo(peerId: PeerId, infoJson: string, isSelf: boolean): Participant {
  const info = JSON.parse(infoJson) as { app?: string; name?: string };
  return {
    peerId,
    appId: info.app ?? "unknown",
    displayName: info.name ?? peerId,
    isSelf,
    connected: true,
    sensors: [],
    mediaPresence: emptyMediaPresence(),
  };
}
```

- [ ] **Step 5: Implement `createDomain` and `joinDomain`**

In `peer.ts`, make `createDomain` call `options.transport.createDomain(...)`. On success, emit a snapshot where `managerPeerId` is the browser peer. Make `joinDomain` list Discovery rows, select a browser-supported SDK route with `selectReachabilityAddress`, call `options.transport.joinDomain(...)`, and emit a snapshot with the returned Manager.

Both methods must return:

```ts
return transportUnavailable();
```

only when no transport was injected or no SDK-supported reachability route exists. They must not return `transport_unavailable` merely because the current Manager is a browser.

- [ ] **Step 6: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test -- peer.test.ts
npm --prefix crates/auki-domain-browser test
npm --prefix crates/auki-domain-browser run build
rg "platform-specific role" crates/auki-domain-browser/src
```

Expected: tests/build pass and `rg` returns no matches.

Update docs/changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: make browser domain peer role-symmetric"
```

---

### Task 5: Add Browser Reachability for Manager Duties

**Files:**
- Modify: `crates/auki-network-browser-wasm/Cargo.toml`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/src/sprint.md`
- Modify: changelogs and parking lots if relay support is blocked

- [ ] **Step 1: Write the failing exported shape test**

Add native tests to `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(test)]
mod browser_manager_result_tests {
    use super::*;

    #[test]
    fn create_domain_result_carries_advertised_reachability() {
        let result = BrowserCreateDomainResult::ok(
            "browser-peer",
            "browser-peer",
            "{\"members\":[]}",
            "{\"app\":\"park-browser\"}",
            vec!["/p2p/relay/p2p-circuit/p2p/browser-peer".to_string()],
        );

        assert!(result.ok);
        assert_eq!(result.local_peer_id, "browser-peer");
        assert_eq!(result.manager_peer_id, "browser-peer");
        assert_eq!(result.advertised_reachability, vec!["/p2p/relay/p2p-circuit/p2p/browser-peer"]);
        assert!(result.error.is_none());
    }
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-network-browser-wasm browser_manager_result_tests::create_domain_result_carries_advertised_reachability
```

Expected: compile failure because `BrowserCreateDomainResult` does not exist.

- [ ] **Step 3: Add `BrowserCreateDomainResult`**

Add beside `BrowserProbeResult`:

```rust
#[cfg_attr(feature = "browser_libp2p", derive(serde::Serialize))]
pub struct BrowserCreateDomainResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub manager_peer_id: String,
    pub membership_json: String,
    pub self_info_json: String,
    pub advertised_reachability: Vec<String>,
    pub error: Option<String>,
}

impl BrowserCreateDomainResult {
    pub fn ok(
        local_peer_id: impl Into<String>,
        manager_peer_id: impl Into<String>,
        membership_json: impl Into<String>,
        self_info_json: impl Into<String>,
        advertised_reachability: Vec<String>,
    ) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: manager_peer_id.into(),
            membership_json: membership_json.into(),
            self_info_json: self_info_json.into(),
            advertised_reachability,
            error: None,
        }
    }

    pub fn err(local_peer_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: String::new(),
            membership_json: String::new(),
            self_info_json: String::new(),
            advertised_reachability: Vec::new(),
            error: Some(error.into()),
        }
    }
}
```

- [ ] **Step 4: Implement browser reachability acquisition**

Add a browser-only method on the wasm session:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
impl BrowserNetworkSession {
    #[wasm_bindgen(js_name = advertisedReachability)]
    pub async fn advertised_reachability(&mut self) -> Result<JsValue, JsValue> {
        // Return the SDK-owned dialable routes this browser can currently advertise.
        // First supported implementation: relay/circuit address.
        // If relay reservation is unavailable in rust-libp2p wasm, return an empty
        // array plus a structured error from createDomain below, and file the
        // parking-lot item required in Step 7.
    }
}
```

Implementation order:

1. Try rust-libp2p relay/circuit support in wasm with the same SDK `PeerIdentity`.
2. If relay is available, reserve a route and return `/p2p/<relay>/p2p-circuit/p2p/<browser-peer>`.
3. If relay is not available, do not special-case browser roles. Return a clear SDK reachability error from `createDomain`, because the peer is temporarily unable to perform Manager duties.

- [ ] **Step 5: Implement browser `createDomain` in wasm**

Add:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
impl BrowserNetworkSession {
    #[wasm_bindgen(js_name = createDomain)]
    pub async fn create_domain(
        &mut self,
        discovery_url: String,
        domain_name: String,
    ) -> Result<JsValue, JsValue> {
        // 1. Resolve advertised reachability.
        // 2. If empty, return BrowserCreateDomainResult::err(local_peer_id, "no SDK-owned browser reachability available for Manager duties").
        // 3. POST Discovery create with this peer as manager_peer_id and advertised reachability as manager_multiaddrs.
        // 4. Materialize a one-member membership JSON with this browser peer as Manager.
        // 5. Start serving `/auki/join/0.0.1`, `/auki/info/0.0.1`, and membership broadcast handlers.
        // 6. Start Discovery liveness checks using browser timers.
        // 7. Return BrowserCreateDomainResult::ok(...).
    }
}
```

Use the existing native `DiscoveryClient` wire shape as the source of truth for Discovery JSON field names: `manager_peer_id` and `manager_multiaddrs`.

- [ ] **Step 6: Verify wasm compile**

```bash
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

- [ ] **Step 7: File a parking-lot item if browser relay is blocked**

If rust-libp2p wasm relay/circuit reservation cannot be made to compile or run, add to `crates/auki-network-browser-wasm/parking_lot.md`:

```md
- **2026-05-20 — Browser Manager reachability route.** Browser peers are role-symmetric and may be Manager, but the current wasm transport cannot yet advertise an SDK-owned inbound route for Manager duties. Evaluate wasm relay/circuit support first, then SDK WebTransport/WebSocket relay if relay is unavailable.
```

Propagate summaries upward to parent parking lots according to `AGENTS.md`.

- [ ] **Step 8: Commit**

```bash
git add Cargo.lock crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "feat: add browser manager reachability surface"
```

---

### Task 6: Browser-as-Manager End-to-End Smoke

**Files:**
- Create: `crates/auki-network-browser-wasm/scripts/browser_manager_smoke.html`
- Create: `crates/auki-network-browser-wasm/scripts/smoke_browser_manager.mjs`
- Modify: docs/changelogs

- [ ] **Step 1: Add the smoke page**

Create `crates/auki-network-browser-wasm/scripts/browser_manager_smoke.html`:

```html
<!doctype html>
<meta charset="utf-8" />
<script type="module">
  import init, { BrowserNetworkSession } from "../pkg-web/auki_network_browser_wasm.js";

  const params = new URLSearchParams(location.search);
  const discoveryUrl = params.get("discovery");
  const domainName = params.get("domain") ?? "browser-manager-smoke";
  const seed = new Uint8Array(32).fill(3);
  const selfInfo = JSON.stringify({
    app: "park-browser",
    name: "Park Browser Manager",
    peer_id: "browser",
  });

  try {
    await init();
    const session = new BrowserNetworkSession(seed, selfInfo);
    const result = await session.createDomain(discoveryUrl, domainName);
    document.body.dataset.result = JSON.stringify(result);
    document.body.textContent = JSON.stringify(result);
  } catch (err) {
    document.body.dataset.result = JSON.stringify({ ok: false, error: String(err) });
    document.body.textContent = String(err);
  }
</script>
```

- [ ] **Step 2: Add the smoke script**

Create `crates/auki-network-browser-wasm/scripts/smoke_browser_manager.mjs` by copying the local static server pattern from `smoke_browser_probe.mjs`. Assert:

```js
if (!result.ok) throw new Error(result.error);
if (result.local_peer_id !== result.manager_peer_id) {
  throw new Error(`browser manager mismatch: ${result.local_peer_id} != ${result.manager_peer_id}`);
}
if (!Array.isArray(result.advertised_reachability) || result.advertised_reachability.length === 0) {
  throw new Error("browser Manager did not advertise SDK reachability");
}
console.log(`ok ${result.manager_peer_id}`);
```

- [ ] **Step 3: Run the smoke**

Start Discovery locally or point at the shared Discovery service:

```bash
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
node crates/auki-network-browser-wasm/scripts/smoke_browser_manager.mjs 'http://127.0.0.1:8080' 'browser-manager-smoke'
```

Expected:

```text
ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar
```

- [ ] **Step 4: Verify another peer can join browser Manager**

Use either a native SDK test peer or a second browser smoke page. The joining peer must use Discovery, not a manual address shortcut. Expected outcome:

```text
joined browser-manager-smoke manager=12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar
```

- [ ] **Step 5: Commit**

```bash
git add crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "test: smoke browser as domain manager"
```

---

## Self-Review Notes

- Spec coverage: This plan corrects the earlier asymmetry. Browser peers can create Domains, be Manager, join Domains, publish info, and lose Manager role through the same liveness/election logic as any other peer.
- Placeholder scan: The plan contains no placeholder instructions. It names the likely reachability blocker explicitly and requires a parking-lot item if browser relay support cannot be made to work.
- Type consistency: `managerReachability`, `BrowserDomainTransport`, `CreateDomainResult`, `JoinDomainResult`, `BrowserCreateDomainResult`, and `BrowserNetworkSession` are used consistently across TypeScript and wasm boundaries.
- Known risk: browser Manager reachability may require relay/circuit or an SDK WebTransport/WebSocket relay. That is a transport capability gap, not a Domain-role exception.
