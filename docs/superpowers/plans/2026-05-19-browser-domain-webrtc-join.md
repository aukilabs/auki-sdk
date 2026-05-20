# Browser Domain WebRTC Join Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `auki-domain-browser`'s fail-closed join shell with a true browser leaf peer that discovers a native Manager, joins the Domain over SDK-owned WebRTC Direct libp2p, publishes browser participant info, and renders the Manager's participant info.

**Architecture:** Promote WebRTC Direct from the probe listener into the real native `auki-network` swarm so Discovery can advertise browser-dialable Manager addresses. Extend `auki-network-browser-wasm` from one-shot probe calls to a stateful browser swarm session that owns the browser PeerId, opens SDK raw substreams for `/auki/join/0.0.1` and `/auki/info/0.0.1`, and accepts inbound `/auki/info/0.0.1` on the same connection. Wire `auki-domain-browser` to this wasm session through a small TypeScript transport interface; Park still sees the existing `BrowserDomainPeer` contract.

**Tech Stack:** Rust 2024, rust-libp2p 0.56, `libp2p-webrtc` native WebRTC Direct, `libp2p-webrtc-websys`, `libp2p-stream`, `wasm-bindgen`, TypeScript, Vitest, `wasm-pack`, Chrome smoke via `playwright-core`.

---

## File Structure

- `crates/auki-network/Cargo.toml` - add a native `browser_webrtc` feature for the production swarm, sharing the WebRTC Direct dependency already proven by `browser_probe`.
- `crates/auki-network/src/browser_webrtc_transport.rs` - native WebRTC Direct transport constructor shared by the production swarm and the probe feature.
- `crates/auki-network/src/swarm.rs` - include the WebRTC Direct transport when `browser_webrtc` is enabled; keep the PeerId and existing behaviours unchanged.
- `crates/auki-network/src/readme.md`, `src/sprint.md`, changelogs - record that native Managers can advertise browser-dialable `/webrtc-direct/certhash/...` addresses.
- `crates/auki-network-browser-wasm/Cargo.toml` - add `libp2p-stream`, `serde_json`, and wasm session dependencies under `browser_libp2p`.
- `crates/auki-network-browser-wasm/src/lib.rs` - add a stateful `BrowserNetworkSession` wasm class plus join/info raw-substream framing.
- `crates/auki-network-browser-wasm/scripts/browser_domain_join_smoke.html` - browser smoke page that creates a session, joins a native Manager, and exposes the snapshot result.
- `crates/auki-network-browser-wasm/scripts/smoke_browser_domain_join.mjs` - Chrome smoke script for the join/info path.
- `crates/auki-domain-browser/src/contract.ts` - carry `managerMultiaddrs` in `DomainSummary`.
- `crates/auki-domain-browser/src/discovery.ts` - map Discovery `manager_multiaddrs` into browser contract rows.
- `crates/auki-domain-browser/src/transport.ts` - TypeScript interface for the wasm session boundary.
- `crates/auki-domain-browser/src/wasmTransport.ts` - adapter from the wasm package exports to `BrowserDomainTransport`.
- `crates/auki-domain-browser/src/peer.ts` - make `joinDomain` select a WebRTC Direct Manager address, call the transport, and emit a joined snapshot.
- `crates/auki-domain-browser/src/*.test.ts` - Vitest coverage for Discovery rows, address selection, join success, and transport failures.
- `crates/auki-domain-browser/src/README.md`, `src/sprint.md`, changelogs - document the first real browser peer join path.

This plan intentionally stops at Domain join plus participant metadata. Sensor catalogs, media presence, audio streams, and browser-created Domains follow after the browser peer stays joined as a real Domain participant.

---

### Task 1: Discovery Rows Carry Manager Multiaddrs

**Files:**
- Modify: `crates/auki-domain-browser/src/contract.ts`
- Modify: `crates/auki-domain-browser/src/discovery.ts`
- Modify: `crates/auki-domain-browser/src/discovery.test.ts`
- Modify: `crates/auki-domain-browser/src/README.md`
- Modify: changelogs

- [ ] **Step 1: Write the failing Discovery mapping test**

Update the first test in `crates/auki-domain-browser/src/discovery.test.ts`:

