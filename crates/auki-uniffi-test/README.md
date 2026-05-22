# auki-uniffi-test

`auki-uniffi-test` is the workspace example for turning a Rust crate into generated bindings with the root `just` recipes.

It is deliberately small and does not wrap a production Auki SDK component. Its job is to show the crate structure, feature flags, exported API shapes, and generated package layout an agent should copy when adding bindings to a real crate.

Use this crate as the reference when you need to answer either question:

- "How do I make a Rust crate generate Swift and Python bindings with UniFFI?"
- "How do I expose the same Rust behavior to JavaScript/WebAssembly with wasm-bindgen?"

## The pattern

The core rule is: keep product logic binding-free, then build thin platform surfaces around it.

This crate has three implementation layers:

- [`src/core.rs`](src/core.rs) — shared Rust behavior with no UniFFI, wasm-bindgen, Tokio, JavaScript, or Swift/Python assumptions.
- [`src/ffi.rs`](src/ffi.rs) — the native UniFFI surface used by Swift and Python generation.
- [`src/wasm.rs`](src/wasm.rs) — the JavaScript/WebAssembly surface used by wasm-bindgen.

[`src/lib.rs`](src/lib.rs) wires those layers together with feature gates:

- native/default builds enable `uniffi`, compile `ffi.rs`, and re-export the UniFFI API.
- wasm builds use `--no-default-features --features wasm`, compile `wasm.rs`, and re-export the wasm-bindgen API.
- `wasm32 + uniffi` is rejected with a compile error because this crate treats UniFFI as the native binding path and wasm-bindgen as the web binding path.

This split keeps the behavior testable as ordinary Rust and lets each binding generator use the shapes it supports best.

## Why `core.rs`, `ffi.rs`, and `wasm.rs` are separate

Bindings are not neutral wrappers. UniFFI and wasm-bindgen need different exported types and async runtimes:

- UniFFI records, enums, errors, objects, constructors, and async functions are annotated with `uniffi::*` macros.
- UniFFI async exports in this crate run on Tokio, so the native binding feature owns the Tokio dependency.
- wasm-bindgen classes and functions are annotated with `#[wasm_bindgen]`.
- wasm-bindgen exposes JavaScript-friendly names such as `makeGreeting`, `delayedGreeting`, and `Counter.addAfter`.
- wasm async uses browser/JavaScript promises through `wasm-bindgen-futures` and `gloo-timers`, not Tokio.
- wasm errors are converted to `JsValue`/`js_sys::Error`; UniFFI errors are typed Rust enums deriving `uniffi::Error` and `thiserror::Error`.

When adding bindings to a real SDK crate, avoid putting these generator-specific concerns in the shared core. Add adapter types and `From` conversions in the binding layer instead.

## Cargo manifest standard

The important parts of [`Cargo.toml`](Cargo.toml) are the crate type, feature flags, optional dependencies, and bindgen helper binary.

### Library crate types

```toml
[lib]
name = "auki_uniffi_test"
crate-type = ["staticlib", "cdylib", "rlib"]
```

Use a stable underscore library name. The binding generator reads this value from Cargo metadata and uses it for native library paths, generated module names, and template variables. If `[lib].name` is omitted, Cargo's normal hyphen-to-underscore default applies.

The crate types serve different consumers:

- `staticlib` — required for the Swift XCFramework path.
- `cdylib` — required for dynamic native loading, including Python UniFFI packages.
- `rlib` — keeps the crate usable from Rust tests and workspace crates.

### Features

```toml
[features]
default = ["uniffi"]
uniffi = [
    "dep:uniffi",
    "dep:tokio",
    "dep:thiserror",
    "tokio/time",
]
cli = ["uniffi", "uniffi/cli"]
wasm = [
    "dep:wasm-bindgen",
    "dep:wasm-bindgen-futures",
    "dep:gloo-timers",
    "dep:js-sys",
]
```

The feature contract is:

- `default = ["uniffi"]` makes ordinary `cargo build -p <crate>` and `cargo test -p <crate>` exercise the native binding surface.
- `uniffi` enables only the dependencies needed by `ffi.rs`.
- `cli` enables the local `uniffi-bindgen` binary used by the generation scripts.
- `wasm` enables only the dependencies needed by `wasm.rs`.

Keep generator dependencies optional. A production crate should not force Swift/Python-only dependencies into JavaScript builds, and it should not force wasm-only dependencies into native builds.

### Target-specific dependencies

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1", default-features = false, optional = true }
uniffi = { version = "0.31", features = ["tokio"], optional = true }
thiserror = { version = "2", optional = true }
```

Native-only dependencies belong behind `cfg(not(target_arch = "wasm32"))`. That prevents wasm builds from resolving libraries that cannot compile for the web target.

The wasm dependencies are optional normal dependencies because they are only activated by the `wasm` feature.

### Bindgen helper binary

```toml
[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"
required-features = ["cli"]
```

Each UniFFI-capable crate carries a tiny local bindgen entry point:

```rust
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

The root scripts invoke this binary with `cargo run -p <crate> --features cli --bin uniffi-bindgen -- generate ...`. This keeps the UniFFI CLI version tied to the crate dependency graph instead of relying on a globally installed binary.

### Tests

```toml
[[test]]
name = "surface"
path = "tests/surface.rs"
required-features = ["uniffi"]
```

Native binding surface tests should require the `uniffi` feature. Keep pure Rust behavior tests in `core.rs` where possible, because they are cheaper and do not depend on binding generation.

## Binding generation ownership

The root scripts are generic. A Rust crate owns the binding-specific contract and package assets needed to generate correct language packages.

For this crate, the source of truth is:

```text
crates/auki-uniffi-test/
  Cargo.toml
  bindings.toml
  bindings/
    javascript/
      package.json.tmpl
      README.md.tmpl
      smoke.mjs
    python/
      pyproject.toml.tmpl
      setup.py.tmpl
      README.md.tmpl
    swift/
      Package.swift.tmpl
```

`Cargo.toml` owns crate identity: package name, version, description, license, repository, authors, library name, features, and the local `uniffi-bindgen` binary target. The generator reads these values through `cargo metadata`; do not repeat them in `bindings.toml`.

`bindings.toml` owns binding policy:

- which generator each language uses (`uniffi` or `wasm_pack`);
- generator feature flags such as `cli` and `wasm`;
- which language outputs are enabled;
- which crate-local templates and smoke tests are used.

The crate-local `bindings/<language>/` directories own package-specific material. The root generator renders those templates into the consumer-facing package directories under root `bindings/<language>/<crate>/`.

The root scripts should not know about concrete crates, generated package templates, or crate-specific smoke tests. They only load Cargo metadata, read `bindings.toml`, run the requested generator, render the crate-owned package assets, and run the crate-owned smoke test when configured.

## Export surface

The proving API intentionally covers the shapes a production SDK binding is likely to need:

- plain sync functions: `add`, `hello`
- records/classes: `Greeting`
- enums: `GreetingStyle`
- typed errors: `TestError`
- async functions: `delayed_greeting`
- stateful objects/classes: `Counter`
- async object methods: `Counter::add_after` / `Counter.addAfter`

When updating a production crate, map every exported item through the same checklist. If the binding generator needs a different shape than the core API, keep the difference in `ffi.rs` or `wasm.rs`.

## Just recipes

Install or converge the local binding-generation toolchain first:

```bash
just install-toolchain
```

That checks the local prerequisites and installs the pinned Rust targets and Cargo CLIs used by the binding scripts.

The public recipes stay small and stable:

```bash
just generate-swift-bindings <crate>
just generate-python-bindings <crate>
just generate-javascript-bindings <crate>
```

Each recipe delegates to [`scripts/bindings/generate_bindings.py`](../../scripts/bindings/generate_bindings.py), which reads `crates/<crate>/bindings.toml` and Cargo metadata.

### Swift

```bash
just generate-swift-bindings auki-uniffi-test
```

The Swift recipe renders this crate's `bindings/swift/Package.swift.tmpl`, builds the native UniFFI library, runs the crate-local `uniffi-bindgen` helper, and builds the Apple XCFramework path. Generated Swift artifacts land under:

```text
bindings/swift/auki-uniffi-test/generated/
```

The static Swift package root lives at:

```text
bindings/swift/auki-uniffi-test/
```

The XCFramework build requires macOS because it uses `xcodebuild` and `lipo`.

### Python

```bash
just generate-python-bindings auki-uniffi-test
```

The Python recipe builds the native UniFFI library, runs the crate-local `uniffi-bindgen` helper, patches the generated loader for the repo package layout, renders this crate's Python package templates, and builds/copies native libraries. Generated package files land under:

```text
bindings/python/auki-uniffi-test/
```

The generated Python API is written to:

```text
bindings/python/auki-uniffi-test/auki_uniffi_test/__init__.py
```

Native libraries are copied under:

```text
bindings/python/auki-uniffi-test/auki_uniffi_test/native/<rust-target>/
```

Default Linux native-library builds use `cross`, so Docker must be running for that part of the recipe.

### JavaScript/WebAssembly

```bash
just generate-javascript-bindings auki-uniffi-test
```

The JavaScript recipe runs `wasm-pack` against `crates/auki-uniffi-test` with the crate-configured `wasm_pack` generator:

```bash
--target web --no-default-features --features wasm
```

Generated package files land under:

```text
bindings/javascript/auki-uniffi-test/
```

The package contains the ESM glue, TypeScript declarations, compiled `.wasm`, rendered package metadata, rendered README, and crate-owned `smoke.mjs`. The recipe stages output in a temporary directory, replaces the final package only after generation succeeds, and runs the smoke test at the end.

## How to make another crate follow this standard

1. Put binding-free behavior in a core module or an existing production crate API. Keep it free of `uniffi`, `wasm-bindgen`, JavaScript types, and generated-package assumptions.
2. Add a native `ffi.rs` surface for Swift/Python with UniFFI macros, typed errors, records, objects, constructors, and async runtime annotations.
3. Add a web `wasm.rs` surface for JavaScript/WebAssembly with wasm-bindgen macros, JavaScript naming, promise-based async, and `JsValue` errors.
4. Gate modules in `lib.rs` so default native builds use `uniffi`, wasm builds use `wasm`, and unsupported feature/target combinations fail clearly.
5. Configure `[lib]` with `staticlib`, `cdylib`, and `rlib`.
6. Keep `uniffi`, `cli`, and `wasm` features separate, with binding-generator dependencies marked `optional = true`.
7. Add the crate-local `src/bin/uniffi-bindgen.rs` helper and gate it behind `required-features = ["cli"]`.
8. Add `bindings.toml` and crate-local templates/smoke tests under `bindings/<language>/`.
9. Add Rust tests for the core behavior and at least one native surface test behind `required-features = ["uniffi"]`.
10. Run the relevant root recipe:

```bash
just generate-swift-bindings <crate>
just generate-python-bindings <crate>
just generate-javascript-bindings <crate>
```

11. Update the crate README, `src/README.md`, `src/sprint.md`, and changelogs according to the repo propagation rules.

Do not copy this crate's toy API into production crates. Copy the structure, feature contract, crate-local binding contract, generation path, and separation between core logic and binding adapters.

## Local verification

From the repo root:

```bash
cargo test -p auki-uniffi-test
cargo build -p auki-uniffi-test
cargo build -p auki-uniffi-test --target wasm32-unknown-unknown --no-default-features --features wasm
```

Run language package generation with the `just` recipes above when changing the binding surfaces or scripts.
