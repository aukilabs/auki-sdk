# WASM libp2p Browser Transport Compile Probe Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first measurable slice of the wasm-libp2p browser transport spike: a new SDK wasm crate that can derive canonical PeerIds, produce an importable wasm package, and prove whether libp2p 0.56 browser transport features compile in this SDK.

**Architecture:** Add `crates/auki-network-browser-wasm` as the low-level browser networking probe crate. The first tasks deliberately avoid Domain join and audio; they expose a small wasm API, smoke-test JS importability, then add a `browser_libp2p` feature that enables rust-libp2p browser transport crates on `wasm32-unknown-unknown`. If the libp2p feature probe does not compile, record the exact blocker and keep `auki-domain-browser` fail-closed.

**Tech Stack:** Rust 2024, `wasm-bindgen`, `wasm-pack`, `auki-identity`, `auki-network` default identity surface, rust-libp2p 0.56 browser features (`wasm-bindgen`, `webrtc-websys`, `webtransport-websys`, `websocket-websys`), Node.js smoke import.

---

## File Structure

- `crates/auki-network-browser-wasm/README.md` — aspirational component spec for the browser wasm networking probe.
- `crates/auki-network-browser-wasm/parking_lot.md` — exact transport compile/runtime blockers surfaced by the spike.
- `crates/auki-network-browser-wasm/changelog.md` — leaf changelog.
- `crates/auki-network-browser-wasm/Cargo.toml` — wasm crate manifest and optional `browser_libp2p` feature.
- `crates/auki-network-browser-wasm/src/README.md` — implemented status.
- `crates/auki-network-browser-wasm/src/sprint.md` — immediate next steps.
- `crates/auki-network-browser-wasm/src/lib.rs` — wasm-bound exported probe surface.
- `crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs` — Node import smoke test for `wasm-pack --target nodejs` output.
- `Cargo.toml` — workspace member list.
- `crates/changelog.md` and root `changelog.md` — changelog propagation.

This plan stops at the compile/import proof. The native browser-compatible listener and real protocol dial test get a follow-up plan once this slice tells us which libp2p browser transport features compile.

---

### Task 1: Scaffold `auki-network-browser-wasm`

**Files:**
- Create: `crates/auki-network-browser-wasm/README.md`
- Create: `crates/auki-network-browser-wasm/parking_lot.md`
- Create: `crates/auki-network-browser-wasm/changelog.md`
- Create: `crates/auki-network-browser-wasm/Cargo.toml`
- Create: `crates/auki-network-browser-wasm/src/README.md`
- Create: `crates/auki-network-browser-wasm/src/sprint.md`
- Create: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Create the crate manifest**

Create `crates/auki-network-browser-wasm/Cargo.toml`:

```toml
[package]
name = "auki-network-browser-wasm"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Browser/WASM libp2p transport probe for the Auki SDK."

[lib]
crate-type = ["cdylib", "rlib"]

[features]
default = []
browser_libp2p = [
    "dep:libp2p",
]

[dependencies]
auki-identity = { path = "../auki-identity" }
auki-network = { path = "../auki-network" }
js-sys = "0.3"
wasm-bindgen = "0.2"

# Enables browser RNG support for transitive `getrandom` 0.2 users when this
# crate is built for `wasm32-unknown-unknown`.
getrandom = { version = "0.2", features = ["js"] }

libp2p = { version = "0.56", default-features = false, optional = true, features = [
    "ed25519",
    "identify",
    "json",
    "macros",
    "noise",
    "ping",
    "request-response",
    "wasm-bindgen",
    "webrtc-websys",
    "websocket-websys",
    "webtransport-websys",
    "yamux",
] }

[dev-dependencies]
wasm-bindgen-test = "0.3"
```

- [ ] **Step 2: Add the crate to the workspace**

Modify root `Cargo.toml` by adding the member immediately after `crates/auki-network`:

```toml
    "crates/auki-network",
    "crates/auki-network-browser-wasm",
    "crates/auki-network-py",
```

- [ ] **Step 3: Create placeholder-safe Rust source**