```ts
it("maps Discovery clusters into DomainSummary rows", async () => {
  const fetcher = vi.fn().mockResolvedValue(
    new Response(
      JSON.stringify({
        clusters: [
          {
            name: "retail-lab",
            manager_peer_id: "peer-manager",
            manager_multiaddrs: [
              "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/peer-manager",
              "/ip4/192.168.9.10/tcp/4001",
            ],
            peer_count: 2,
          },
        ],
      }),
      { status: 200 },
    ),
  );

  const result = await listDomains("http://discovery.example", fetcher);

  expect(fetcher).toHaveBeenCalledWith("http://discovery.example/clusters");
  expect(result).toEqual({
    ok: true,
    value: [
      {
        name: "retail-lab",
        managerPeerId: "peer-manager",
        managerMultiaddrs: [
          "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/peer-manager",
          "/ip4/192.168.9.10/tcp/4001",
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

Expected: the test fails because `DomainSummary` and `listDomains` do not expose `managerMultiaddrs`.

- [ ] **Step 3: Add the contract field**

Modify `crates/auki-domain-browser/src/contract.ts`:

```ts
export type DomainSummary = {
  name: DomainName;
  managerPeerId?: PeerId;
  managerMultiaddrs: string[];
  peerCount?: number;
};
```

- [ ] **Step 4: Map `manager_multiaddrs` defensively**

Modify `DiscoveryCluster` and the push body in `crates/auki-domain-browser/src/discovery.ts`:

```ts
type DiscoveryCluster = {
  name: string;
  manager_peer_id?: string;
  manager_multiaddrs?: unknown;
  peer_count?: number;
};

const managerMultiaddrs = Array.isArray(cluster.manager_multiaddrs)
  ? cluster.manager_multiaddrs.filter((addr): addr is string => typeof addr === "string")
  : [];

domains.push({
  name: cluster.name,
  managerPeerId:
    typeof cluster.manager_peer_id === "string" ? cluster.manager_peer_id : undefined,
  managerMultiaddrs,
  peerCount: typeof cluster.peer_count === "number" ? cluster.peer_count : undefined,
});
```

- [ ] **Step 5: Update the tests that construct `DomainSummary`**

Every expected Domain row in `crates/auki-domain-browser/src/discovery.test.ts` must include `managerMultiaddrs: []` when the fixture omits addresses.

- [ ] **Step 6: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test -- discovery.test.ts
npm --prefix crates/auki-domain-browser run build
```

Update docs/changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: expose browser manager multiaddrs"
```

---

### Task 2: Native Managers Advertise WebRTC Direct on the Production Swarm

**Files:**
- Modify: `crates/auki-network/Cargo.toml`
- Create: `crates/auki-network/src/browser_webrtc_transport.rs`
- Modify: `crates/auki-network/src/browser_probe.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Modify: `crates/auki-network/src/swarm.rs`
- Modify: `crates/auki-network/src/readme.md`
- Modify: `crates/auki-network/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Write the failing production-swarm identity test**

Add this test under `#[cfg(all(test, feature = "browser_webrtc"))]` in `crates/auki-network/src/swarm.rs`:

```rust
#[test]
fn browser_webrtc_swarm_keeps_sdk_peer_identity() {
    let identity = PeerIdentity::from_seed(&[41u8; 32]);
    let swarm = build_swarm(&identity, SwarmConfig::default()).expect("swarm builds");

    assert_eq!(*swarm.local_peer_id(), identity.peer_id());
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-network --features swarm,browser_webrtc browser_webrtc_swarm_keeps_sdk_peer_identity
```

Expected: compile failure because `browser_webrtc` is not a feature.

- [ ] **Step 3: Add the feature and shared transport constructor**

In `crates/auki-network/Cargo.toml`:

```toml
browser_webrtc = ["swarm", "dep:libp2p-webrtc", "dep:rand"]
browser_probe = ["browser_webrtc"]
```

Create `crates/auki-network/src/browser_webrtc_transport.rs`:

```rust
use libp2p::{
    PeerId,
    core::{muxing::StreamMuxerBox, transport::Boxed, Transport as _},
};
use libp2p_webrtc as webrtc;
use rand::thread_rng;

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

Export it from `crates/auki-network/src/lib.rs`:

```rust
#[cfg(feature = "browser_webrtc")]
pub mod browser_webrtc_transport;
```

- [ ] **Step 4: Reuse the shared constructor in the probe module**

In `crates/auki-network/src/browser_probe.rs`, delete the local `webrtc_direct_transport` implementation and import:

```rust
use crate::browser_webrtc_transport::webrtc_direct_transport;
```

- [ ] **Step 5: Add WebRTC Direct to `build_swarm` when enabled**

In `crates/auki-network/src/swarm.rs`, split the builder before `.with_behaviour(...)` into a helper function so the transport stack stays readable:

```rust
#[cfg(feature = "browser_webrtc")]
fn add_browser_webrtc_transport<Provider, Phase>(
    builder: libp2p::SwarmBuilder<Provider, Phase>,
) -> Result<
    libp2p::SwarmBuilder<Provider, libp2p::builder::phase::OtherTransportPhase<impl libp2p::builder::phase::AuthenticatedMultiplexedTransport>>,
    std::convert::Infallible,
>
where
    libp2p::SwarmBuilder<Provider, Phase>: BrowserWebrtcBuilderExt<Provider>,
{
    builder.with_other_transport(crate::browser_webrtc_transport::webrtc_direct_transport)
}
```

If the type-state helper above proves too brittle, keep the production path explicit and duplicated: one `#[cfg(feature = "browser_webrtc")]` `build_swarm` branch that calls `.with_other_transport(...)`, and one `#[cfg(not(feature = "browser_webrtc"))]` branch that preserves the current TCP + QUIC + relay stack. Do not change `Behaviour` or `NetworkRuntime`.

- [ ] **Step 6: Verify and commit**

```bash
cargo test -p auki-network --features swarm,browser_webrtc browser_webrtc_swarm_keeps_sdk_peer_identity
cargo check -p auki-network --features swarm,browser_webrtc
cargo check -p auki-network --features browser_probe --example browser_probe_listener
```

Run a native Manager app after the `auki-domain` task below and verify Discovery rows include at least one `/webrtc-direct/certhash/.../p2p/<manager>` address.

Update docs/changelogs, then commit:

```bash
git add Cargo.toml Cargo.lock crates/auki-network crates/changelog.md changelog.md
git commit -m "feat: add browser webrtc to native swarm"
```

---

### Task 3: Browser WASM Session Opens SDK Raw Substreams

**Files:**
- Modify: `crates/auki-network-browser-wasm/Cargo.toml`
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/src/sprint.md`
- Modify: changelogs

- [ ] **Step 1: Write failing result-shape tests**

Add native tests in `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(test)]
mod browser_domain_session_result_tests {
    use super::*;

    #[test]
    fn join_result_carries_membership_and_manager_info() {
        let result = BrowserJoinResult::ok(
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar",
            "manager-peer",
            "{\"members\":[]}",
            "{\"app\":\"park\"}",
        );

        assert!(result.ok);
        assert_eq!(result.local_peer_id, "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar");
        assert_eq!(result.manager_peer_id, "manager-peer");
        assert_eq!(result.membership_json, "{\"members\":[]}");
        assert_eq!(result.manager_info_json, "{\"app\":\"park\"}");
        assert!(result.error.is_none());
    }
}
```

- [ ] **Step 2: Run the failing test**

```bash
cargo test -p auki-network-browser-wasm browser_domain_session_result_tests::join_result_carries_membership_and_manager_info
```

Expected: compile failure because `BrowserJoinResult` does not exist.

- [ ] **Step 3: Add wasm dependencies**

In `crates/auki-network-browser-wasm/Cargo.toml`, extend `browser_libp2p`:

```toml
browser_libp2p = [
    "dep:futures",
    "dep:libp2p",
    "dep:libp2p-stream",
    "dep:serde",
    "dep:serde_json",
    "dep:serde-wasm-bindgen",
    "dep:wasm-bindgen-futures",
    "dep:web-sys",
]

