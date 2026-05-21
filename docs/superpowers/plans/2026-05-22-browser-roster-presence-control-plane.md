# Browser Roster Presence Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make browser Domain peers that join through a native Manager see each other live and return successful media presence operations before real audio bytes are streamed.

**Architecture:** Keep `/auki/join/0.0.1` as the admission handshake, then add a Manager-backed browser session stream for long-lived roster and media presence snapshots. The browser wasm session owns a persistent libp2p swarm, sends local metadata/sensors/media intent to the Manager, and receives pushed roster snapshots. The TypeScript facade remains a thin Park-compatible wrapper over the wasm session.

**Tech Stack:** Rust, libp2p stream substreams, wasm-bindgen, serde JSON framing, TypeScript/Vitest, Playwright smoke tests.

---

## File Structure

- `crates/auki-network/src/browser_session_protocol.rs` — browser session wire messages, snapshot structs, and length-prefixed JSON framing.
- `crates/auki-network/src/lib.rs` — re-export the browser session protocol behind the existing browser/join protocol feature path.
- `crates/auki-network/examples/browser_join_listener.rs` — native smoke Manager that accepts browser session streams, stores browser participant state, and pushes roster snapshots to connected browser peers.
- `crates/auki-network-browser-wasm/src/lib.rs` — wasm `BrowserDomainSession` long-lived control-plane session, JS observer registration, local metadata/sensor/media state, and background swarm task.
- `crates/auki-domain-browser/src/peer.ts` — keep the facade compatible with wasm sessions and update local fallback media presence when a session lacks native observation.
- `crates/auki-domain-browser/src/peer.test.ts` — red/green tests for live roster snapshots and media presence updates at the TypeScript boundary.
- `crates/auki-network-browser-wasm/scripts/smoke_park_two_browser_acceptance.mjs` — existing final acceptance: A sees B, B sees A, publish/listen return ok.
- Component changelogs — append entries in `crates/auki-network*/changelog.md`, `crates/auki-domain-browser/changelog.md`, `crates/changelog.md`, and root `changelog.md`.

---

### Task 1: Protocol Types And Framing

**Files:**
- Create: `crates/auki-network/src/browser_session_protocol.rs`
- Modify: `crates/auki-network/src/lib.rs`
- Test: `crates/auki-network/src/browser_session_protocol.rs`

- [ ] **Step 1: Write the failing protocol tests**

Add tests that pin the protocol id, a client hello, a media publication update, and a server snapshot:

```rust
#[test]
fn protocol_id_is_stable() {
    assert_eq!(BROWSER_SESSION_PROTOCOL, "/auki/browser-session/0.0.1");
}

#[tokio::test]
async fn client_and_server_messages_round_trip() {
    let participant = sample_participant("browser-a", true);
    let hello = BrowserSessionClientMessage::Hello {
        domain_name: "browser-two-peer-smoke".to_string(),
        participant: participant.clone(),
    };
    let publish = BrowserSessionClientMessage::SetSensorPublication {
        sensor_id: "audio".to_string(),
        enabled: true,
    };
    let snapshot = BrowserSessionServerMessage::Snapshot {
        snapshot: BrowserRosterSnapshot {
            self_peer_id: "browser-a".to_string(),
            domain_name: "browser-two-peer-smoke".to_string(),
            manager_peer_id: "manager".to_string(),
            participants: vec![participant],
        },
    };

    let mut bytes = Vec::new();
    write_client_message(&mut bytes, &hello).await.unwrap();
    write_client_message(&mut bytes, &publish).await.unwrap();
    write_server_message(&mut bytes, &snapshot).await.unwrap();

    let mut cursor = futures::io::Cursor::new(bytes);
    assert_eq!(read_client_message(&mut cursor).await.unwrap(), hello);
    assert_eq!(read_client_message(&mut cursor).await.unwrap(), publish);
    assert_eq!(read_server_message(&mut cursor).await.unwrap(), snapshot);
}
```

Run: `cargo test -p auki-network browser_session_protocol --features join_protocol -- --nocapture`

Expected: FAIL because `browser_session_protocol` does not exist.

- [ ] **Step 2: Implement the protocol module**

Add these production types and helpers:

```rust
pub const BROWSER_SESSION_PROTOCOL: &str = "/auki/browser-session/0.0.1";
pub const MAX_BROWSER_SESSION_FRAME_BYTES: u32 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSessionClientMessage {
    Hello { domain_name: String, participant: BrowserSessionParticipant },
    UpdateParticipant { participant: BrowserSessionParticipant },
    SetSensorPublication { sensor_id: String, enabled: bool },
    Subscribe { peer_id: String, sensor_id: String },
    Unsubscribe { peer_id: String, sensor_id: String },
    Leave,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSessionServerMessage {
    Snapshot { snapshot: BrowserRosterSnapshot },
    Ack,
    Error { code: String, message: String },
}
```