Create `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sdkName)]
pub fn sdk_name() -> String {
    "auki-network-browser-wasm".to_string()
}
```

- [ ] **Step 4: Create component docs**

Create `crates/auki-network-browser-wasm/README.md`:

```markdown
# auki-network-browser-wasm

Browser/WASM networking probe for the Auki SDK.

This crate tests whether the SDK can run a rust-libp2p peer in the browser while preserving canonical Auki peer identity and SDK-owned protocol streams. It is lower-level than `auki-domain-browser`: this crate proves browser networking primitives, while `auki-domain-browser` remains Park's Domain-level adapter.

The first implementation slice exposes identity/import probes and then compiles rust-libp2p browser transport features. Domain join, browser Manager behavior, and audio are later work.
```

Create `crates/auki-network-browser-wasm/parking_lot.md`:

```markdown
# Parking Lot — auki-network-browser-wasm

Open questions and blockers for the browser/WASM networking probe.

## Items

- **2026-05-19 — Native browser-compatible listener.** Once the `browser_libp2p` wasm feature compiles, choose and implement the matching native SDK listener/probe fixture: WebRTC Direct first, WebTransport second, Secure WebSocket only as fallback.
```

Create `crates/auki-network-browser-wasm/changelog.md`:

```markdown
# Changelog — auki-network-browser-wasm

Append-only timeline of changes for the browser/WASM networking probe. Latest entry on top.

---

### Nils's codex · May 19, HKT, 2026

Created the `auki-network-browser-wasm` crate scaffold for the rust-libp2p browser transport spike. The crate starts as an importable wasm shell; identity and libp2p browser feature probes follow in separate commits.
```

Create `crates/auki-network-browser-wasm/src/README.md`:

```markdown
# auki-network-browser-wasm/src

Implementation status for the browser/WASM networking probe.

Currently implemented:

- wasm crate scaffold
- `sdkName()` smoke export

Not yet implemented:

- canonical seed-to-PeerId export
- Node/browser import smoke script
- rust-libp2p browser feature compile probe
- native browser-compatible probe listener
- browser-to-native protocol dial
```

Create `crates/auki-network-browser-wasm/src/sprint.md`:

```markdown
# auki-network-browser-wasm/src — sprint

## Now

Prove the first wasm package boundary:

- build the crate for `wasm32-unknown-unknown`
- export a small wasm-bindgen function
- add canonical seed-to-PeerId derivation
- add a JS import smoke test
- compile the rust-libp2p browser transport feature set

## Next

If `browser_libp2p` compiles, build an SDK-owned native probe listener and open one named protocol stream from the browser wasm peer.
```

- [ ] **Step 5: Propagate changelogs**

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**[`auki-network-browser-wasm`](auki-network-browser-wasm/changelog.md) — browser/WASM networking probe scaffold.** Added the crate shell for the rust-libp2p browser transport spike, with docs and workspace registration before identity/import/libp2p probes.
```

Prepend to root `changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Browser/WASM networking probe scaffolded.** Added `auki-network-browser-wasm` as the low-level crate for proving rust-libp2p browser transport before wiring real Domain join into Park. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 6: Verify the scaffold**

Run:

```bash
cargo check -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown
```

Expected: both commands pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "chore: scaffold browser wasm network probe"
```

---

### Task 2: Canonical PeerId WASM Export

**Files:**
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Write the failing Rust unit test**

Replace `crates/auki-network-browser-wasm/src/lib.rs` with this test-first version:

```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sdkName)]
pub fn sdk_name() -> String {
    "auki-network-browser-wasm".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_seed_03_peer_id_matches_sdk_vector() {
        assert_eq!(
            peer_id_from_seed_bytes(&[3u8; 32]).expect("valid seed"),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
    }
}
```

- [ ] **Step 2: Run the failing test**

Run:

```bash
cargo test -p auki-network-browser-wasm locked_seed_03_peer_id_matches_sdk_vector
```

Expected: compile failure mentioning `peer_id_from_seed_bytes` is not found.

- [ ] **Step 3: Implement canonical seed-to-PeerId export**

Replace `crates/auki-network-browser-wasm/src/lib.rs` with:

```rust
use auki_identity::Wallet;
use auki_network::PeerIdentity;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = sdkName)]
pub fn sdk_name() -> String {
    "auki-network-browser-wasm".to_string()
}