libp2p-stream = { version = "=0.4.0-alpha", optional = true }
serde_json = { version = "1", optional = true }
```

- [ ] **Step 4: Add `BrowserJoinResult`**

Add this struct beside `BrowserProbeResult`:

```rust
#[cfg_attr(feature = "browser_libp2p", derive(serde::Serialize))]
pub struct BrowserJoinResult {
    pub ok: bool,
    pub local_peer_id: String,
    pub manager_peer_id: String,
    pub membership_json: String,
    pub manager_info_json: String,
    pub error: Option<String>,
}

impl BrowserJoinResult {
    pub fn ok(
        local_peer_id: impl Into<String>,
        manager_peer_id: impl Into<String>,
        membership_json: impl Into<String>,
        manager_info_json: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: manager_peer_id.into(),
            membership_json: membership_json.into(),
            manager_info_json: manager_info_json.into(),
            error: None,
        }
    }

    pub fn err(local_peer_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            local_peer_id: local_peer_id.into(),
            manager_peer_id: String::new(),
            membership_json: String::new(),
            manager_info_json: String::new(),
            error: Some(error.into()),
        }
    }
}
```

- [ ] **Step 5: Add raw frame helpers**

Add browser-only helpers in `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn write_framed_json<T: serde::Serialize>(
    stream: &mut libp2p::swarm::Stream,
    value: &T,
) -> Result<(), String> {
    use futures::AsyncWriteExt as _;
    let bytes = serde_json::to_vec(value).map_err(|err| format!("encode: {err}"))?;
    let len: u32 = bytes
        .len()
        .try_into()
        .map_err(|_| format!("frame too large: {} bytes", bytes.len()))?;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|err| format!("write length: {err}"))?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|err| format!("write body: {err}"))?;
    stream.flush().await.map_err(|err| format!("flush: {err}"))
}

#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
async fn read_framed_json<T: for<'de> serde::Deserialize<'de>>(
    stream: &mut libp2p::swarm::Stream,
    max_bytes: u32,
) -> Result<T, String> {
    use futures::AsyncReadExt as _;
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|err| format!("read length: {err}"))?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err("frame is empty".to_string());
    }
    if len > max_bytes {
        return Err(format!("frame too large: {len} bytes (max {max_bytes})"));
    }
    let mut bytes = vec![0u8; len as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|err| format!("read body: {err}"))?;
    serde_json::from_slice(&bytes).map_err(|err| format!("decode: {err}"))
}
```

- [ ] **Step 6: Add a stateful wasm session export**

Add a wasm class that owns the browser swarm and a `libp2p_stream::Control`:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
pub struct BrowserNetworkSession {
    local_peer_id: String,
    control: libp2p_stream::Control,
}
```

The constructor must:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
impl BrowserNetworkSession {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: &[u8], participant_info_json: String) -> Result<BrowserNetworkSession, JsValue> {
        // 1. Validate 32-byte seed with seed_array(seed).
        // 2. Build PeerIdentity through peer_identity_from_seed_bytes.
        // 3. Build a WebRTC Direct browser swarm with libp2p_stream::Behaviour.
        // 4. Create a control handle and accept `/auki/info/0.0.1`.
        // 5. Spawn the swarm driver with wasm_bindgen_futures::spawn_local.
        // 6. Spawn an inbound info responder that returns `participant_info_json`.
        // 7. Return BrowserNetworkSession { local_peer_id, control }.
    }
}
```

When filling this in, use `libp2p_stream::Behaviour::new()` as the sole behaviour for the session. Keep the existing one-shot `dialBrowserProbe` code intact; it remains the low-level probe surface.

- [ ] **Step 7: Add `joinManager` method**

Add this wasm method:

```rust
#[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
#[wasm_bindgen]
impl BrowserNetworkSession {
    #[wasm_bindgen(js_name = joinManager)]
    pub async fn join_manager(
        &mut self,
        manager_peer_id: String,
        manager_address: String,
    ) -> Result<JsValue, JsValue> {
        // 1. Parse manager_peer_id as libp2p::PeerId.
        // 2. Parse manager_address as libp2p::Multiaddr.
        // 3. Add the address to the swarm/control path before opening.
        // 4. Open `/auki/join/0.0.1` with libp2p_stream::Control::open_stream.
        // 5. Write `{ "multiaddrs": [] }` as length-prefixed JSON.
        // 6. Read a length-prefixed JoinResponse JSON capped at 1 MiB.
        // 7. If `kind` is `reject`, return BrowserJoinResult::err.
        // 8. If `kind` is `accept`, open `/auki/info/0.0.1` to the Manager.
        // 9. Write `{}` as InfoRequest and read InfoResponse capped at 64 KiB.
        // 10. Return BrowserJoinResult::ok(local_peer_id, manager_peer_id, membership_json, participant_info_json).
    }
}
```

The JoinResponse JSON shape to deserialize locally:

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrowserJoinResponse {
    Accept {
        membership_json: String,
        #[serde(default)]
        successor_token: Vec<u8>,
    },
    Reject {
        reason: String,
    },
}
```

