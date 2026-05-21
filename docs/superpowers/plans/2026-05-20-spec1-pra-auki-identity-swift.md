# auki-identity-swift Implementation Plan (Spec 1, PR A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the smallest, foundational slice of the [SDK Swift binding expansion](../specs/2026-05-20-sdk-swift-binding-expansion-design.md): add a new `swift-bindings` cargo feature to `crates/auki-identity` and `crates/auki-network` that gates UniFFI proc-macros on `Wallet` and `PeerIdentity`, and stand up `bindings/swift/auki-identity-swift/` as a thin scaffolding host. After this PR, a Swift consumer can construct a `Wallet`, derive a `PeerIdentity`, and read its `PeerId` string. PR A establishes the feature-flag-and-annotation pattern that PR B (network expansion) and PR C (auki-domain-swift) build on.

**Architecture:** The `swift-bindings` feature is additive-only. When off (the default), the upstream crate compiles exactly as today — no UniFFI dep is pulled in. When on, `#[cfg_attr(feature = "swift-bindings", uniffi::*)]` annotations on existing types tell UniFFI to generate scaffolding. The binding crate `bindings/swift/auki-identity-swift/` is essentially `uniffi::setup_scaffolding!()` plus per-component doc files plus the `build-xcframework.sh` build script — no wrapper structs, no hand-mapped Records, no per-method shims. The only hand-written upstream additions in PR A are: a `wallet_id_str()` helper on `Wallet` (because `WalletId(pub String)` is a tuple struct UniFFI Records can't represent without refactoring), and a `peer_id_string()` helper on `PeerIdentity` (returns the canonical libp2p peer-id string, deriving it from the keypair). Both helpers are gated by `#[cfg(feature = "swift-bindings")]` so they only exist when the feature is on.

**Tech Stack:** Rust 2024 edition, Cargo workspace, UniFFI 0.31 with `tokio` feature (kept available for PR B), `cfg_attr` for feature-gated proc-macros, Xcode 26.3 toolchain for the iOS XCFramework build (validated by Stage 1's PR #152 to work cleanly on `aarch64-apple-ios{,-sim}` / `x86_64-apple-ios`).

---

## File Structure

Files this PR creates or modifies. Each task below names the specific files it touches.

**Upstream Rust crates (annotations + feature):**
- Modify: `crates/auki-identity/Cargo.toml` (add `swift-bindings` feature, optional `uniffi` dep)
- Modify: `crates/auki-identity/src/lib.rs` (annotate `Wallet` + add `wallet_id_str()` helper, both gated)
- Modify: `crates/auki-identity/parking_lot.md` (note the new feature)
- Modify: `crates/auki-identity/changelog.md` (entry)
- Modify: `crates/auki-network/Cargo.toml` (add `swift-bindings` feature + optional `uniffi` dep — only PeerIdentity annotated in PR A; rest lands in PR B)
- Modify: `crates/auki-network/src/lib.rs` (annotate `PeerIdentity` + add `peer_id_string()` helper, both gated)
- Modify: `crates/auki-network/parking_lot.md` (note the new feature; same one that PR B will extend)
- Modify: `crates/auki-network/changelog.md` (entry)
- Modify: `crates/changelog.md` (one-liner per crate change)

**New binding crate:**
- Create: `bindings/swift/auki-identity-swift/Cargo.toml`
- Create: `bindings/swift/auki-identity-swift/.gitignore`
- Create: `bindings/swift/auki-identity-swift/src/lib.rs`
- Create: `bindings/swift/auki-identity-swift/src/bin/uniffi-bindgen.rs`
- Create: `bindings/swift/auki-identity-swift/build-xcframework.sh`
- Create: `bindings/swift/auki-identity-swift/README.md`
- Create: `bindings/swift/auki-identity-swift/parking_lot.md`
- Create: `bindings/swift/auki-identity-swift/changelog.md`
- Create: `bindings/swift/auki-identity-swift/src/readme.md`
- Create: `bindings/swift/auki-identity-swift/src/sprint.md`

**Workspace + indices:**
- Modify: `Cargo.toml` (workspace `members`)
- Modify: `bindings/swift/README.md` (add to per-crate table)
- Modify: `bindings/swift/parking_lot.md` (per-package summary)
- Modify: `bindings/swift/changelog.md` (entry)
- Modify: `bindings/changelog.md` (entry)
- Modify: `changelog.md` (root, one-liner)

---

### Task 1: Add `swift-bindings` cargo feature to `crates/auki-identity`

**Files:**
- Modify: `crates/auki-identity/Cargo.toml`

The feature is empty for now (no annotations exist yet) but wires in the optional `uniffi` dep that Task 2 will use.

- [ ] **Step 1: Show the failing build**

Run:
```bash
cargo build --features swift-bindings -p auki-identity
```

Expected: FAIL with `error: Package 'auki-identity' does not have feature 'swift-bindings'`.

This is the "test" that drives Step 2.

- [ ] **Step 2: Add the optional uniffi dep**

In `crates/auki-identity/Cargo.toml`, locate the `[dependencies]` block:

```toml
[dependencies]
auki-hash = { path = "../auki-hash" }
auki-jcs = { path = "../auki-jcs" }
ed25519-dalek = { version = "2", default-features = false, features = ["fast", "std", "zeroize", "rand_core"] }
rand_core = { version = "0.6", features = ["std", "getrandom"] }
serde = { version = "1", features = ["derive"] }
serde_bytes = "0.11"
serde_json = "1"
```

After the last line of `[dependencies]`, add:

```toml

# Optional — only pulled in when the `swift-bindings` feature is on. Carries
# the UniFFI proc-macros for the Swift binding crate under
# `bindings/swift/auki-identity-swift/`. Default-off so non-Swift consumers
# (Python sidecars, Rust daemons) don't pull uniffi into their dep graph.
uniffi = { version = "0.31", features = ["tokio"], optional = true }
```

- [ ] **Step 3: Add the `[features]` section**

`crates/auki-identity/Cargo.toml` currently has no `[features]` section. Insert one between `[dependencies]` and `[dev-dependencies]`:

```toml

[features]
# Enables UniFFI proc-macros on `Wallet` (and any future identity types
# the Swift binding needs). When off, the crate compiles exactly as today.
swift-bindings = ["dep:uniffi"]
```

- [ ] **Step 4: Verify feature-on build succeeds**

Run:
```bash
cargo build --features swift-bindings -p auki-identity
```

Expected: PASS. No annotations exist yet, so uniffi is just an unused dep; compilation succeeds.

- [ ] **Step 5: Verify feature-off (default) build is unchanged**

Run:
```bash
cargo build -p auki-identity
cargo test -p auki-identity
```

Expected: PASS, identical output to before this task. The feature is opt-in.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-identity/Cargo.toml
git commit -m "feat(auki-identity): add optional swift-bindings cargo feature"
```

---

### Task 2: Annotate `Wallet` with UniFFI proc-macros (feature-gated)

**Files:**
- Modify: `crates/auki-identity/src/lib.rs`

Annotate the `Wallet` struct as a UniFFI `Object`. Expose its constructors (`new`, `from_seed`) and a curated method set (`seed`, `wallet_id_str` — a new helper added in this task). Skip `public_key`, `id`, `sign`, `sign_canonical_json`, `derive_child` — out of PR A's scope per the spec's "Scope" section.

- [ ] **Step 1: Add a feature-gated regression test**

Append to `crates/auki-identity/src/lib.rs` (inside the existing `#[cfg(test)] mod tests` if present; otherwise add one at the bottom of the file):

```rust
#[cfg(all(test, feature = "swift-bindings"))]
mod swift_bindings_tests {
    use super::*;

    /// Deterministic round-trip: same seed → same wallet → same wallet_id.
    /// Asserts the `swift-bindings` feature didn't accidentally change behavior.
    #[test]
    fn wallet_id_str_is_deterministic_for_fixed_seed() {
        let seed = [7u8; 32];
        let w1 = Wallet::from_seed(&seed);
        let w2 = Wallet::from_seed(&seed);
        assert_eq!(w1.wallet_id_str(), w2.wallet_id_str());
    }

    /// `Wallet::new` produces distinct identities on subsequent calls.
    #[test]
    fn wallet_new_returns_distinct_wallets() {
        let a = Wallet::new();
        let b = Wallet::new();
        assert_ne!(a.wallet_id_str(), b.wallet_id_str());
    }

    /// Seed round-trip: write the seed, reconstruct, identity is preserved.
    #[test]
    fn seed_round_trips() {
        let original = Wallet::new();
        let seed = original.seed();
        let reconstructed = Wallet::from_seed(&seed);
        assert_eq!(original.wallet_id_str(), reconstructed.wallet_id_str());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails (helper doesn't exist yet)**

Run:
```bash
cargo test --features swift-bindings -p auki-identity swift_bindings_tests
```

Expected: FAIL — compilation error `no method named 'wallet_id_str' found for struct 'Wallet'`. That's the driver for Step 3.

- [ ] **Step 3: Add the `wallet_id_str` helper and `Object` annotation on Wallet**

Locate the `Wallet` struct definition in `crates/auki-identity/src/lib.rs` (around line 57). Currently:

```rust
pub struct Wallet {
    signing_key: SigningKey,
}
```

Replace with:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct Wallet {
    signing_key: SigningKey,
}
```

Locate the `impl Wallet { ... }` block (starts around line 89). Currently the block does not have the `#[uniffi::export]` attribute. Replace its `impl Wallet {` opening with:

```rust
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl Wallet {
```

Then annotate the two constructors inside the impl block — change `pub fn new() -> Self {` to:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new() -> Self {
```

And `pub fn from_seed(seed: &[u8; 32]) -> Self {` to:

```rust
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
```

Leave the other methods (`seed`, `public_key`, `id`, `sign`, `sign_canonical_json`, `derive_child`) un-annotated for now — `#[uniffi::export]` on the impl block exposes them all by default. **But** UniFFI requires every exported method's signature to be FFI-compatible, and `public_key` / `id` / `sign` / `sign_canonical_json` / `derive_child` return / take non-FFI types (`PublicKey`, `WalletId`, `Signature`, `serde_json::Value`, `&str`). To keep PR A small, we hide them with `#[cfg_attr(not(feature = "swift-bindings"), …)]`-style guards is wrong — `#[uniffi::export]` on the impl block tries to export everything. The right pattern: split the impl block into two.

Split the `impl Wallet { … }` block into two impl blocks:

1. The annotated impl block exposes only the Swift-friendly subset (`new`, `from_seed`, `seed`, plus the new `wallet_id_str`).
2. A second, plain `impl Wallet { … }` block holds the unexposed methods (`public_key`, `id`, `sign`, `sign_canonical_json`, `derive_child`).

The change after Step 3 in `crates/auki-identity/src/lib.rs` looks like (showing the relevant region; preserve existing doc comments verbatim — only the surrounding `impl` blocks change):

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
pub struct Wallet {
    signing_key: SigningKey,
}

// Methods exposed to UniFFI / Swift. Keep small at PR A; expand in
// later PRs as the iosapp consumer needs them.
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl Wallet {
    /// Generate a fresh wallet with a cryptographically random ed25519 keypair.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn new() -> Self {
        let mut csprng = rand_core::OsRng;
        Self {
            signing_key: SigningKey::generate(&mut csprng),
        }
    }

    /// Construct a wallet from a 32-byte seed (the ed25519 secret key bytes).
    /// Same seed → same wallet, deterministically.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    /// 32-byte seed (the ed25519 secret key bytes). Treat as sensitive — anyone
    /// holding these bytes can sign as this wallet.
    pub fn seed(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Content-addressed identity as a plain string. `wallet_id_str` rather
    /// than `wallet_id` because the underlying `WalletId(pub String)` is a
    /// tuple struct which UniFFI's Record macro doesn't directly support; a
    /// `String`-returning method keeps the FFI seam simple. Non-Swift Rust
    /// callers continue to use `Wallet::id()` for the typed `WalletId`.
    pub fn wallet_id_str(&self) -> String {
        self.id().0
    }
}

// Methods that aren't UniFFI-exposed (yet). Future PRs can lift items up
// into the annotated impl block as the iosapp consumer needs them.
impl Wallet {
    /// Public half of the wallet's keypair.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.verifying_key().to_bytes())
    }

    /// Content-addressed identity. Stable for a given pubkey.
    pub fn id(&self) -> WalletId {
        public_key_id(&self.public_key())
    }

    // ... `sign`, `sign_canonical_json`, `derive_child` remain here, verbatim
    // ... from the original impl block. Copy them across — do not delete.
}
```

When applying this edit, copy the existing method bodies for `sign`, `sign_canonical_json`, and `derive_child` from the original `impl Wallet` block into the second (unexposed) impl block verbatim — including their doc comments. Do not change their logic.

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cargo test --features swift-bindings -p auki-identity swift_bindings_tests
```

Expected: PASS — all three tests succeed. `wallet_id_str` exists and is deterministic.

- [ ] **Step 5: Verify the default-feature build and tests still pass**

Run:
```bash
cargo build -p auki-identity
cargo test -p auki-identity
```

Expected: PASS. The default build skips `uniffi`, skips `wallet_id_str` (gated only by `#[uniffi::export]` on its impl block — wait, this is a subtle bug: the `wallet_id_str` method is in the annotated impl block but the impl block's `#[cfg_attr]` is on `uniffi::export`, not on the entire impl. The METHOD still exists when the feature is off; only the `#[uniffi::export]` attribute disappears. So `wallet_id_str` is always present. Confirm by reading the resulting code carefully: yes, `wallet_id_str` is just a regular method when the feature is off — that's the desired behavior, since it's a small public addition useful to any Rust caller, not Swift-specific behavior.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-identity/src/lib.rs
git commit -m "feat(auki-identity): annotate Wallet with UniFFI proc-macros (feature-gated)

Adds the swift-bindings-gated UniFFI Object derivation on Wallet plus
#[uniffi::constructor]/#[uniffi::export] annotations on a curated subset
of methods: new, from_seed, seed, and a new wallet_id_str helper that
returns the WalletId as a plain String (tuple-struct WalletId is not
directly UniFFI-representable).

Methods kept out of the UniFFI export at PR A: public_key, id, sign,
sign_canonical_json, derive_child. They remain on Wallet as a second
non-exported impl block; later PRs can lift them when iosapp needs them."
```

---

### Task 3: Add `swift-bindings` cargo feature to `crates/auki-network`

**Files:**
- Modify: `crates/auki-network/Cargo.toml`

PR A only uses this feature to annotate `PeerIdentity` in `crates/auki-network/src/lib.rs`. PR B will extend it to annotate `NetworkRuntime`, the stream surface, etc. Add the feature now so PR A is the one that establishes the pattern across both upstream crates.

- [ ] **Step 1: Show the failing build**

Run:
```bash
cargo build --features swift-bindings -p auki-network
```

Expected: FAIL with `error: Package 'auki-network' does not have feature 'swift-bindings'`.

- [ ] **Step 2: Add the optional uniffi dep**

In `crates/auki-network/Cargo.toml`, locate the `[dependencies]` section. After the existing non-optional dependencies (`auki-identity`, `libp2p-identity`, `multiaddr`, `serde`, `serde_json`), insert:

```toml

# Optional — pulled in only when the `swift-bindings` feature is on.
# Carries the UniFFI proc-macros for `bindings/swift/auki-network-swift`.
# Default-off so non-Swift consumers (Python sidecars, Rust daemons) don't
# pull uniffi into their dep graph.
uniffi = { version = "0.31", features = ["tokio"], optional = true }
```

- [ ] **Step 3: Add the `swift-bindings` feature to `[features]`**

In `crates/auki-network/Cargo.toml`, locate the `[features]` section. Add a new line after `default = []` and before `swarm = [...]`:

```toml
# Enables UniFFI proc-macros on a curated subset of auki-network's public
# types (PeerIdentity in PR A; NetworkRuntime + the stream surface in
# PR B). When off, the crate compiles exactly as today — no UniFFI dep
# in the graph.
swift-bindings = ["dep:uniffi"]
```

The block ends up looking like:

```toml
[features]
# Default-off so M0 stays WASM-friendly (Console derives peer-id without
# pulling in libp2p's transport stack). Daemons opt in.
default = []
# Enables UniFFI proc-macros on a curated subset of auki-network's public
# types (PeerIdentity in PR A; NetworkRuntime + the stream surface in
# PR B). When off, the crate compiles exactly as today — no UniFFI dep
# in the graph.
swift-bindings = ["dep:uniffi"]
# Stream protocol wire types live in `auki-datatypes` (Step 2 of the …
swarm = ["dep:libp2p", … ]
# … rest unchanged …
```

- [ ] **Step 4: Verify feature-on build succeeds**

Run:
```bash
cargo build --features swift-bindings -p auki-network
```

Expected: PASS. uniffi is wired but unused; compilation succeeds.

- [ ] **Step 5: Verify default + every existing feature still builds**

Run each of these:

```bash
cargo build -p auki-network
cargo build --features swarm -p auki-network
cargo build --features discovery_client -p auki-network
cargo build --features "swarm,discovery_client" -p auki-network
cargo test -p auki-network
```

Expected: all PASS, identical output to before this task. The new feature is fully additive.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/Cargo.toml
git commit -m "feat(auki-network): add optional swift-bindings cargo feature

PR A uses it only to annotate PeerIdentity in lib.rs. PR B extends to
annotate NetworkRuntime + stream surface."
```

---

### Task 4: Annotate `PeerIdentity` with UniFFI proc-macros (feature-gated)

**Files:**
- Modify: `crates/auki-network/src/lib.rs`

The `PeerIdentity` type and the `PEER_DERIVATION_LABEL = "peer/v1"` constant live around lines 80–135 of `crates/auki-network/src/lib.rs`. PR A annotates `PeerIdentity` as a UniFFI `Object` with constructor `from_wallet` and a new `peer_id_string()` helper returning the canonical libp2p peer-id string.

- [ ] **Step 1: Add the feature-gated regression test**

Append to `crates/auki-network/src/lib.rs` (inside the existing `#[cfg(test)] mod tests { ... }` if present; otherwise add one at the bottom of the file):

```rust
#[cfg(all(test, feature = "swift-bindings"))]
mod swift_bindings_tests {
    use super::*;
    use auki_identity::Wallet;

    /// Deterministic: same wallet seed → same PeerId string.
    #[test]
    fn peer_id_string_is_deterministic_for_fixed_seed() {
        let wallet = Wallet::from_seed(&[7u8; 32]);
        let p1 = PeerIdentity::from_wallet(&wallet);
        let p2 = PeerIdentity::from_wallet(&wallet);
        assert_eq!(p1.peer_id_string(), p2.peer_id_string());
        // Spot-check: canonical libp2p PeerId strings start with "12D3KooW".
        assert!(p1.peer_id_string().starts_with("12D3KooW"));
    }

    /// Distinct wallets → distinct peer ids.
    #[test]
    fn distinct_wallets_yield_distinct_peer_ids() {
        let a = PeerIdentity::from_wallet(&Wallet::from_seed(&[1u8; 32]));
        let b = PeerIdentity::from_wallet(&Wallet::from_seed(&[2u8; 32]));
        assert_ne!(a.peer_id_string(), b.peer_id_string());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
cargo test --features swift-bindings -p auki-network swift_bindings_tests
```

Expected: FAIL — compilation error `no method named 'peer_id_string' found for struct 'PeerIdentity'`.

- [ ] **Step 3: Annotate `PeerIdentity` and add `peer_id_string()` helper**

Locate the `PeerIdentity` struct definition in `crates/auki-network/src/lib.rs` (around line 95). Currently:

```rust
#[derive(Clone)]
pub struct PeerIdentity {
    keypair: Keypair,
}
```

Replace with:

```rust
#[cfg_attr(feature = "swift-bindings", derive(uniffi::Object))]
#[derive(Clone)]
pub struct PeerIdentity {
    keypair: Keypair,
}
```

Locate the `impl PeerIdentity { ... }` block (starts around line 101). Split it into two impl blocks the same way Task 2 did for `Wallet`: the annotated block exposes the Swift-friendly subset (`from_wallet`, plus the new `peer_id_string`), the plain block holds the rest (`from_seed`, `keypair`, the existing public-key getter).

The annotated impl block (replace the existing single `impl PeerIdentity { ... }` opening):

```rust
// Methods exposed to UniFFI / Swift. Keep small at PR A; expand in
// later PRs as the iosapp consumer needs them.
#[cfg_attr(feature = "swift-bindings", uniffi::export)]
impl PeerIdentity {
    /// Derive the peer identity from `wallet`. Equivalent to
    /// `PeerIdentity::from_seed(&wallet.derive_child("peer/v1").seed())`.
    /// A backup of the wallet seed is sufficient to regenerate this.
    #[cfg_attr(feature = "swift-bindings", uniffi::constructor)]
    pub fn from_wallet(wallet: &Wallet) -> Self {
        let peer_wallet = wallet.derive_child(PEER_DERIVATION_LABEL);
        Self::from_seed(&peer_wallet.seed())
    }

    /// Canonical libp2p peer-id string (`12D3KooW…`). The Swift side
    /// consumes this as a plain `String`; PR B introduces the
    /// `PeerId`-as-`String` UniFFI custom type that auto-exposes peer-id
    /// arguments and return values across the rest of auki-network's
    /// methods. PR A only needs this one getter, so we expose it as a
    /// pre-stringified helper rather than dragging in the custom-type
    /// registration here.
    pub fn peer_id_string(&self) -> String {
        self.keypair.public().to_peer_id().to_string()
    }
}

// Methods not yet UniFFI-exposed. PR B may lift these up as needed.
impl PeerIdentity {
    /// Construct directly from a 32-byte ed25519 seed. (existing doc.)
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        // ... copy existing body verbatim ...
    }

    /// libp2p `Keypair` for swarm construction in M1. Holds the secret;
    /// don't hand this out beyond the swarm.
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    // ... any other existing methods (e.g. public-key getter) remain here,
    // copied verbatim from the original impl block. Do not delete.
}
```

When applying this edit, copy each existing method body (including `from_seed`, `keypair`, and any public-key getter that exists after line 135 of the file) into the second impl block verbatim. Do not change their logic.

Verify before continuing that the call sites of `PeerIdentity::from_wallet` and the others in the rest of the workspace still compile — `cargo check` from the workspace root.

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cargo test --features swift-bindings -p auki-network swift_bindings_tests
```

Expected: PASS — both tests succeed.

- [ ] **Step 5: Verify default-feature build + every existing feature combo still passes**

Run each of these:

```bash
cargo build -p auki-network
cargo build --features swarm -p auki-network
cargo build --features discovery_client -p auki-network
cargo test -p auki-network
```

Expected: all PASS, identical output to before Task 3 and Task 4 combined.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-network/src/lib.rs
git commit -m "feat(auki-network): annotate PeerIdentity with UniFFI proc-macros (feature-gated)

PR A's slice of the swift-bindings feature on auki-network: annotates
PeerIdentity as a uniffi::Object with from_wallet as a constructor and
a new peer_id_string() helper returning the canonical libp2p peer-id
string. The NetworkRuntime, the stream surface, and the PeerId-as-
String custom type land in PR B."
```

---

### Task 5: Create `bindings/swift/auki-identity-swift/Cargo.toml`

**Files:**
- Create: `bindings/swift/auki-identity-swift/Cargo.toml`

- [ ] **Step 1: Create the crate directory + Cargo.toml**

Run:
```bash
mkdir -p bindings/swift/auki-identity-swift/src/bin
```

Then write `bindings/swift/auki-identity-swift/Cargo.toml`:

```toml
[package]
name = "auki-identity-swift"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "UniFFI Swift bindings for `auki-identity` (Wallet) and the identity-shaped pieces of `auki-network` (PeerIdentity). Thin scaffolding host: the actual UniFFI annotations live on the upstream types behind the `swift-bindings` cargo feature."

[lib]
# `staticlib` is the iOS-consumable artifact; `cdylib` lets host
# uniffi-bindgen introspect it; `rlib` keeps in-workspace Rust consumers
# working. Distinct lib name so the crate coexists with auki-identity's
# default `auki_identity` lib in one workspace, same trick auki-network-py
# uses (`auki_network_py`).
name = "auki_identity_swift"
crate-type = ["staticlib", "cdylib", "rlib"]

[features]
default = []
# Enables the host `uniffi-bindgen` helper binary. Default-off so a plain
# `cargo build`/`cargo test` of the library doesn't pull the UniFFI CLI.
cli = ["uniffi/cli"]

[dependencies]
# Renamed via `package =` so the upstream crate's name doesn't collide
# with our own lib name. The `swift-bindings` feature enables the
# UniFFI proc-macros on Wallet (see crates/auki-identity/Cargo.toml).
auki-identity-rs = { package = "auki-identity", path = "../../../crates/auki-identity", features = ["swift-bindings"] }
# PeerIdentity lives in auki-network's lib.rs; PR A enables only the
# `swift-bindings` feature (not `swarm`/`discovery_client`) so this crate
# stays off the libp2p iOS-link sharp edges.
auki-network-rs = { package = "auki-network", path = "../../../crates/auki-network", features = ["swift-bindings"] }
uniffi = { version = "0.31", features = ["tokio"] }

[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"
required-features = ["cli"]
```

- [ ] **Step 2: Verify the manifest parses + the workspace resolves the deps**

Run:
```bash
cargo metadata --format-version 1 --no-deps > /dev/null
```

Expected: no error, even though the crate isn't yet a workspace member (cargo will warn that it's not in `members`). The package metadata is parseable.

- [ ] **Step 3: Commit**

```bash
git add bindings/swift/auki-identity-swift/Cargo.toml
git commit -m "feat(auki-identity-swift): Cargo.toml for the new binding crate

Thin scaffolding host. Depends on auki-identity + auki-network with the
new swift-bindings feature on each (PR A's slice); does not pull swarm
or discovery_client, so libp2p iOS sharp edges stay out of scope here."
```

---

### Task 6: Create the binding crate's `src/lib.rs` (scaffolding host)

**Files:**
- Create: `bindings/swift/auki-identity-swift/src/lib.rs`

The lib.rs is intentionally tiny: a `setup_scaffolding!()` invocation and `pub use` re-exports so the UniFFI metadata aggregated by `setup_scaffolding!()` covers the upstream-annotated types.

- [ ] **Step 1: Write the failing scaffolding-host test**

Write `bindings/swift/auki-identity-swift/src/lib.rs`:

```rust
//! UniFFI Swift bindings — thin scaffolding host.
//!
//! Per the SDK Swift binding expansion design spec (revision 2), the
//! actual UniFFI proc-macros live on the upstream types under the
//! `swift-bindings` cargo feature. This crate's only job is to call
//! `uniffi::setup_scaffolding!()`, which aggregates the metadata emitted
//! by upstream crates (`auki-identity::Wallet`,
//! `auki-network::PeerIdentity`) into a single `cdylib`/`staticlib` that
//! Swift consumes. The `pub use` re-exports below make the upstream
//! types visible to UniFFI's metadata scanner.

pub use auki_identity_rs::Wallet;
pub use auki_network_rs::PeerIdentity;

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the upstream types are constructable through the
    /// binding crate's re-exports, and the FFI-friendly methods produce
    /// the expected deterministic outputs. This is the proof that the
    /// scaffolding + feature-flagged annotations land coherently.
    #[test]
    fn wallet_and_peer_identity_round_trip_through_re_exports() {
        let wallet = Wallet::from_seed(&[42u8; 32]);
        let wallet_again = Wallet::from_seed(&[42u8; 32]);
        assert_eq!(wallet.wallet_id_str(), wallet_again.wallet_id_str());

        let peer = PeerIdentity::from_wallet(&wallet);
        let peer_again = PeerIdentity::from_wallet(&wallet_again);
        assert_eq!(peer.peer_id_string(), peer_again.peer_id_string());
        assert!(peer.peer_id_string().starts_with("12D3KooW"));
    }
}
```

- [ ] **Step 2: Create the placeholder `uniffi-bindgen` bin**

Write `bindings/swift/auki-identity-swift/src/bin/uniffi-bindgen.rs`:

```rust
//! Host-side Swift codegen entry point. Build with `--features cli`,
//! then `cargo run --features cli --bin uniffi-bindgen -- generate
//! --library <staticlib> --language swift --out-dir <dir>`. Not part of
//! the shipped library. Same pattern as
//! `bindings/swift/auki-network-swift/src/bin/uniffi-bindgen.rs`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
```

- [ ] **Step 3: Add the crate to workspace members**

The crate isn't yet a workspace member, so `cargo build -p auki-identity-swift` would resolve via the global path-dep rules but the workspace's `Cargo.lock` won't include it. Add to `Cargo.toml` (root). Locate the `[workspace]` `members` list and insert (after the existing `bindings/swift/auki-network-swift` line):

```toml
    "bindings/swift/auki-identity-swift",
```

- [ ] **Step 4: Run the test**

Run:
```bash
cargo test -p auki-identity-swift
```

Expected: PASS — the smoke test inside the binding crate succeeds. The `setup_scaffolding!()` macro expands cleanly. UniFFI metadata for `Wallet` and `PeerIdentity` is aggregated by the binding crate.

- [ ] **Step 5: Verify host build + `cli` feature build both succeed**

Run:
```bash
cargo build -p auki-identity-swift
cargo build -p auki-identity-swift --features cli --bin uniffi-bindgen
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add bindings/swift/auki-identity-swift/src/lib.rs \
        bindings/swift/auki-identity-swift/src/bin/uniffi-bindgen.rs \
        Cargo.toml
git commit -m "feat(auki-identity-swift): scaffolding host lib.rs + workspace member

uniffi::setup_scaffolding!() + pub use re-exports of the upstream-
annotated types (Wallet from auki-identity, PeerIdentity from
auki-network's identity-shaped slice). Smoke test asserts the
re-exports compose correctly and the deterministic round-trips
behave the same as the upstream tests. Added to workspace members."
```

---

### Task 7: Add the `.gitignore` + `build-xcframework.sh` build script

**Files:**
- Create: `bindings/swift/auki-identity-swift/.gitignore`
- Create: `bindings/swift/auki-identity-swift/build-xcframework.sh`

Identical pattern to the validated Stage 1 script at `bindings/swift/auki-network-swift/build-xcframework.sh`. Only the crate name + lib name change.

- [ ] **Step 1: Write `.gitignore`**

Write `bindings/swift/auki-identity-swift/.gitignore`:

```
# XCFramework + generated Swift bindings — build output of
# build-xcframework.sh. Not committed (the root .gitignore's `/target`
# is root-anchored and doesn't cover this crate-local dir). The
# distribution decision (committed SwiftPM package vs. downstream build
# step) is tracked in the parent bindings/swift/parking_lot.md.
target-xcframework/
```

- [ ] **Step 2: Write `build-xcframework.sh`**

Write `bindings/swift/auki-identity-swift/build-xcframework.sh`:

```bash
#!/usr/bin/env bash
# Build the auki-identity-swift XCFramework + generated Swift bindings.
#
# Validated on rustc 1.94 + Xcode 26.3 against the three Apple targets
# below. Produces a two-slice AukiIdentity.xcframework (device ios-arm64
# + fat simulator ios-arm64_x86_64-simulator) plus the generated Swift
# glue (auki_identity_swift.swift) in $OUT/swift/, kept *outside* the
# xcframework Headers dir so SwiftPM consumers can pick it up at the
# package level while the xcframework Headers stay clean (FFI header +
# modulemap only).
#
# Same TLS-backend story as auki-network-swift PR #152: rustls `ring`
# 0.17 has first-class iOS cross-compile support so no CC/SDK env
# intervention is required. PR A doesn't pull swarm/libp2p so no
# SystemConfiguration.framework consumer-link concern applies here yet
# (it lands when PR B's network expansion brings the swarm feature in).
#
# Prereqs:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$CRATE_DIR/../../.." && pwd)"
LIB_NAME="auki_identity_swift"
OUT="$CRATE_DIR/target-xcframework"
BINDINGS="$OUT/bindings"
mkdir -p "$BINDINGS"

cd "$WORKSPACE_ROOT"

# 1. Build the static lib for device + both simulator arches.
for TARGET in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p auki-identity-swift --target "$TARGET"
done

# 2. Fat static lib for the simulator (xcframework rejects two slices
#    with the same platform); device lib stays standalone.
DEVICE_LIB="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"
SIM_FAT="$OUT/lib${LIB_NAME}-sim.a"
lipo -create \
  "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
  "target/x86_64-apple-ios/release/lib${LIB_NAME}.a" \
  -output "$SIM_FAT"

# 3. Generate the Swift bindings + FFI header/modulemap from the device
#    static lib. Generating against the device .a is correct: uniffi-bindgen
#    reads arch-independent UNIFFI_META_* symbols via the `object` crate, so
#    the resulting .swift/.h/.modulemap are correct for both slices.
cargo run --release --features cli --bin uniffi-bindgen -- generate \
  --library "$DEVICE_LIB" \
  --language swift \
  --out-dir "$BINDINGS"
# Xcode expects `module.modulemap`.
if [ -f "$BINDINGS/${LIB_NAME}FFI.modulemap" ]; then
  mv "$BINDINGS/${LIB_NAME}FFI.modulemap" "$BINDINGS/module.modulemap"
fi
# The Swift glue file is consumed at the SwiftPM-package level, not
# embedded in the xcframework — move it out of $BINDINGS so step 4's
# `-headers $BINDINGS` packages only the FFI header + modulemap.
SWIFT_OUT="$OUT/swift"
mkdir -p "$SWIFT_OUT"
mv "$BINDINGS/${LIB_NAME}.swift" "$SWIFT_OUT/${LIB_NAME}.swift"

# 4. Assemble the XCFramework (headers = the FFI .h + modulemap only).
rm -rf "$OUT/AukiIdentity.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_LIB" -headers "$BINDINGS" \
  -library "$SIM_FAT"    -headers "$BINDINGS" \
  -output "$OUT/AukiIdentity.xcframework"

echo "XCFramework: $OUT/AukiIdentity.xcframework"
echo "Swift glue : $SWIFT_OUT/${LIB_NAME}.swift"
```

- [ ] **Step 3: Make the script executable**

Run:
```bash
chmod +x bindings/swift/auki-identity-swift/build-xcframework.sh
```

- [ ] **Step 4: Verify .gitignore covers the target output**

Run:
```bash
git check-ignore bindings/swift/auki-identity-swift/target-xcframework
```

Expected: prints `bindings/swift/auki-identity-swift/target-xcframework` (the path is ignored).

- [ ] **Step 5: Commit**

```bash
git add bindings/swift/auki-identity-swift/.gitignore bindings/swift/auki-identity-swift/build-xcframework.sh
git commit -m "feat(auki-identity-swift): build-xcframework.sh + crate .gitignore

Mirrors the validated Stage 1 auki-network-swift script. Produces a
two-slice AukiIdentity.xcframework + auki_identity_swift.swift glue
file in target-xcframework/ (gitignored)."
```

---

### Task 8: Write the per-crate doc files (README, parking_lot, changelog, src/readme, src/sprint)

**Files:**
- Create: `bindings/swift/auki-identity-swift/README.md`
- Create: `bindings/swift/auki-identity-swift/parking_lot.md`
- Create: `bindings/swift/auki-identity-swift/changelog.md`
- Create: `bindings/swift/auki-identity-swift/src/readme.md`
- Create: `bindings/swift/auki-identity-swift/src/sprint.md`

These follow the auki-sdk per-component convention (see `CLAUDE.md` / `CONTRIBUTING.md`). Same shape as `bindings/swift/auki-network-swift/`'s docs.

- [ ] **Step 1: Write `README.md`**

Write `bindings/swift/auki-identity-swift/README.md`:

```markdown
# auki-identity-swift

UniFFI Swift bindings for [`auki-identity`](../../../crates/auki-identity) (`Wallet`) and the identity-shaped slice of [`auki-network`](../../../crates/auki-network) (`PeerIdentity`).

Sibling of [`auki-identity-py`](../../python/auki-identity-py); one binding crate per Rust component, no umbrella `auki-swift`. Thin scaffolding host — actual UniFFI proc-macros live on the upstream types behind the `swift-bindings` cargo feature.

## Surface (target)

```swift
let wallet = Wallet.fromSeed(seed: data32Bytes)   // or Wallet()
let walletId = wallet.walletIdStr()
let peer = PeerIdentity.fromWallet(wallet: wallet)
let peerIdString = peer.peerIdString()            // canonical 12D3KooW…
```

| Swift type | Rust source |
|---|---|
| `Wallet` | `auki_identity::Wallet` |
| `PeerIdentity` | `auki_network::PeerIdentity` |

Out of scope at v0 (PR A): `derive_child`, `sign`/`sign_canonical_json`, `Signature`/`verify`/`CreationCert`. These stay accessible to non-Swift Rust callers via the un-exported `impl` blocks; future PRs lift them if iosapp's features need them.

## Build

Host gate:

```bash
cargo build -p auki-identity-swift
cargo test  -p auki-identity-swift
```

iOS XCFramework:

```bash
bindings/swift/auki-identity-swift/build-xcframework.sh
```

## Status

PR A of [Spec 1](../../docs/superpowers/specs/2026-05-20-sdk-swift-binding-expansion-design.md). See [`src/readme.md`](src/readme.md) for what's implemented and [`src/sprint.md`](src/sprint.md) for what's next.
```

- [ ] **Step 2: Write `parking_lot.md`**

Write `bindings/swift/auki-identity-swift/parking_lot.md`:

```markdown
# Parking lot — auki-identity-swift

Open questions specific to the Swift identity binding.

---

## `WalletId` is hidden behind `wallet_id_str()`

The upstream `WalletId(pub String)` tuple struct isn't a UniFFI `Record` candidate without an upstream refactor (Records require named fields). PR A's binding exposes a `wallet_id_str() -> String` helper instead. If iosapp wants typed handling on the Swift side (`struct WalletId: Hashable { let raw: String }`) that's a thin Swift-side wrapper — no Rust change needed. Revisit if a real consumer needs typed Swift treatment.

## Async-shaped Swift API vs. `-py` sync precedent _(inherited from auki-network-swift)_

Same standing flag-for-human-confirmation as the existing `auki-network-swift` parking lot. Swift's async-await + iOS main-thread rules mean the binding exposes async where the upstream is async; the `-py` precedent is sync. Confirm before any reversal.

## `Wallet`'s other methods (sign, derive_child, sign_canonical_json) not exposed yet

The binding only exposes the v0-essential subset (`new`, `from_seed`, `seed`, `wallet_id_str`). Lift the others into the annotated impl block when a real iosapp feature needs them.
```

- [ ] **Step 3: Write `changelog.md`**

Write `bindings/swift/auki-identity-swift/changelog.md`:

```markdown
# Changelog — auki-identity-swift

Append-only changelog for this crate. See [CLAUDE.md](../../../CLAUDE.md) for the format and propagation rules.

Latest entry on top.

---

### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**New crate: auki-identity-swift (PR A of Spec 1).** Thin scaffolding host for the Swift binding to `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. UniFFI 0.31 proc-macros live on the upstream types behind a new `swift-bindings` cargo feature; this crate is `uniffi::setup_scaffolding!()` + `pub use` re-exports + per-component docs + `build-xcframework.sh`. Surface at PR A: `Wallet::{new, from_seed, seed, wallet_id_str}` and `PeerIdentity::{from_wallet, peer_id_string}`. Discovery / NetworkRuntime / stream surface land in PR B (`auki-network-swift` expansion); ClusterManager in PR C (`auki-domain-swift`).
```

When committing, replace `<YYYY-MM-DD HH:MM HKT>` with the actual commit timestamp.

- [ ] **Step 4: Write `src/readme.md`**

Write `bindings/swift/auki-identity-swift/src/readme.md`:

```markdown
# `auki-identity-swift/src/`

Implementation status for [`auki-identity-swift`](../README.md). Honest about what is real today.

## Files

- [`lib.rs`](lib.rs) — `uniffi::setup_scaffolding!()` + `pub use` re-exports of `auki_identity_rs::Wallet` and `auki_network_rs::PeerIdentity`. A smoke test asserts the re-exports compose correctly.
- [`bin/uniffi-bindgen.rs`](bin/uniffi-bindgen.rs) — host Swift-codegen entry point, gated behind the `cli` feature.

## What works today

- **Host build + tests green.** `cargo build -p auki-identity-swift` and `cargo test -p auki-identity-swift` succeed.
- **Wallet surface**: `from_seed` (constructor), `new` (constructor, CSPRNG), `seed()`, `wallet_id_str()`. UniFFI proc-macros expand the upstream `auki-identity::Wallet` type into a UniFFI `Object` when the upstream `swift-bindings` feature is on (this crate's `Cargo.toml` enables it on the path-dep).
- **PeerIdentity surface**: `from_wallet` (constructor), `peer_id_string()` — returns the canonical libp2p peer-id string.
- **iOS XCFramework build scripted** in `build-xcframework.sh` but not yet validated end-to-end on this crate (it follows the same shape as `auki-network-swift`'s script which was validated by Stage 1 PR #152). Run `bindings/swift/auki-identity-swift/build-xcframework.sh` once to confirm.

## What does NOT work yet

- `derive_child`, `sign`/`sign_canonical_json`, `public_key()`, `id()` (typed `WalletId`) — see `parking_lot.md`.
- Stream surfaces, NetworkRuntime, ClusterManager — those are PRs B and C of Spec 1.

## Rust mapping

| Swift | Rust |
|---|---|
| `Wallet` | `auki_identity::Wallet` |
| `PeerIdentity` | `auki_network::PeerIdentity` |

## Verification

```bash
cargo test -p auki-identity-swift                                          # host gate
cargo build -p auki-identity-swift                                          # host gate
cargo build --features swift-bindings -p auki-identity                      # upstream-feature gate
cargo build --features swift-bindings -p auki-network                       # upstream-feature gate
bindings/swift/auki-identity-swift/build-xcframework.sh                     # iOS XCFramework
```
```

- [ ] **Step 5: Write `src/sprint.md`**

Write `bindings/swift/auki-identity-swift/src/sprint.md`:

```markdown
# Sprint — auki-identity-swift

Closing the gap between [`src/readme.md`](readme.md) (what's implemented) and [the outer README](../README.md) (the spec).

## Now

PR A landed: thin scaffolding host, upstream `swift-bindings` feature on `auki-identity` (Wallet) and `auki-network` (PeerIdentity only). Surface: `Wallet::{new, from_seed, seed, wallet_id_str}`, `PeerIdentity::{from_wallet, peer_id_string}`. Host build + tests green. iOS XCFramework script in place, validated by running `build-xcframework.sh`.

## Next

In priority order:

1. **PR B — `auki-network-swift` expansion.** Annotate `NetworkRuntime`, the stream surface, `PeerLivenessEvent` callback. Adds `PeerId`/`Multiaddr` UniFFI custom types — once those land, the `PeerIdentity::peer_id_string()` helper this crate currently exposes may be replaceable by a direct `peer_id() -> PeerId` method (since `PeerId` would auto-marshal as a String via the custom type). Either way, the helper stays as the v0 surface.
2. **PR C — `auki-domain-swift`.** ClusterManager bootstrap consumes `PeerIdentity` arguments, so this crate's surface is the gate for that consumption.
3. **Spec 2 — iosapp wiring.** Keychain helper consumes `Wallet::{from_seed, generate, seed}` to implement the iOS analogue of `auki-identity::load_or_mint_seed`.

## Open Items

See [`parking_lot.md`](../parking_lot.md). Nothing blocks current consumers.

## Out Of Scope

- Cluster lifecycle / peer enumeration — `auki-domain-swift` (PR C).
- Stream / audio surface — `auki-network-swift` expansion (PR B).
- `Signature` / `verify` / `CreationCert` / `derive_child` / typed `WalletId` — defer until a real iosapp feature needs them.
```

- [ ] **Step 6: Verify the binding crate still builds + tests pass after doc files exist**

Run:
```bash
cargo test -p auki-identity-swift
```

Expected: PASS — doc files don't affect compilation.

- [ ] **Step 7: Commit**

```bash
git add bindings/swift/auki-identity-swift/README.md \
        bindings/swift/auki-identity-swift/parking_lot.md \
        bindings/swift/auki-identity-swift/changelog.md \
        bindings/swift/auki-identity-swift/src/readme.md \
        bindings/swift/auki-identity-swift/src/sprint.md
git commit -m "docs(auki-identity-swift): per-component doc files"
```

---

### Task 9: Update `bindings/swift/` indices

**Files:**
- Modify: `bindings/swift/README.md` (per-crate table)
- Modify: `bindings/swift/parking_lot.md` (per-package summary)
- Modify: `bindings/swift/changelog.md` (entry)

The umbrella `bindings/README.md` and `bindings/parking_lot.md` already mention `swift/`, so no change there.

- [ ] **Step 1: Add `auki-identity-swift` to `bindings/swift/README.md`'s table**

Locate the table under "Swift Bindings" in `bindings/swift/README.md`. It currently has one row for `auki-network-swift`. Add a row above it (alphabetical) for `auki-identity-swift`:

```markdown
| [`auki-identity-swift`](auki-identity-swift) | UniFFI Swift bindings for `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. Thin scaffolding host; UniFFI proc-macros live on the upstream types behind a `swift-bindings` cargo feature. |
```

- [ ] **Step 2: Add per-package summary to `bindings/swift/parking_lot.md`**

In `bindings/swift/parking_lot.md`, locate the "Per-package parking lots" section. Add (alphabetical) before the `auki-network-swift` entry:

```markdown
- [`auki-network-swift/`](auki-network-swift/parking_lot.md) — Stage 1 UniFFI Discovery binding for `aukilabs/iosapp` (landed 2026-05-19, iOS XCFramework validated same day on TLS backend **`ring 0.17`** via reqwest's `rustls-tls` default — not `aws-lc-rs`, which is not pulled); async-shaped Swift API vs the `-py` sync precedent (**flagged for human confirmation**); where generated Swift / XCFramework artifacts live + committed-vs-built distribution; stream-payload parity rule for Stage 2; `with_http` (custom reqwest::Client for proxies/TLS roots/timeouts) deliberately not exposed at Stage 1
```

Wait — that's the EXISTING entry verbatim. Above it, add the new one:

```markdown
- [`auki-identity-swift/`](auki-identity-swift/parking_lot.md) — **new crate, landed <DATE>** (PR A of Spec 1: thin scaffolding host for `Wallet` + `PeerIdentity`); `WalletId` hidden behind a `wallet_id_str()` helper (upstream tuple struct not Record-compatible); async-shaped Swift API vs `-py` sync precedent (inherited from auki-network-swift); `Wallet`'s `sign`/`derive_child`/typed-`WalletId` surfaces deferred until iosapp needs them.
```

Replace `<DATE>` with the date of the PR landing (commit timestamp).

- [ ] **Step 3: Add changelog entry to `bindings/swift/changelog.md`**

Insert above the existing latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**New crate `auki-identity-swift` (PR A of Spec 1).** Thin UniFFI scaffolding host for `auki-identity::Wallet` and the identity-shaped slice of `auki-network::PeerIdentity`. UniFFI proc-macros live on the upstream types behind a new `swift-bindings` cargo feature on each of `crates/auki-identity` and `crates/auki-network` (PR A's slice of the latter). Surface: `Wallet::{new, from_seed, seed, wallet_id_str}`, `PeerIdentity::{from_wallet, peer_id_string}`. Host gate green; iOS XCFramework build scripted.
```

Replace `<YYYY-MM-DD HH:MM HKT>` with the actual commit time.

- [ ] **Step 4: Commit**

```bash
git add bindings/swift/README.md bindings/swift/parking_lot.md bindings/swift/changelog.md
git commit -m "docs(bindings/swift): index updates for auki-identity-swift"
```

---

### Task 10: Propagate the change up to `bindings/changelog.md` and `changelog.md` (root)

**Files:**
- Modify: `bindings/changelog.md` (one-liner)
- Modify: `crates/changelog.md` (one-liner — upstream Cargo.toml + lib.rs changes propagate here)
- Modify: `changelog.md` (root, one-liner)
- Modify: `crates/auki-identity/changelog.md`, `crates/auki-identity/parking_lot.md` (leaf entries for the upstream feature addition)
- Modify: `crates/auki-network/changelog.md`, `crates/auki-network/parking_lot.md` (leaf entries for the upstream feature + PeerIdentity annotation)

Per `CLAUDE.md`'s hierarchical propagation rule: detailed entry at each leaf, one-liner at each parent up to root.

- [ ] **Step 1: Append leaf changelog entry to `crates/auki-identity/changelog.md`**

Insert above the latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**Added optional `swift-bindings` cargo feature.** Enables UniFFI proc-macros on `Wallet` (gated by `#[cfg_attr(feature = "swift-bindings", ...)]` so the feature is fully additive — default-feature build is unchanged). `wallet_id_str() -> String` helper added because the upstream `WalletId(pub String)` tuple struct isn't UniFFI-Record-compatible. The exposed surface at PR A: constructors `new`/`from_seed`, methods `seed`/`wallet_id_str`. `derive_child`/`sign`/etc. remain on `Wallet` via a second non-exported impl block; not exposed to Swift in PR A. Consumed by the new `bindings/swift/auki-identity-swift` crate (Spec 1 PR A).
```

- [ ] **Step 2: Append parking-lot note to `crates/auki-identity/parking_lot.md`**

Insert as a new entry near the top:

```markdown
## `swift-bindings` cargo feature

PR A added an additive `swift-bindings` feature that gates UniFFI proc-macros on the public types. Default behavior unchanged. The feature lives here, not in a separate `*-swift` upstream crate, to keep the binding crate (`bindings/swift/auki-identity-swift`) a thin scaffolding host. As more iosapp features land, the curated subset exposed under the feature may grow — currently: `Wallet::{new, from_seed, seed, wallet_id_str}`.
```

- [ ] **Step 3: Append leaf changelog entry to `crates/auki-network/changelog.md`**

Insert above the latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**Added optional `swift-bindings` cargo feature; annotated `PeerIdentity` (PR A's slice).** Same additive pattern as `auki-identity`'s new feature. PR A's surface: `PeerIdentity::{from_wallet, peer_id_string}`. `peer_id_string()` returns the canonical libp2p peer-id `String` so PR A doesn't yet need the `PeerId`-as-`String` UniFFI custom type (that lands in PR B). PR B will extend the feature to annotate `NetworkRuntime`, `PeerLivenessEvent`, the full stream surface, and the existing Discovery client (replacing Stage 1's hand-written `bindings/swift/auki-network-swift/src/lib.rs` wrappers).
```

- [ ] **Step 4: Append parking-lot note to `crates/auki-network/parking_lot.md`**

Insert as a new entry near the top:

```markdown
## `swift-bindings` cargo feature

PR A added an additive `swift-bindings` feature that currently gates UniFFI proc-macros on `PeerIdentity` only. PR B extends it to `NetworkRuntime`, `PeerLivenessEvent`, the stream surface, and the existing Discovery types (replacing the hand-written Stage 1 binding-crate wrappers). The feature lives here, not in a separate `*-swift` upstream crate.
```

- [ ] **Step 5: Append `crates/changelog.md` one-liner**

Insert above the latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**[`auki-identity`](auki-identity/changelog.md) + [`auki-network`](auki-network/changelog.md) — new optional `swift-bindings` cargo feature.** Additive, feature-gated UniFFI proc-macros on `Wallet` + `PeerIdentity`. Default builds unchanged. Consumed by the new [`auki-identity-swift`](../bindings/swift/auki-identity-swift/changelog.md) binding crate (Spec 1 PR A).
```

- [ ] **Step 6: Append `bindings/changelog.md` one-liner**

Insert above the latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**`auki-identity-swift` added to [`bindings/swift/`](swift/changelog.md).** PR A of Spec 1. Thin UniFFI scaffolding host for `Wallet` + `PeerIdentity` via the upstream `swift-bindings` cargo feature; surface at v0 is the minimum the iosapp Keychain helper needs.
```

- [ ] **Step 7: Append root `changelog.md` one-liner**

Insert above the latest entry:

```markdown
### Nils's claude · <YYYY-MM-DD HH:MM HKT>

**Spec 1 PR A landed: `auki-identity-swift` + upstream `swift-bindings` feature on `auki-identity` and `auki-network`.** New binding crate under `bindings/swift/`, thin scaffolding host; UniFFI proc-macros live on the upstream types behind a new additive cargo feature. Surface at PR A: `Wallet::{new, from_seed, seed, wallet_id_str}`, `PeerIdentity::{from_wallet, peer_id_string}`. PRs B (network expansion) and C (`auki-domain-swift`) follow. See [`bindings/changelog.md`](bindings/changelog.md) and [`crates/changelog.md`](crates/changelog.md) for level-down propagation.
```

- [ ] **Step 8: Verify nothing else changed unexpectedly**

Run:
```bash
git status --short
```

Expected: only the changelog/parking-lot files modified plus the new spec file is staged.

- [ ] **Step 9: Commit**

```bash
git add crates/auki-identity/changelog.md crates/auki-identity/parking_lot.md \
        crates/auki-network/changelog.md crates/auki-network/parking_lot.md \
        crates/changelog.md bindings/changelog.md changelog.md
git commit -m "docs: propagate Spec 1 PR A changelogs + parking-lot leaf-to-root"
```

---

### Task 11: Validate the iOS XCFramework build end-to-end

**Files:**
- (No new files; produces `target-xcframework/` artifacts inside the binding crate dir, all gitignored)

This is the integration test for PR A: confirm the script produces a well-formed XCFramework with the expected Swift surface.

- [ ] **Step 1: Verify iOS Rust targets are installed**

Run:
```bash
rustup target list --installed | grep apple-ios
```

Expected output includes (in any order):
```
aarch64-apple-ios
aarch64-apple-ios-sim
x86_64-apple-ios
```

If any are missing, add them: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`.

- [ ] **Step 2: Verify Xcode is the active developer dir**

Run:
```bash
xcode-select -p
```

Expected: `/Applications/Xcode.app/Contents/Developer` (or another full Xcode path, not `/Library/Developer/CommandLineTools`). If wrong, ask Nils to run `sudo xcode-select -s /Applications/Xcode.app`.

- [ ] **Step 3: Run the build script**

Run:
```bash
./bindings/swift/auki-identity-swift/build-xcframework.sh
```

Expected: ~3–5 minutes (release builds for three targets). Ends with:
```
XCFramework: …/bindings/swift/auki-identity-swift/target-xcframework/AukiIdentity.xcframework
Swift glue : …/bindings/swift/auki-identity-swift/target-xcframework/swift/auki_identity_swift.swift
```

- [ ] **Step 4: Inspect the XCFramework's slices**

Run:
```bash
find bindings/swift/auki-identity-swift/target-xcframework/AukiIdentity.xcframework -name "*.a" -exec sh -c 'echo "{}:"; lipo -info "{}"' \;
```

Expected:
```
…/ios-arm64/libauki_identity_swift.a:
Non-fat file: …/ios-arm64/libauki_identity_swift.a is architecture: arm64
…/ios-arm64_x86_64-simulator/libauki_identity_swift-sim.a:
Architectures in the fat file: …/ios-arm64_x86_64-simulator/libauki_identity_swift-sim.a are: x86_64 arm64
```

- [ ] **Step 5: Inspect the generated Swift surface**

Run:
```bash
grep -nE "class Wallet|class PeerIdentity|func (fromSeed|new|seed|walletIdStr|fromWallet|peerIdString)" \
  bindings/swift/auki-identity-swift/target-xcframework/swift/auki_identity_swift.swift | head -30
```

Expected: lines showing `open class Wallet`, `open class PeerIdentity`, and class methods / functions for `fromSeed`, `new` (or `init`), `seed`, `walletIdStr`, `fromWallet`, `peerIdString`. Exact spellings depend on UniFFI 0.31's Swift codegen (`fromSeed` becomes a class function or initializer).

- [ ] **Step 6: Verify the gitignore covers the build output**

Run:
```bash
git status --porcelain | grep "target-xcframework"
```

Expected: empty output — `target-xcframework/` is ignored.

- [ ] **Step 7: No commit**

This task produces only gitignored build artifacts; nothing to commit. The validation is the gate that the PR is mergeable.

---

## Self-Review

(Run by the plan author after writing — not the implementer.)

**Spec coverage.** Walk the Spec 1 "Per-crate plan" → `auki-identity-swift` section and check each requirement maps to a task above:

- ✅ `swift-bindings` feature on `auki-identity` (Task 1) and on `auki-network` (Task 3).
- ✅ `Wallet` annotated as `uniffi::Object` with `new`/`from_seed` constructors and `seed`/`wallet_id_str` exposed (Task 2).
- ✅ `PeerIdentity` annotated as `uniffi::Object` with `from_wallet` constructor and `peer_id_string` exposed (Task 4).
- ✅ Binding crate `Cargo.toml` enables `swift-bindings` on each path-dep (Task 5).
- ✅ Binding crate `lib.rs` is `setup_scaffolding!() + pub use` re-exports (Task 6).
- ✅ Workspace `Cargo.toml` adds the new member (Task 6 Step 3).
- ✅ `build-xcframework.sh` + `.gitignore` (Task 7).
- ✅ Per-crate doc files (Task 8).
- ✅ `bindings/swift/` index updates (Task 9).
- ✅ Changelog + parking-lot propagation leaf-to-root, including upstream crates (Task 10).
- ✅ iOS XCFramework end-to-end validation (Task 11).

**Placeholder scan.** Searched the plan for "TBD", "TODO", "implement later", "etc." — all instances of "etc." are in prose context (e.g. "stream / etc."), not as placeholders for missing tasks. The `<YYYY-MM-DD HH:MM HKT>` and `<DATE>` placeholders in changelog snippets are correctly framed as "replace with commit timestamp" — these are template values the implementer fills in at commit time, not unfinished plan content.

**Type consistency.** Method names across tasks: `wallet_id_str` (Task 2 introduces, Task 6 smoke-tests, Tasks 8/9/10 reference). `peer_id_string` (Task 4 introduces, Task 6 smoke-tests, Tasks 8/9/10 reference). Class/struct names: `Wallet`, `PeerIdentity` (consistent everywhere). No drift.

**Bite-sized granularity.** Each task has 5–9 steps, each step is one action (write code / run command / read output / commit). Largest task is Task 8 (doc files) with seven file-writes; reasonable since each file write is ~30 lines of templated content.