#[wasm_bindgen(js_name = peerIdFromSeed)]
pub fn peer_id_from_seed(seed: &[u8]) -> Result<String, JsValue> {
    let seed = seed_array(seed)?;
    peer_id_from_seed_bytes(&seed).map_err(JsValue::from_str)
}

pub fn peer_id_from_seed_bytes(seed: &[u8; 32]) -> Result<String, String> {
    let wallet = Wallet::from_seed(seed);
    let identity = PeerIdentity::from_wallet(&wallet);
    Ok(identity.peer_id().to_string())
}

fn seed_array(seed: &[u8]) -> Result<[u8; 32], String> {
    if seed.len() != 32 {
        return Err(format!("seed must be 32 bytes, got {}", seed.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(seed);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_seed_03_peer_id_matches_sdk_vector() {
        assert_eq!(
            peer_id_from_seed_bytes(&[3u8; 32]).expect("valid seed"),
            "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
        );
    }

    #[test]
    fn rejects_wrong_length_seed() {
        let err = seed_array(&[1, 2, 3]).expect_err("short seed rejected");
        assert_eq!(err, "seed must be 32 bytes, got 3");
    }
}
```

- [ ] **Step 4: Verify native tests and wasm build**

Run:

```bash
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web
```

Expected: all commands pass. If `wasm-pack` fails because generated output collides with workspace assumptions, keep the Rust tests green and record the exact `wasm-pack` error in `crates/auki-network-browser-wasm/parking_lot.md`.

- [ ] **Step 5: Update status docs and changelogs**

Update `crates/auki-network-browser-wasm/src/README.md` so the "Currently implemented" list is:

```markdown
- wasm crate scaffold
- `sdkName()` smoke export
- canonical seed-to-PeerId export
```

and remove `canonical seed-to-PeerId export` from "Not yet implemented".

Prepend to `crates/auki-network-browser-wasm/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

Added the canonical `peerIdFromSeed(seed)` wasm export. The Rust test pins seed `[3u8; 32]` to the SDK's locked libp2p PeerId vector, and wrong-length seeds return a typed error instead of panicking.
```

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**[`auki-network-browser-wasm`](auki-network-browser-wasm/changelog.md) — canonical PeerId wasm export.** Added `peerIdFromSeed(seed)` and pinned the browser wasm crate to the SDK's locked seed-to-libp2p-PeerId vector.
```

Prepend to root `changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Browser/WASM probe exports canonical PeerIds.** `auki-network-browser-wasm` now exposes `peerIdFromSeed(seed)` and tests it against the SDK's locked libp2p PeerId vector. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "feat: export browser wasm peer id"
```

---

### Task 3: Node Import Smoke Test

**Files:**
- Create: `crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: changelogs

- [ ] **Step 1: Write the smoke script**

Create `crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs`:

```js
import init, { peerIdFromSeed, sdkName } from "../pkg-node/auki_network_browser_wasm.js";

await init();

const seed = new Uint8Array(32).fill(3);
const peerId = peerIdFromSeed(seed);

if (sdkName() !== "auki-network-browser-wasm") {
  throw new Error(`unexpected sdkName: ${sdkName()}`);
}

if (peerId !== "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar") {
  throw new Error(`unexpected peer id: ${peerId}`);
}

console.log(`ok ${peerId}`);
```

- [ ] **Step 2: Run the smoke script before building node output**

Run:

```bash
node crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs
```

Expected: failure because `pkg-node/auki_network_browser_wasm.js` does not exist.

- [ ] **Step 3: Build Node-target wasm and rerun smoke**

Run:

```bash
wasm-pack build crates/auki-network-browser-wasm --target nodejs --out-dir pkg-node
node crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs
```

Expected: `node` prints:

```text
ok 12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar
```

- [ ] **Step 4: Ensure generated wasm packages stay untracked**

Run:

```bash
git status --short crates/auki-network-browser-wasm
```

Expected: `pkg-node/` and `pkg-web/` are not shown as untracked. If they are shown, add this line to root `.gitignore`:

```gitignore
crates/*/pkg*/
```

Then rerun `git status --short crates/auki-network-browser-wasm`.

- [ ] **Step 5: Update status docs and changelogs**

Update `crates/auki-network-browser-wasm/src/README.md` so "Currently implemented" includes:

```markdown
- Node import smoke script for `wasm-pack --target nodejs`
```

and remove `Node/browser import smoke script` from "Not yet implemented" by replacing it with:

```markdown
- browser-page import smoke script
```

Prepend to `crates/auki-network-browser-wasm/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

Added a Node import smoke script for the wasm package. `wasm-pack --target nodejs` output can be imported from JavaScript and reproduces the locked PeerId vector through the wasm boundary.
```

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**[`auki-network-browser-wasm`](auki-network-browser-wasm/changelog.md) — wasm package import smoke test.** Added a Node smoke script proving the generated wasm package imports and returns the locked PeerId vector through JavaScript.
```

Prepend to root `changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Browser/WASM probe imports from JavaScript.** `auki-network-browser-wasm` now has a Node smoke script proving generated wasm can be imported and used from JS. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 6: Commit**

```bash
git add .gitignore crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "test: smoke import browser wasm package"
```

---

### Task 4: rust-libp2p Browser Feature Compile Probe

**Files:**
- Modify: `crates/auki-network-browser-wasm/src/lib.rs`
- Modify: `crates/auki-network-browser-wasm/src/README.md`
- Modify: `crates/auki-network-browser-wasm/src/sprint.md`
- Modify: `crates/auki-network-browser-wasm/parking_lot.md` only if the compile probe fails
- Modify: changelogs

- [ ] **Step 1: Add the failing feature expectations**

Append this test module to `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[cfg(test)]
mod transport_feature_tests {
    use super::*;

    #[test]
    fn base_build_reports_no_transport_features() {
        let features = supported_transports_vec();
        assert_eq!(features, vec!["identity-only"]);
    }
}
```

Run:

```bash
cargo test -p auki-network-browser-wasm base_build_reports_no_transport_features
```

Expected: compile failure because `supported_transports_vec` does not exist.

- [ ] **Step 2: Implement feature reporting**

Add these exports above the existing test modules in `crates/auki-network-browser-wasm/src/lib.rs`:

```rust
#[wasm_bindgen(js_name = supportedTransports)]
pub fn supported_transports() -> js_sys::Array {
    supported_transports_vec()
        .into_iter()
        .map(JsValue::from_str)
        .collect()
}

pub fn supported_transports_vec() -> Vec<&'static str> {
    #[cfg(all(target_arch = "wasm32", feature = "browser_libp2p"))]
    {
        // These imports intentionally prove the libp2p umbrella crate exposes
        // the browser transport modules under the selected feature set.
        use libp2p::webrtc_websys as _;
        use libp2p::websocket_websys as _;
        use libp2p::webtransport_websys as _;

        return vec![
            "libp2p-webrtc-websys",
            "libp2p-webtransport-websys",
            "libp2p-websocket-websys",
        ];
    }

    #[cfg(not(all(target_arch = "wasm32", feature = "browser_libp2p")))]
    {
        vec!["identity-only"]
    }
}
```

- [ ] **Step 3: Verify base build still passes**

Run:

```bash
cargo test -p auki-network-browser-wasm
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown
wasm-pack build crates/auki-network-browser-wasm --target nodejs --out-dir pkg-node
node crates/auki-network-browser-wasm/scripts/smoke_import_node.mjs
```

Expected: all commands pass.

- [ ] **Step 4: Run the browser libp2p compile probe**

Run:

```bash
cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p
wasm-pack build crates/auki-network-browser-wasm --target web --out-dir pkg-web -- --features browser_libp2p
```

Expected if the SDK's current libp2p 0.56 stack supports the probe: both commands pass.

If either command fails, do not replace the SDK networking rule with a shortcut. Capture the first concrete blocker by prepending one precise item to `crates/auki-network-browser-wasm/parking_lot.md` under `## Items`. The item must name the exact command that failed, quote the first compiler error that points at the root cause, choose exactly one classification from `dependency/version`, `browser API`, `native listener`, `address advertisement`, `certificate`, or `SDK architecture`, and state the next action. For example, if `cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p` fails because a libp2p browser crate is missing for the selected version, write:

```markdown
- **2026-05-19 — browser_libp2p compile blocker.** `cargo check -p auki-network-browser-wasm --target wasm32-unknown-unknown --features browser_libp2p` failed while enabling rust-libp2p browser features for `wasm32-unknown-unknown`. First failing error: `package libp2p-webrtc-websys was not found for libp2p 0.56`. Classification: dependency/version. Next action: decide whether to update libp2p, reduce the feature set, or try the next transport candidate.
```

The quoted error above is only an example. The committed parking-lot item must use the real command and real first error from the implementation run.

- [ ] **Step 5: Update status docs and changelogs**

If the feature probe passes, update `crates/auki-network-browser-wasm/src/README.md` so "Currently implemented" includes:

```markdown
- rust-libp2p browser transport feature compile probe
```

and remove `rust-libp2p browser feature compile probe` from "Not yet implemented".

If the feature probe fails, keep that item under "Not yet implemented" and add this line to the "Currently implemented" list:

```markdown
- exact `browser_libp2p` compile blocker captured in `parking_lot.md`
```

If the feature probe passes, prepend to `crates/auki-network-browser-wasm/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

Ran the rust-libp2p browser feature compile probe for `wasm32-unknown-unknown`. Outcome: pass. The crate still keeps `auki-domain-browser` fail-closed for Domain join until a browser peer can dial a native SDK probe.
```

If the feature probe fails, prepend to `crates/auki-network-browser-wasm/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

Ran the rust-libp2p browser feature compile probe for `wasm32-unknown-unknown`. Outcome: blocked, with the exact first blocker recorded in `parking_lot.md`. The crate still keeps `auki-domain-browser` fail-closed for Domain join until a browser peer can dial a native SDK probe.
```

Prepend to `crates/changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**[`auki-network-browser-wasm`](auki-network-browser-wasm/changelog.md) — rust-libp2p browser feature compile probe.** Ran the `browser_libp2p` wasm feature check and recorded whether the current SDK stack can compile WebRTC/WebTransport/WebSocket browser transport modules.
```

Prepend to root `changelog.md`:

```markdown
### Nils's codex · May 19, HKT, 2026

**Browser/WASM rust-libp2p feature probe run.** `auki-network-browser-wasm` now records whether the current SDK dependency graph compiles rust-libp2p browser transports for wasm before any Park Domain join work proceeds. See [`crates/changelog.md`](crates/changelog.md) for crate-level propagation.
```

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network-browser-wasm crates/changelog.md changelog.md
git commit -m "test: probe browser libp2p wasm features"
```

---

## Follow-Up Plan Required

After Task 4:

- If `browser_libp2p` passes, write the native probe listener + browser dial implementation plan.
- If `browser_libp2p` fails on WebRTC Direct dependencies, update the plan to try WebTransport-only features before Secure WebSocket.
- In all cases, keep `auki-domain-browser.joinDomain` returning `transport_unavailable` until a browser wasm peer can open an SDK-owned protocol stream to a native SDK peer.

## Self-Review Notes

- Spec coverage: This plan covers the package shape, canonical PeerId export, wasm package importability, and the first rust-libp2p browser transport compile proof. It intentionally stops before native listener/dial because that depends on the compile probe outcome.
- Placeholder scan: No task contains empty placeholder markers. The Task 4 blocker-reporting instructions include one concrete example and require implementers to replace it with actual compiler output if the probe fails.
- Type consistency: The exported names `sdkName`, `peerIdFromSeed`, and `supportedTransports` are stable across Rust, wasm-bindgen, and the JS smoke script.