The InfoResponse JSON shape:

```rust
#[derive(serde::Deserialize)]
struct BrowserInfoResponse {
    participant_info_json: String,
}
```

- [ ] **Step 8: Verify and commit**

```bash
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Update docs/changelogs, then commit:

```bash
git add Cargo.lock crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "feat: add browser domain wasm session"
```

---

### Task 4: Wire `auki-domain-browser` to the WASM Session

**Files:**
- Create: `crates/auki-domain-browser/src/transport.ts`
- Create: `crates/auki-domain-browser/src/wasmTransport.ts`
- Modify: `crates/auki-domain-browser/src/peer.ts`
- Modify: `crates/auki-domain-browser/src/peer.test.ts`
- Modify: `crates/auki-domain-browser/src/index.ts`
- Modify: docs/changelogs

- [ ] **Step 1: Add the transport interface**

Create `crates/auki-domain-browser/src/transport.ts`:

```ts
export type JoinManagerResult =
  | {
      ok: true;
      localPeerId: string;
      managerPeerId: string;
      membershipJson: string;
      managerInfoJson: string;
    }
  | { ok: false; localPeerId: string; error: string };

export type BrowserDomainTransport = {
  localPeerId(): string;
  joinManager(managerPeerId: string, managerAddress: string): Promise<JoinManagerResult>;
};

export function selectBrowserDialableAddress(addresses: string[]): string | null {
  return addresses.find((address) => address.includes("/webrtc-direct/")) ?? null;
}
```

- [ ] **Step 2: Write selection tests**

Create `crates/auki-domain-browser/src/transport.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { selectBrowserDialableAddress } from "./transport.js";

describe("selectBrowserDialableAddress", () => {
  it("chooses the WebRTC Direct manager address", () => {
    expect(
      selectBrowserDialableAddress([
        "/ip4/192.168.9.10/tcp/4001",
        "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager",
      ]),
    ).toBe("/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager");
  });

  it("returns null when Discovery has no browser-dialable address", () => {
    expect(selectBrowserDialableAddress(["/ip4/192.168.9.10/tcp/4001"])).toBeNull();
  });
});
```

- [ ] **Step 3: Add peer options for transport injection**

Modify `CreateBrowserDomainPeerOptions` in `crates/auki-domain-browser/src/peer.ts`:

```ts
import type { BrowserDomainTransport } from "./transport.js";