Include `BrowserSessionParticipant`, `BrowserSessionSensor`, `BrowserMediaPresence`, and `BrowserRosterSnapshot` with snake_case Rust fields and serde `camelCase` renaming so JS receives Park-compatible field names.

- [ ] **Step 3: Export the module**

In `crates/auki-network/src/lib.rs`, add:

```rust
#[cfg(feature = "join_protocol")]
pub mod browser_session_protocol;
```

- [ ] **Step 4: Verify green**

Run: `cargo test -p auki-network browser_session_protocol --features join_protocol -- --nocapture`

Expected: PASS.

---

### Task 2: Native Smoke Manager Pushes Browser Rosters

**Files:**
- Modify: `crates/auki-network/examples/browser_join_listener.rs`
- Test: `crates/auki-network/examples/browser_join_listener.rs`

- [ ] **Step 1: Write the failing state test**

Add a pure Rust helper test that proves adding A then B produces symmetric snapshots:

```rust
#[test]
fn browser_roster_state_pushes_symmetric_snapshots() {
    let manager = "manager-peer".to_string();
    let mut roster = BrowserRosterState::new("browser-two-peer-smoke", manager.clone());
    roster.upsert(sample_participant("browser-a", true));
    roster.upsert(sample_participant("browser-b", true));

    let snapshot_a = roster.snapshot_for("browser-a");
    let snapshot_b = roster.snapshot_for("browser-b");

    assert!(snapshot_a.participants.iter().any(|p| p.peer_id == "browser-b" && !p.is_self));
    assert!(snapshot_b.participants.iter().any(|p| p.peer_id == "browser-a" && !p.is_self));
    assert_eq!(snapshot_a.manager_peer_id, manager);
    assert_eq!(snapshot_b.manager_peer_id, manager);
}
```

Run: `cargo test -p auki-network --example browser_join_listener browser_roster_state --features browser_probe,join_protocol -- --nocapture`

Expected: FAIL because `BrowserRosterState` does not exist.

- [ ] **Step 2: Add roster state and connected stream registry**

Implement `BrowserRosterState` and a `tokio::sync::broadcast::Sender<BrowserRosterSnapshot>` per connected browser. Use the libp2p stream peer id as the authoritative peer id; the participant body supplies metadata, sensors, and media presence only.

- [ ] **Step 3: Accept browser session streams**

In the example, accept `BROWSER_SESSION_PROTOCOL` alongside `JOIN_PROTOCOL`. For each stream:

1. Read `BrowserSessionClientMessage::Hello`.
2. Store the participant under the authenticated stream peer id.
3. Spawn a writer loop that sends `BrowserSessionServerMessage::Snapshot`.
4. Read subsequent client messages and mutate participant state.
5. Broadcast snapshots to all connected browser session writers.

- [ ] **Step 4: Keep join membership useful**

Update the join response membership JSON so each accepted browser join contains all browser peer ids known to the smoke Manager at that moment. This preserves the existing one-shot join behavior while the pushed browser session becomes the live source of truth.

- [ ] **Step 5: Verify green**

Run: `cargo test -p auki-network --example browser_join_listener browser_roster_state --features browser_probe,join_protocol -- --nocapture`

Expected: PASS.

---

### Task 3: Wasm Browser Session Keeps A Live Control Plane

**Files:**
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`

- [ ] **Step 1: Add a failing wasm-facing state test where possible**

Add non-wasm tests for local participant conversion and default media presence:

```rust
#[test]
fn local_browser_participant_defaults_match_park_contract() {
    let participant = browser_session_participant(
        "browser-a".to_string(),
        BrowserMetadata { app_id: "park".to_string(), display_name: "Park A".to_string() },
        vec![BrowserSensor { id: "audio".to_string(), kind: "audio".to_string(), label: "Microphone".to_string(), publishable: true, subscribable: false }],
        BrowserMediaPresence::default(),
        true,
    );

    assert_eq!(participant.peer_id, "browser-a");
    assert_eq!(participant.app_id, "park");
    assert_eq!(participant.media_presence.mic_available, true);
}
```

Run: `cargo test -p auki-network-browser-wasm local_browser_participant --features browser_libp2p -- --nocapture`

Expected: FAIL until the helper/state exists.

- [ ] **Step 2: Add interior session state**

Add state for metadata, sensors, media presence, current snapshot, JS observers, and an optional outbound browser session message sender. Use `Rc<BrowserDomainSessionState>` because browser wasm is single-threaded.

- [ ] **Step 3: Add JS-visible state methods**

Expose:

```rust
#[wasm_bindgen(js_name = setParticipantMetadata)]
pub fn set_participant_metadata(&self, metadata: JsValue) -> Result<JsValue, JsValue>;

#[wasm_bindgen(js_name = declareLocalSensors)]
pub fn declare_local_sensors(&self, sensors: JsValue) -> Result<JsValue, JsValue>;

