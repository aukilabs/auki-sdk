---
name: auki-binding-validation
description: Use when an Auki SDK crate or module with generated Python, Swift, or JavaScript bindings is added or changed and the binding surface must be checked in real generated packages or example apps.
---

# Auki Binding Validation

Validate bindings as consumer APIs, not as generated files. The expected public API must be visible in every enabled binding target and exercised through a language package, smoke test, or small example app.

## Start Here

1. Identify the changed crate and read its three layers: `crates/<crate>/README.md`, `crates/<crate>/src/README.md`, and `crates/<crate>/src/sprint.md`.
2. Check binding policy:

```bash
python3 scripts/bindings/generate_bindings.py plan python <crate>
python3 scripts/bindings/generate_bindings.py plan swift <crate>
python3 scripts/bindings/generate_bindings.py plan javascript <crate>
python3 scripts/bindings/generate_bindings.py list
```

Only generate targets enabled in `crates/<crate>/bindings.toml`.

## Regenerate Targets

Run the relevant root recipes from the repo root:

```bash
just generate-python-bindings <crate>
just generate-swift-bindings <crate>
just generate-javascript-bindings <crate>
```

For fast local Python checks, limit native builds to the host when appropriate:

```bash
AUKI_PYTHON_NATIVE_TARGETS="$(rustc -vV | awk '/host:/ {print $2}')" just generate-python-bindings <crate>
```

Do not hand-edit generated output under `bindings/<language>/<crate>/`. Fix the source surface instead: `src/core.rs`, `src/ffi.rs`, `src/wasm.rs`, `bindings.toml`, or crate-owned `bindings/<language>/` templates and tests.

## Inspect Public API

Run coverage before manual inspection:

```bash
just binding-api-coverage <crate>
just check-binding-api-coverage <crate>
```

Treat gaps as real until proved otherwise. If an item should not be bound, add a code-adjacent `binding-exclude:` comment with a reason.

Then inspect generated package entry points for the API the consumer expects:

- Python: `bindings/python/<crate>/<module>/__init__.py`
- Swift: `bindings/swift/<crate>/generated/` and package sources
- JavaScript: `bindings/javascript/<crate>/index.js`, `index.d.ts`, and wasm-pack declarations

Check names, constructors, records/classes, error shapes, async methods, byte/JSON boundaries, and any `convert_time` or `convert_pose` infrastructure surface the module is meant to expose.

## Prove In A Consumer

Prefer crate-owned smoke tests under `crates/<crate>/bindings/<language>/`; the generator should copy or run them. If the behavior is cross-crate or platform-level, use an existing example app such as an iOS or browser test app. Create a minimal example only when no existing app can prove the new surface.

The proof must call the generated binding API, not Rust internals. It should verify the expected behavior and at least one failure/error path when the new API has validation or runtime errors.

## Done Means

- Enabled binding generation succeeds.
- Expected public API is present in all enabled target packages.
- Smoke test or example app exercises the generated API.
- Rust tests relevant to the changed module pass.
- README/source docs/sprint notes and changelogs are updated using repo propagation rules.