export type CreateBrowserDomainPeerOptions = {
  peerId: PeerId;
  fetcher?: Fetcher;
  transport?: BrowserDomainTransport;
};
```

- [ ] **Step 4: Write a failing join success test**

Replace the old "fails closed for join" test with:

```ts
it("joins through the browser transport and emits a participant snapshot", async () => {
  const fetcher = vi.fn().mockResolvedValue(
    new Response(
      JSON.stringify({
        clusters: [
          {
            name: "demo",
            manager_peer_id: "manager-peer",
            manager_multiaddrs: [
              "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager-peer",
            ],
            peer_count: 2,
          },
        ],
      }),
      { status: 200 },
    ),
  );
  const transport = {
    localPeerId: () => "self-peer",
    joinManager: vi.fn().mockResolvedValue({
      ok: true,
      localPeerId: "self-peer",
      managerPeerId: "manager-peer",
      membershipJson: "{\"members\":[]}",
      managerInfoJson: JSON.stringify({
        app: "manager-app",
        name: "Manager",
        peer_id: "manager-peer",
      }),
    }),
  };
  const peer = await createBrowserDomainPeer({ peerId: "self-peer", fetcher, transport });
  const snapshots: unknown[] = [];
  peer.observeParticipants((snapshot) => snapshots.push(snapshot));

  const result = await peer.joinDomain("http://discovery.example", "demo");

  expect(result).toEqual({ ok: true, value: undefined });
  expect(transport.joinManager).toHaveBeenCalledWith(
    "manager-peer",
    "/ip4/127.0.0.1/udp/55214/webrtc-direct/certhash/uEiBprobe/p2p/manager-peer",
  );
  expect(snapshots.at(-1)).toMatchObject({
    selfPeerId: "self-peer",
    domainName: "demo",
    managerPeerId: "manager-peer",
    electionState: "stable",
  });
});
```

- [ ] **Step 5: Implement `joinDomain`**

In `peer.ts`, implement:

```ts
async joinDomain(discoveryUrl, domainName) {
  if (!options.transport) return transportUnavailable();

  const domains = await listDiscoveryDomains(discoveryUrl, fetcher);
  if (!domains.ok) return domains;

  const domain = domains.value.find((candidate) => candidate.name === domainName);
  if (!domain || !domain.managerPeerId) {
    return fail("domain_join_failed", `Domain ${domainName} has no Manager in Discovery.`);
  }

  const managerAddress = selectBrowserDialableAddress(domain.managerMultiaddrs);
  if (!managerAddress) {
    return fail("transport_unavailable", `Domain ${domainName} has no browser-dialable Manager address.`);
  }

  const joined = await options.transport.joinManager(domain.managerPeerId, managerAddress);
  if (!joined.ok) {
    return fail("domain_join_failed", joined.error);
  }

  snapshot = {
    selfPeerId: options.peerId,
    domainName,
    participants: [
      participantFromManagerInfo(joined.managerPeerId, joined.managerInfoJson),
      selfParticipant(options.peerId),
    ],
    managerPeerId: joined.managerPeerId,
    electionState: "stable",
  };
  emit();
  return ok<void>(undefined);
}
```

Add small helpers in the same file:

```ts
function selfParticipant(peerId: PeerId): Participant {
  return {
    peerId,
    appId: "park-browser",
    displayName: `Browser ${peerId.slice(-6)}`,
    isSelf: true,
    connected: true,
    sensors: [],
    mediaPresence: emptyMediaPresence(),
  };
}

function participantFromManagerInfo(peerId: PeerId, infoJson: string): Participant {
  const info = JSON.parse(infoJson) as { app?: string; name?: string };
  return {
    peerId,
    appId: info.app ?? "unknown",
    displayName: info.name ?? peerId,
    isSelf: false,
    connected: true,
    sensors: [],
    mediaPresence: emptyMediaPresence(),
  };
}

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
```

- [ ] **Step 6: Add the wasm transport adapter**

Create `crates/auki-domain-browser/src/wasmTransport.ts`:

```ts
import type { BrowserDomainTransport, JoinManagerResult } from "./transport.js";

type WasmSession = {
  localPeerId(): string;
  joinManager(managerPeerId: string, managerAddress: string): Promise<{
    ok: boolean;
    local_peer_id: string;
    manager_peer_id: string;
    membership_json: string;
    manager_info_json: string;
    error?: string;
  }>;
};

