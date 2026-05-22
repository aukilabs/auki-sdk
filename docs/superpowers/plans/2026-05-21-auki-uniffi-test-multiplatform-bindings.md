# Auki UniFFI Test Multiplatform Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `auki-uniffi-test` into a small proving crate with shared Rust core logic, native UniFFI packages for Swift/Python, and wasm-bindgen JavaScript package generation.

**Architecture:** The crate keeps one business-logic implementation in `core.rs`. Native bindings live in `ffi.rs` behind the `uniffi` feature and preserve the existing Swift/Python generation recipes. Browser/JavaScript bindings live in `wasm.rs` behind the `wasm` feature and expose the same behavior through wasm-bindgen.

**Tech Stack:** Rust workspace, UniFFI 0.31, tokio for native async delays, wasm-bindgen, wasm-bindgen-futures, gloo-timers, wasm-pack or wasm-bindgen CLI, cross, Docker, Xcode command line tools, Python packaging through setuptools.

---

## File Structure

- `justfile`: add `install-toolchain`; keep `generate-swift-bindings <crate>` and `generate-python-bindings <crate>`; add `generate-javascript-bindings <crate>`.
- `scripts/install-toolchain.sh`: idempotently installs Rust targets and Cargo CLI tools, then validates external tools.
- `scripts/generate-javascript-bindings.sh`: builds the crate for `wasm32-unknown-unknown` with `--no-default-features --features wasm`, runs JS binding generation, and writes `bindings/javascript/<crate>/`.
- `crates/auki-uniffi-test/Cargo.toml`: split features into `uniffi`, `cli`, and `wasm`; add optional wasm dependencies.
- `crates/auki-uniffi-test/src/core.rs`: pure Rust surface and tests; no UniFFI, wasm-bindgen, tokio, or JS-specific types.
- `crates/auki-uniffi-test/src/ffi.rs`: UniFFI DTOs, errors, exported functions, and `Counter` wrapper over core logic.
- `crates/auki-uniffi-test/src/wasm.rs`: wasm-bindgen DTOs/functions/classes over core logic.
- `crates/auki-uniffi-test/src/lib.rs`: feature-gated module wiring only.
- `crates/auki-uniffi-test/tests/surface.rs`: native UniFFI-facing Rust tests still pass through the default `uniffi` feature.
- `bindings/javascript/auki-uniffi-test/`: generated JavaScript package output.
- `crates/auki-uniffi-test/{README.md,changelog.md,src/readme.md,src/sprint.md}` and propagated changelogs: document the new proving-crate role.

## Task 0: Toolchain Installer

**Files:**
- Modify: `justfile`
- Create: `scripts/install-toolchain.sh`
- Modify: `changelog.md`

- [ ] **Step 1: Add the Just recipe**

Add this recipe to `justfile`:

```just
install-toolchain:
    bash scripts/install-toolchain.sh
```

- [ ] **Step 2: Create the installer script**

Create `scripts/install-toolchain.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required; install it from https://rustup.rs/" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required; install Rust with rustup first" >&2
  exit 1
fi

rust_targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-apple-ios
  aarch64-apple-ios-sim
  x86_64-apple-ios
  wasm32-unknown-unknown
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
)

for target in "${rust_targets[@]}"; do
  rustup target add "$target"
done

cargo install --locked cross
cargo install --locked wasm-pack
cargo install --locked wasm-bindgen-cli

python3 --version
cross --version
wasm-pack --version
wasm-bindgen --version

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for Linux cross builds" >&2
  exit 1
fi
docker info >/dev/null

if [[ "$(uname -s)" == "Darwin" ]]; then
  xcodebuild -version
  xcrun --version
  command -v lipo >/dev/null
fi

echo "Auki SDK toolchain is installed."
```

- [ ] **Step 3: Run the installer**

Run: `just install-toolchain`

Expected: exits 0, installs missing Rust targets/tools, and prints `Auki SDK toolchain is installed.`

