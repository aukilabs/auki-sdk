# Repository Guidelines

## Project Structure & Module Organization

`core/` contains stable SDK, authentication, P2P, relay-booking, and DMS crates; `labs/` contains experimental protocols and robotics code. Both trees have `bindings/` and `examples/`. Crate sources live in `src/`, integration tests in `tests/`, shared fixtures in `test-support/`, documentation in `docs/`, and camera assets in `labs/examples/camera-mesh/assets/`.

## Build, Test, and Development Commands

Use Rust 1.89+ (edition 2024). Run Cargo commands from the repository root, substituting the affected crate for `auki-sdk`.

- `cargo build --locked -p auki-sdk`: build the SDK.
- `cargo test --locked -p auki-sdk`: run unit, integration, and documentation tests.
- `cargo fmt --all -- --check`: check Rust formatting.
- `cargo clippy --locked -p auki-sdk --all-targets -- -D warnings`: lint the SDK.
- `cargo run --locked -p auki-portable-echo-native`: run Echo after credentials and identity setup in `docs/tutorials/first-peer.md`.

In `core/examples/portable-echo/web/`, run `npm ci`, then `npm run dev` to start Vite. `npm run check` compiles WASM and checks TypeScript; `npm run build` produces production assets. Follow its README for Node and wasm-pack prerequisites.

## Coding Style & Naming Conventions

Use rustfmt with four-space Rust indentation; name functions/modules `snake_case`, types `UpperCamelCase`, and constants `SCREAMING_SNAKE_CASE`. Match existing four-space Python and two-space TypeScript formatting. Keep application protocols optional and regenerate bindings/protobuf output through build scripts.

## Testing Guidelines

Rust tests use `#[test]`, `#[tokio::test]`, and descriptive `snake_case` names. Browser tests use `wasm-bindgen-test` and `test-support/` harnesses. Python tests use pytest and `test_*.py`; run `maturin develop --locked`, then `python -m pytest` in the binding directory with a virtual environment and pytest installed. Preserve fixtures in `tests/locked/`. No numeric coverage threshold is configured; add regression tests for behavior changes.

## Commit & Pull Request Guidelines

Recent history generally uses Conventional Commits: `feat(mappers): ...`, `fix(maps): ...`, and `docs: ...`; mark breaking changes with `!`. Keep changes focused. Describe the problem, resulting behavior, affected crates/platforms, and validation commands. Link related issues, include screenshots for UI changes, and update relevant documentation.

## Configuration & Secrets

Configure examples with `AUKI_EMAIL`, `AUKI_PASSWORD`, and `AUKI_DOMAIN_ID` as documented. Keep credentials and peer identity files out of commits.