export function createWasmTransport(session: WasmSession): BrowserDomainTransport {
  return {
    localPeerId: () => session.localPeerId(),
    async joinManager(managerPeerId, managerAddress): Promise<JoinManagerResult> {
      const result = await session.joinManager(managerPeerId, managerAddress);
      if (!result.ok) {
        return { ok: false, localPeerId: result.local_peer_id, error: result.error ?? "join failed" };
      }
      return {
        ok: true,
        localPeerId: result.local_peer_id,
        managerPeerId: result.manager_peer_id,
        membershipJson: result.membership_json,
        managerInfoJson: result.manager_info_json,
      };
    },
  };
}
```

Export it from `src/index.ts`.

- [ ] **Step 7: Verify and commit**

```bash
npm --prefix crates/auki-domain-browser test
npm --prefix crates/auki-domain-browser run build
```

Update docs/changelogs, then commit:

```bash
git add crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "feat: join browser domain through wasm transport"
```

---

### Task 5: End-to-End Browser Domain Join Smoke

**Files:**
- Create: `crates/auki-network-browser-wasm/scripts/browser_domain_join_smoke.html`
- Create: `crates/auki-network-browser-wasm/scripts/smoke_browser_domain_join.mjs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-domain-browser/src/README.md`
- Modify: changelogs

- [ ] **Step 1: Add the smoke page**

Create `browser_domain_join_smoke.html` that imports the built wasm package, constructs `BrowserNetworkSession`, calls `joinManager`, and writes the JSON result to `document.body.dataset.result`. Use the same seed vector as the probe smoke:

```html
<!doctype html>
<meta charset="utf-8" />
<script type="module">
  import init, { BrowserNetworkSession } from "../pkg-web/auki_network_browser_wasm.js";

  const params = new URLSearchParams(location.search);
  const address = params.get("address");
  const manager = params.get("manager");
  const seed = new Uint8Array(32).fill(3);
  const selfInfo = JSON.stringify({
    app: "park-browser",
    name: "Park Browser",
    peer_id: "browser",
  });

  try {
    await init();
    const session = new BrowserNetworkSession(seed, selfInfo);
    const result = await session.joinManager(manager, address);
    document.body.dataset.result = JSON.stringify(result);
    document.body.textContent = JSON.stringify(result);
  } catch (err) {
    document.body.dataset.result = JSON.stringify({ ok: false, error: String(err) });
    document.body.textContent = String(err);
  }
</script>
```

- [ ] **Step 2: Add the smoke script**

Create `smoke_browser_domain_join.mjs` by copying `smoke_browser_probe.mjs` and changing the URL path plus assertions:

```js
if (!result.ok) throw new Error(result.error);
if (result.local_peer_id !== "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar") {
  throw new Error(`bad local peer: ${result.local_peer_id}`);
}
if (!result.membership_json) throw new Error("missing membership_json");
if (!result.manager_info_json) throw new Error("missing manager_info_json");
console.log(`ok ${result.local_peer_id}`);
```

- [ ] **Step 3: Run against a native Manager with browser WebRTC enabled**

Start a native SDK app that creates a Domain and advertises a `/webrtc-direct/certhash/.../p2p/<manager>` Manager address through Discovery. Then run:

```bash
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
node crates/auki-network-browser-wasm/scripts/smoke_browser_domain_join.mjs '<manager-peer-id>' '<webrtc-direct-manager-multiaddr>'
```

Expected:

```text
ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar
```

If this fails, capture the exact blocker in `crates/auki-network-browser-wasm/parking_lot.md` using one of these labels: `native webrtc production swarm`, `raw stream negotiation`, `join protocol`, `info protocol`, `browser session lifetime`, or `Discovery address advertisement`. Propagate parking-lot summaries upward.

- [ ] **Step 4: Verify and commit**

```bash
cargo check -p auki-network --features swarm,browser_webrtc
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
npm --prefix crates/auki-domain-browser test
npm --prefix crates/auki-domain-browser run build
```

Update docs/changelogs, then commit:

```bash
git add crates/auki-network crates/auki-network-browser-wasm crates/auki-domain-browser crates/changelog.md changelog.md
git commit -m "test: smoke browser domain join"
```

---

## Self-Review Notes

- Spec coverage: The plan preserves the SDK-owned networking rule by using native SDK WebRTC Direct addresses, browser wasm libp2p, raw SDK substreams, and no browser-native WebRTC call path. It covers Discovery address selection, Domain join, browser participant info production, Manager participant info consumption, and UI snapshot updates.
- Placeholder scan: No placeholder markers remain; each task has concrete file paths, test commands, expected failures, and implementation shapes.
- Type consistency: `managerMultiaddrs`, `BrowserDomainTransport`, `BrowserJoinResult`, `BrowserNetworkSession`, `joinManager`, `membership_json`, and `manager_info_json` are named consistently across the Rust wasm boundary and TypeScript adapter.