- [ ] **Step 4: Commit**

```bash
git add justfile scripts/install-toolchain.sh changelog.md
git commit -m "chore: add binding toolchain installer"
```

## Task 1: Split Pure Core Logic

**Files:**
- Create: `crates/auki-uniffi-test/src/core.rs`
- Modify: `crates/auki-uniffi-test/src/lib.rs`
- Modify: `crates/auki-uniffi-test/tests/surface.rs`

- [ ] **Step 1: Write core tests first**

Add this test module to the bottom of new `core.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_plain_values_match_current_surface() {
        assert_eq!(add(2, 3), 5);
        assert_eq!(hello("Auki"), "Hello, Auki.");
    }

    #[test]
    fn core_greeting_validates_empty_name() {
        let err = make_greeting("", GreetingStyle::Casual).expect_err("empty name rejected");
        assert_eq!(err, TestError::EmptyName);
    }

    #[test]
    fn core_greeting_tracks_style_and_length() {
        let greeting = make_greeting("Auki", GreetingStyle::Formal).expect("valid greeting");
        assert_eq!(greeting.message, "Good day, Auki.");
        assert_eq!(greeting.name_length, 4);
        assert_eq!(greeting.style, GreetingStyle::Formal);
    }

    #[test]
    fn core_counter_holds_state() {
        let mut counter = CounterState::new(10);
        assert_eq!(counter.value(), 10);
        assert_eq!(counter.add(7), 17);
        assert_eq!(counter.value(), 17);
    }

    #[test]
    fn core_delay_validation_rejects_large_delay() {
        assert_eq!(
            validate_delay(1_001),
            Err(TestError::DelayTooLarge { max_ms: 1_000 })
        );
    }
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p auki-uniffi-test core_ -- --nocapture`

Expected: fails because `core.rs` and its types/functions do not exist yet.

- [ ] **Step 3: Implement `core.rs`**

