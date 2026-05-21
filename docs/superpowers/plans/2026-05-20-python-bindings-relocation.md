# Python Bindings Relocation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every Python-facing SDK package from `crates/` to `bindings/python/` without changing package names, module names, or runtime behavior.

**Architecture:** `crates/` remains the home for Rust SDK components and adapters, while `bindings/python/` becomes the home for Python packaging surfaces. PyO3 crates remain Cargo workspace members under their new paths; pure Python `auki-datatypes-py` moves with the rest of the Python family. Historical changelog prose is not rewritten.

**Tech Stack:** Rust Cargo workspace, PyO3, maturin, pure Python packaging via `pyproject.toml`, Markdown documentation.

---

### Task 1: Move Python Package Directories

**Files:**
- Move: `crates/auki-datatypes-py` -> `bindings/python/auki-datatypes-py`
- Move: `crates/auki-domain-py` -> `bindings/python/auki-domain-py`
- Move: `crates/auki-identity-py` -> `bindings/python/auki-identity-py`
- Move: `crates/auki-layout-py` -> `bindings/python/auki-layout-py`
- Move: `crates/auki-logs-py` -> `bindings/python/auki-logs-py`
- Move: `crates/auki-manifests-py` -> `bindings/python/auki-manifests-py`
- Move: `crates/auki-network-py` -> `bindings/python/auki-network-py`
- Move: `crates/auki-registry-py` -> `bindings/python/auki-registry-py`
- Move: `crates/auki-session-py` -> `bindings/python/auki-session-py`

- [ ] **Step 1: Create the destination directory**

Run: `mkdir -p bindings/python`
Expected: `bindings/python` exists.

- [ ] **Step 2: Move the Python package directories**

Run one move per package:

```bash
git mv crates/auki-datatypes-py bindings/python/auki-datatypes-py
git mv crates/auki-domain-py bindings/python/auki-domain-py
git mv crates/auki-identity-py bindings/python/auki-identity-py
git mv crates/auki-layout-py bindings/python/auki-layout-py
git mv crates/auki-logs-py bindings/python/auki-logs-py
git mv crates/auki-manifests-py bindings/python/auki-manifests-py
git mv crates/auki-network-py bindings/python/auki-network-py
git mv crates/auki-registry-py bindings/python/auki-registry-py
git mv crates/auki-session-py bindings/python/auki-session-py
```

Expected: `find bindings/python -maxdepth 1 -type d -name 'auki-*-py'` lists all nine Python package directories.

### Task 2: Update Cargo Workspace and Path Dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `bindings/python/auki-domain-py/Cargo.toml`
- Modify: `bindings/python/auki-identity-py/Cargo.toml`
- Modify: `bindings/python/auki-layout-py/Cargo.toml`
- Modify: `bindings/python/auki-logs-py/Cargo.toml`
- Modify: `bindings/python/auki-manifests-py/Cargo.toml`
- Modify: `bindings/python/auki-network-py/Cargo.toml`
- Modify: `bindings/python/auki-registry-py/Cargo.toml`

- [ ] **Step 1: Replace workspace member paths**

In `Cargo.toml`, change each Python workspace member from `crates/auki-*-py` to `bindings/python/auki-*-py`. Leave non-Python Rust members under `crates/`.

- [ ] **Step 2: Update Python crate dependencies on Rust crates**

In moved PyO3 crate `Cargo.toml` files, change dependencies on Rust SDK crates from sibling paths like:

```toml
auki-layout-rs = { package = "auki-layout", path = "../auki-layout" }
```

to paths that climb from `bindings/python/<package>` back to `crates/`:

```toml
auki-layout-rs = { package = "auki-layout", path = "../../../crates/auki-layout" }
```

Apply the same shape for `auki-domain`, `auki-identity`, `auki-network`, `auki-registry`, `auki-logs`, `auki-manifests`, and other Rust SDK dependencies.

- [ ] **Step 3: Update Python crate dependencies on sibling Python crates**

For moved PyO3 crates that depend on other moved PyO3 crates, keep them as siblings under `bindings/python`. Example:

```toml
auki-network-py = { path = "../auki-network-py" }
```

Expected: no `path = "../auki-*"` dependency points accidentally to a missing sibling Rust crate.

### Task 3: Update Current Documentation and Command Paths

**Files:**
- Modify: `README.md`
- Modify: `Glossary.md`
- Modify: `crates/README.md`
- Modify: current README, `src/readme.md`, `src/sprint.md`, and test docstrings under `bindings/python/auki-*-py`
- Do not modify old changelog prose solely to rewrite historical paths.

- [ ] **Step 1: Update root component links**

Replace current documentation links such as:

```markdown
[`auki-domain-py`](crates/auki-domain-py)
```

with:

```markdown
[`auki-domain-py`](bindings/python/auki-domain-py)
```

- [ ] **Step 2: Update command snippets**

Replace active build and test instructions such as:

```bash
maturin develop -m crates/auki-network-py/Cargo.toml
pytest crates/auki-network-py/python_tests/
cd crates/auki-datatypes-py
```

with:

```bash
maturin develop -m bindings/python/auki-network-py/Cargo.toml
pytest bindings/python/auki-network-py/python_tests/
cd bindings/python/auki-datatypes-py
```

- [ ] **Step 3: Update relative Markdown links inside moved packages**

Links from `bindings/python/auki-*-py/README.md` to Rust crates should use `../../../crates/<crate>`. Links between Python packages should stay relative siblings, for example `../auki-network-py`.

### Task 4: Add Relocation Changelog Entries

**Files:**
- Modify: `bindings/python/<package>/changelog.md` for each moved Python package
- Modify: `crates/changelog.md`
- Modify: `changelog.md`

- [ ] **Step 1: Add leaf changelog entries**

At the top of each moved Python package changelog, add a new entry saying the package moved from `crates/<package>` to `bindings/python/<package>` with no runtime or package-name changes.

- [ ] **Step 2: Add parent summaries**

Append a `crates/changelog.md` entry stating that Python binding packages left `crates/`, and append a root `changelog.md` one-liner with the same relocation summary.

Expected: changelog propagation records the current change without rewriting old entries.

### Task 5: Verify Build and Stale Paths

**Files:**
- No intended source edits unless verification reveals broken paths.

- [ ] **Step 1: Run Cargo check**

Run: `cargo check --workspace --exclude auki-network-swift`
Expected: PASS, aside from any pre-existing warnings.

- [ ] **Step 2: Run focused Rust tests for moved PyO3 crates**

Run:

```bash
cargo test -p auki-identity-py -p auki-layout-py -p auki-manifests-py -p auki-registry-py
```

Expected: PASS, aside from any pre-existing warnings.

- [ ] **Step 3: Scan for stale current paths**

Run:

```bash
rg -n "crates/auki-[a-z-]+-py|cd crates/auki-[a-z-]+-py|maturin develop -m crates/auki-[a-z-]+-py" README.md Glossary.md crates bindings -g '!**/changelog.md'
```

Expected: no stale current documentation or command paths remain outside intentionally historical changelog files.