#[wasm_bindgen(js_name = observeParticipants)]
pub fn observe_participants(&self, callback: js_sys::Function) -> js_sys::Function;
```

Each method updates local state immediately and sends `UpdateParticipant` if a browser session stream is active.

- [ ] **Step 4: Make join start the persistent session**

After `/auki/join/0.0.1` accepts, open `/auki/browser-session/0.0.1`, send `Hello`, and spawn a background task that:

1. Polls the swarm.
2. Writes queued client messages to the session stream.
3. Reads server snapshots.
4. Rewrites `isSelf` for the local peer.
5. Emits snapshots to JS observers.

Keep `joinDomain` returning the existing `{ ok, value: { domainName, managerPeerId, membershipJson } }` shape so the TypeScript facade and Park stay compatible.

- [ ] **Step 5: Media presence operations return ok**

Change `setSensorPublication`, `subscribeToSensor`, and `unsubscribeFromSensor` to update local media presence, queue the corresponding browser session message, and return `ok` once a session has joined. Before join, return `transport_unavailable`.

- [ ] **Step 6: Verify Rust build/tests**

Run:

```bash
cargo test -p auki-network-browser-wasm --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Expected: PASS.

---

### Task 4: TypeScript Facade Local Media Presence

**Files:**
- Modify: `crates/auki-domain-browser/src/peer.ts`
- Modify: `crates/auki-domain-browser/src/peer.test.ts`

- [ ] **Step 1: Write the failing facade tests**

Add tests that prove successful publication and subscription calls update snapshots when the injected SDK session does not own observation:

```ts
it("updates fallback media presence when SDK media operations succeed", async () => {
  const session = {
    peerId: () => "wasm-peer",
    joinDomain: vi.fn().mockResolvedValue({ ok: true, value: { domainName: "demo", managerPeerId: "manager-peer" } }),
    setSensorPublication: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    subscribeToSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
    unsubscribeFromSensor: vi.fn().mockResolvedValue({ ok: true, value: undefined }),
  };
  const peer = await createBrowserDomainPeer({ peerId: "fallback", sdkSession: session });
  const snapshots: unknown[] = [];
  peer.observeParticipants((snapshot) => snapshots.push(snapshot));

  await peer.joinDomain("http://discovery.example", "demo");
  await peer.setSensorPublication("audio", true);
  await peer.subscribeToSensor("remote-peer", "audio");

  expect(snapshots.at(-1)).toMatchObject({
    participants: [
      {
        peerId: "wasm-peer",
        mediaPresence: {
          micPublicationEnabled: true,
          listeningToPeerId: "remote-peer",
          listeningToSensorId: "audio",
          selectedRemoteStreamState: "connecting",
        },
      },
    ],
  });
});
```

Run: `npm --prefix crates/auki-domain-browser test -- peer.test.ts`

Expected: FAIL because media calls currently only delegate.

- [ ] **Step 2: Update fallback snapshot state**

When SDK media methods return ok and the SDK session did not supply its own `observeParticipants`, mutate the local participant media presence and emit a snapshot. Keep sessions with native `observeParticipants` delegated to the SDK.

- [ ] **Step 3: Verify green**

Run: `npm --prefix crates/auki-domain-browser test -- peer.test.ts`

Expected: PASS.

---

### Task 5: Acceptance Smoke

**Files:**
- Existing scripts only unless the smoke needs clearer output.

- [ ] **Step 1: Build wasm artifact for Park**

Run the repo's existing wasm/browser build command used by the current branch. If no wrapper exists, run:

```bash
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Expected: PASS.

- [ ] **Step 2: Start the native browser join listener**

Run:

```bash
cargo run -p auki-network --example browser_join_listener --features browser_probe,join_protocol
```

Expected: stdout prints `PARK_BROWSER_JOIN_ADDR=<webrtc-direct multiaddr>/p2p/<manager>`.

- [ ] **Step 3: Start Park dev server**

From `/Users/nilspihl/aukilabs-park`, run the existing Park command for the browser-peer scaffold, then open the local URL used by the smoke script.

- [ ] **Step 4: Run two-browser acceptance**

Run:

```bash
node crates/auki-network-browser-wasm/scripts/smoke_park_two_browser_acceptance.mjs "$PARK_BROWSER_JOIN_ADDR" http://127.0.0.1:7880
```

Expected: PASS with both browser peers seeing each other and all media operations returning ok.

---

## Self-Review

- Spec coverage: the plan covers live browser roster propagation, metadata/sensor publication, media presence operations returning ok, and the existing two-browser Park acceptance.
- Placeholder scan: no task uses TBD/TODO/fill-in instructions.
- Type consistency: protocol and facade names intentionally match the existing Park TypeScript contract: `PeerSnapshot`, `Participant`, `SensorSummary`, and `MediaPresence`.