Move behavior into this pure Rust module:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Greeting {
    pub message: String,
    pub name_length: u32,
    pub style: GreetingStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestError {
    EmptyName,
    DelayTooLarge { max_ms: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterState {
    value: i32,
}

impl CounterState {
    pub fn new(initial: i32) -> Self {
        Self { value: initial }
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn add(&mut self, delta: i32) -> i32 {
        self.value += delta;
        self.value
    }
}

pub fn add(left: i32, right: i32) -> i32 {
    left + right
}

pub fn hello(name: &str) -> String {
    format!("Hello, {name}.")
}

pub fn make_greeting(name: &str, style: GreetingStyle) -> Result<Greeting, TestError> {
    if name.is_empty() {
        return Err(TestError::EmptyName);
    }

    let message = match style {
        GreetingStyle::Casual => format!("Hello, {name}."),
        GreetingStyle::Formal => format!("Good day, {name}."),
    };

    Ok(Greeting {
        message,
        name_length: name.chars().count() as u32,
        style,
    })
}

pub fn validate_delay(delay_ms: u32) -> Result<(), TestError> {
    const MAX_DELAY_MS: u32 = 1_000;
    if delay_ms > MAX_DELAY_MS {
        return Err(TestError::DelayTooLarge {
            max_ms: MAX_DELAY_MS,
        });
    }
    Ok(())
}
```

- [ ] **Step 4: Wire `lib.rs` temporarily**

Add `mod core;` to `lib.rs`. Keep the current UniFFI exports in `lib.rs` for this task, converting them to call `core`.

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p auki-uniffi-test`

Expected: all 6 existing surface tests and the new core tests pass.

```bash
git add crates/auki-uniffi-test/src/core.rs crates/auki-uniffi-test/src/lib.rs crates/auki-uniffi-test/tests/surface.rs
git commit -m "refactor: split auki uniffi test core logic"
```

## Task 2: Move UniFFI Surface Into `ffi.rs`

**Files:**
- Create: `crates/auki-uniffi-test/src/ffi.rs`
- Modify: `crates/auki-uniffi-test/src/lib.rs`
- Modify: `crates/auki-uniffi-test/Cargo.toml`

- [ ] **Step 1: Add feature structure**

Update `Cargo.toml`:

```toml
[features]
default = ["uniffi"]
uniffi = ["dep:uniffi", "tokio/rt-multi-thread", "tokio/time"]
cli = ["uniffi", "uniffi/cli"]
wasm = []
```

Make `uniffi` optional:

```toml
uniffi = { version = "0.31", features = ["tokio"], optional = true }
```

- [ ] **Step 2: Move UniFFI declarations into `ffi.rs`**

Create `ffi.rs` with UniFFI DTOs, error mapping, exports, and object wrapper:

```rust
use crate::core;
use std::sync::{Arc, Mutex};
use std::time::Duration;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Greeting {
    pub message: String,
    pub name_length: u32,
    pub style: GreetingStyle,
}

#[derive(uniffi::Error, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TestError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("delay is too large; max {max_ms} ms")]
    DelayTooLarge { max_ms: u32 },
}

impl From<GreetingStyle> for core::GreetingStyle {
    fn from(value: GreetingStyle) -> Self {
        match value {
            GreetingStyle::Casual => Self::Casual,
            GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::GreetingStyle> for GreetingStyle {
    fn from(value: core::GreetingStyle) -> Self {
        match value {
            core::GreetingStyle::Casual => Self::Casual,
            core::GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::Greeting> for Greeting {
    fn from(value: core::Greeting) -> Self {
        Self {
            message: value.message,
            name_length: value.name_length,
            style: value.style.into(),
        }
    }
}

impl From<core::TestError> for TestError {
    fn from(value: core::TestError) -> Self {
        match value {
            core::TestError::EmptyName => Self::EmptyName,
            core::TestError::DelayTooLarge { max_ms } => Self::DelayTooLarge { max_ms },
        }
    }
}

#[uniffi::export]
pub fn add(left: i32, right: i32) -> i32 {
    core::add(left, right)
}

#[uniffi::export]
pub fn hello(name: String) -> String {
    core::hello(&name)
}

#[uniffi::export]
pub fn make_greeting(name: String, style: GreetingStyle) -> Result<Greeting, TestError> {
    core::make_greeting(&name, style.into())
        .map(Into::into)
        .map_err(Into::into)
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn delayed_greeting(name: String, delay_ms: u32) -> Result<Greeting, TestError> {
    core::validate_delay(delay_ms).map_err(TestError::from)?;
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
    }
    core::make_greeting(&name, core::GreetingStyle::Casual)
        .map(Into::into)
        .map_err(Into::into)
}

#[derive(uniffi::Object, Debug)]
pub struct Counter {
    state: Mutex<core::CounterState>,
}

#[uniffi::export]
impl Counter {
    #[uniffi::constructor]
    pub fn new(initial: i32) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(core::CounterState::new(initial)),
        })
    }

    pub fn value(&self) -> i32 {
        self.state.lock().expect("counter mutex poisoned").value()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl Counter {
    pub async fn add_after(&self, delta: i32, delay_ms: u32) -> Result<i32, TestError> {
        core::validate_delay(delay_ms).map_err(TestError::from)?;
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
        }
        Ok(self
            .state
            .lock()
            .expect("counter mutex poisoned")
            .add(delta))
    }
}
```

- [ ] **Step 3: Reduce `lib.rs` to module wiring**

Replace `lib.rs` with:

```rust
//! Small binding-generation proving crate.

pub mod core;

#[cfg(feature = "uniffi")]
mod ffi;

#[cfg(feature = "uniffi")]
pub use ffi::*;
```

- [ ] **Step 4: Verify native recipes still work**

Run:

```bash
cargo test -p auki-uniffi-test
just generate-swift-bindings auki-uniffi-test
just generate-python-bindings auki-uniffi-test
```

Expected: all commands pass and generated Swift/Python APIs keep the same public names.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-uniffi-test/Cargo.toml crates/auki-uniffi-test/src/lib.rs crates/auki-uniffi-test/src/ffi.rs
git commit -m "refactor: isolate auki uniffi test native ffi"
```

## Task 3: Add Wasm-Bindgen Surface

**Files:**
- Create: `crates/auki-uniffi-test/src/wasm.rs`
- Modify: `crates/auki-uniffi-test/src/lib.rs`
- Modify: `crates/auki-uniffi-test/Cargo.toml`

- [ ] **Step 1: Add wasm dependencies**

Update `Cargo.toml`:

```toml
[features]
wasm = ["dep:wasm-bindgen", "dep:wasm-bindgen-futures", "dep:gloo-timers"]

[dependencies]
wasm-bindgen = { version = "0.2", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
gloo-timers = { version = "0.3", features = ["futures"], optional = true }
```

- [ ] **Step 2: Write the wasm wrapper**

Create `wasm.rs`:

```rust
use crate::core;
use gloo_timers::future::TimeoutFuture;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GreetingStyle {
    Casual,
    Formal,
}

#[wasm_bindgen]
pub struct Greeting {
    message: String,
    name_length: u32,
    style: GreetingStyle,
}

#[wasm_bindgen]
impl Greeting {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    #[wasm_bindgen(getter, js_name = nameLength)]
    pub fn name_length(&self) -> u32 {
        self.name_length
    }

    #[wasm_bindgen(getter)]
    pub fn style(&self) -> GreetingStyle {
        self.style
    }
}

#[wasm_bindgen]
pub struct Counter {
    state: RefCell<core::CounterState>,
}

#[wasm_bindgen]
impl Counter {
    #[wasm_bindgen(constructor)]
    pub fn new(initial: i32) -> Self {
        Self {
            state: RefCell::new(core::CounterState::new(initial)),
        }
    }

    pub fn value(&self) -> i32 {
        self.state.borrow().value()
    }

    #[wasm_bindgen(js_name = addAfter)]
    pub async fn add_after(&self, delta: i32, delay_ms: u32) -> Result<i32, JsValue> {
        validate_delay(delay_ms)?;
        if delay_ms > 0 {
            TimeoutFuture::new(delay_ms).await;
        }
        Ok(self.state.borrow_mut().add(delta))
    }
}

#[wasm_bindgen]
pub fn add(left: i32, right: i32) -> i32 {
    core::add(left, right)
}

#[wasm_bindgen]
pub fn hello(name: &str) -> String {
    core::hello(name)
}

#[wasm_bindgen(js_name = makeGreeting)]
pub fn make_greeting(name: &str, style: GreetingStyle) -> Result<Greeting, JsValue> {
    core::make_greeting(name, style.into())
        .map(Into::into)
        .map_err(error_to_js)
}

#[wasm_bindgen(js_name = delayedGreeting)]
pub async fn delayed_greeting(name: String, delay_ms: u32) -> Result<Greeting, JsValue> {
    validate_delay(delay_ms)?;
    if delay_ms > 0 {
        TimeoutFuture::new(delay_ms).await;
    }
    core::make_greeting(&name, core::GreetingStyle::Casual)
        .map(Into::into)
        .map_err(error_to_js)
}

fn validate_delay(delay_ms: u32) -> Result<(), JsValue> {
    core::validate_delay(delay_ms).map_err(error_to_js)
}

fn error_to_js(error: core::TestError) -> JsValue {
    match error {
        core::TestError::EmptyName => JsValue::from_str("name must not be empty"),
        core::TestError::DelayTooLarge { max_ms } => {
            JsValue::from_str(&format!("delay is too large; max {max_ms} ms"))
        }
    }
}

impl From<GreetingStyle> for core::GreetingStyle {
    fn from(value: GreetingStyle) -> Self {
        match value {
            GreetingStyle::Casual => Self::Casual,
            GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::GreetingStyle> for GreetingStyle {
    fn from(value: core::GreetingStyle) -> Self {
        match value {
            core::GreetingStyle::Casual => Self::Casual,
            core::GreetingStyle::Formal => Self::Formal,
        }
    }
}

impl From<core::Greeting> for Greeting {
    fn from(value: core::Greeting) -> Self {
        Self {
            message: value.message,
            name_length: value.name_length,
            style: value.style.into(),
        }
    }
}
```

- [ ] **Step 3: Export wasm module from `lib.rs`**

Add:

```rust
#[cfg(feature = "wasm")]
mod wasm;

#[cfg(feature = "wasm")]
pub use wasm::*;
```

- [ ] **Step 4: Verify wasm compile**

Run:

```bash
cargo build -p auki-uniffi-test --target wasm32-unknown-unknown --no-default-features --features wasm
```

Expected: build exits 0.

- [ ] **Step 5: Commit**

```bash
git add crates/auki-uniffi-test/Cargo.toml crates/auki-uniffi-test/src/lib.rs crates/auki-uniffi-test/src/wasm.rs
git commit -m "feat: add wasm-bindgen auki uniffi test surface"
```

## Task 4: JavaScript Binding Generation

**Files:**
- Modify: `justfile`
- Create: `scripts/generate-javascript-bindings.sh`
- Create: `bindings/javascript/README.md`
- Create: `bindings/javascript/changelog.md`
- Create: `bindings/javascript/parking_lot.md`

- [ ] **Step 1: Add the Just recipe**

Add:

```just
generate-javascript-bindings crate:
    bash scripts/generate-javascript-bindings.sh "{{crate}}"
```

- [ ] **Step 2: Create the JavaScript generation script**

Create `scripts/generate-javascript-bindings.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <crate>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crate="$1"
out_dir="bindings/javascript/${crate}"

if command -v wasm-pack >/dev/null 2>&1; then
  rm -rf "$out_dir"
  wasm-pack build "crates/${crate}" \
    --target web \
    --out-dir "$repo_root/${out_dir}" \
    --no-default-features \
    --features wasm
else
  echo "wasm-pack is required; run just install-toolchain" >&2
  exit 1
fi

echo "Generated JavaScript bindings in $out_dir"
```

- [ ] **Step 3: Add JavaScript bindings docs**

Create `bindings/javascript/README.md`:

```markdown
# JavaScript Bindings

JavaScript-facing SDK packages live here. `auki-uniffi-test` is the proving package for wasm-bindgen generation over shared Rust core logic.
```

Create `bindings/javascript/parking_lot.md`:

```markdown
# Parking lot — JavaScript bindings

Open questions for JavaScript and web bindings.

---

## Package target policy

`auki-uniffi-test` starts with `wasm-pack --target web`. Revisit whether production packages need `bundler`, `nodejs`, or multiple package outputs once a real consumer is identified.
```

Create `bindings/javascript/changelog.md`:

```markdown
# Changelog — JavaScript bindings

Append-only timeline of JavaScript binding changes.

---

### Nils's codex · May 21, HKT, 2026

Added the JavaScript binding family scaffold and `auki-uniffi-test` wasm-bindgen generation target.
```

- [ ] **Step 4: Generate JavaScript package**

Run: `just generate-javascript-bindings auki-uniffi-test`

Expected: `bindings/javascript/auki-uniffi-test/` contains `.js`, `.d.ts`, `.wasm`, and `package.json`.

- [ ] **Step 5: Commit**

```bash
git add justfile scripts/generate-javascript-bindings.sh bindings/javascript
git commit -m "feat: generate javascript bindings for auki uniffi test"
```

## Task 5: JavaScript Smoke Test

**Files:**
- Create: `bindings/javascript/auki-uniffi-test/smoke.mjs`
- Modify: `scripts/generate-javascript-bindings.sh`

- [ ] **Step 1: Add smoke script after generation**

Create `bindings/javascript/auki-uniffi-test/smoke.mjs` after `wasm-pack` output:

```javascript
import init, {
  add,
  hello,
  makeGreeting,
  delayedGreeting,
  Counter,
  GreetingStyle,
} from "./auki_uniffi_test.js";

await init();

if (add(2, 3) !== 5) throw new Error("add failed");
if (hello("JavaScript") !== "Hello, JavaScript.") throw new Error("hello failed");

const greeting = makeGreeting("JavaScript", GreetingStyle.Formal);
if (greeting.message !== "Good day, JavaScript.") throw new Error("formal greeting failed");
if (greeting.nameLength !== 10) throw new Error("nameLength failed");

const delayed = await delayedGreeting("JavaScript", 0);
if (delayed.message !== "Hello, JavaScript.") throw new Error("delayed greeting failed");

const counter = new Counter(10);
if (counter.value() !== 10) throw new Error("counter initial failed");
const updated = await counter.addAfter(7, 0);
if (updated !== 17) throw new Error("counter update failed");
if (counter.value() !== 17) throw new Error("counter final failed");

console.log("javascript wasm smoke ok");
```

- [ ] **Step 2: Run smoke test from generation script**

Append to `scripts/generate-javascript-bindings.sh`:

```bash
node "${out_dir}/smoke.mjs"
```

- [ ] **Step 3: Verify JavaScript generation**

Run: `just generate-javascript-bindings auki-uniffi-test`

Expected: exits 0 and prints `javascript wasm smoke ok`.

- [ ] **Step 4: Commit**

```bash
git add scripts/generate-javascript-bindings.sh bindings/javascript/auki-uniffi-test/smoke.mjs
git commit -m "test: smoke javascript bindings"
```

## Task 6: Documentation and Changelog Propagation

**Files:**
- Modify: `crates/auki-uniffi-test/README.md`
- Modify: `crates/auki-uniffi-test/src/readme.md`
- Modify: `crates/auki-uniffi-test/src/sprint.md`
- Modify: `crates/auki-uniffi-test/changelog.md`
- Modify: `crates/changelog.md`
- Modify: `bindings/changelog.md`
- Modify: `bindings/parking_lot.md`
- Modify: `changelog.md`

- [ ] **Step 1: Update crate README**

Document these commands:

```bash
just install-toolchain
just generate-swift-bindings auki-uniffi-test
just generate-python-bindings auki-uniffi-test
just generate-javascript-bindings auki-uniffi-test
```

State that native Swift/Python use UniFFI, JavaScript uses wasm-bindgen, and both binding layers call `src/core.rs`.

- [ ] **Step 2: Update src/readme.md**

List:

```markdown
- `core.rs` — pure Rust behavior shared by all bindings.
- `ffi.rs` — UniFFI native binding surface.
- `wasm.rs` — wasm-bindgen JavaScript binding surface.
- `bin/uniffi-bindgen.rs` — local UniFFI codegen entry point.
```

- [ ] **Step 3: Update sprint.md**

Move current work to "Done" and set "Next" to applying the same split to the first production SDK component after `auki-uniffi-test` proves the flow.

- [ ] **Step 4: Propagate changelogs**

Add leaf and parent entries that say:

```markdown
`auki-uniffi-test` now proves shared-core multiplatform binding generation: UniFFI for Swift/Python and wasm-bindgen for JavaScript, with `just install-toolchain` covering setup.
```

- [ ] **Step 5: Run final verification**

Run:

```bash
just install-toolchain
cargo test -p auki-uniffi-test
just generate-swift-bindings auki-uniffi-test
just generate-python-bindings auki-uniffi-test
just generate-javascript-bindings auki-uniffi-test
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit**

```bash
git add crates/auki-uniffi-test crates/changelog.md bindings changelog.md justfile scripts docs/superpowers/plans/2026-05-21-auki-uniffi-test-multiplatform-bindings.md
git commit -m "docs: plan auki uniffi test multiplatform bindings"
```
