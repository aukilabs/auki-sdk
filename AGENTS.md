# Repository Guidelines

## Project Structure & Module Organization

- `core/auki-*/src/` holds stable SDK libraries; `labs/auki-*/src/` holds experimental libraries, including `auki-protocols` and `auki-session`.
- `core/bindings/{python,web,swift,expo}/` holds SDK adapters; `labs/bindings/` holds experimental adapters. Keep shared protocol behavior in Rust.
- `core/examples/` holds networking apps; `labs/examples/` holds experiments. Crate `tests/` holds integration tests and fixtures; `test-support/` holds authentication harnesses.
- `docs/` holds tutorials, how-to guides, explanations, and references for app developers; `README.md` is the entrypoint.

## Build, Test, and Development Commands

Use Rust 1.89.0+ (edition 2024). From the repository root:

- `cargo build --locked -p auki-sdk`: build the native SDK.
- `cargo test --locked -p auki-sdk`: run SDK unit, integration, and documentation tests.
- `cargo test --locked -p auki-protocols --features catalog,stream --test locked_json`: verify canonical fixtures.
- `cargo run --locked -p auki-portable-echo-native`: run echo after configuring credentials, Domain, and identity per its README.

In `core/examples/portable-echo/web/`, follow README prerequisites, run `npm ci`, then `npm run dev`. `npm run build` builds Wasm, checks TypeScript, and bundles assets.

## Coding Style & Naming Conventions

Use four-space Rust indentation, `snake_case` functions/modules, `PascalCase` types, and `SCREAMING_SNAKE_CASE` constants. Follow binding conventions. Check formatting with `cargo fmt --all -- --check`; lint changed crates with `cargo clippy --locked -p <crate> --all-targets -- -D warnings`. Enable relevant protocol features.

## Testing Guidelines

Use Rust's harness, Tokio asynchronous tests, `wasm-bindgen-test`, and pytest. Name tests descriptively in `snake_case`; Python files use `test_*.py`. After installing bindings with Maturin, run `python -m pytest python_tests` where configured. Add behavior regression coverage; preserve locked fixtures. No repository-wide numeric coverage threshold is configured.

## Commit & Pull Request Guidelines

Branch from `develop`, e.g. `fix/351-hide-cleared-traces`. Use Conventional Commits, e.g. `fix(sdk): preserve peer identity`, with why/how/what bodies. PRs should describe changes and validation, link relevant SDK Kanban issues with `Closes #N`, and update affected crate and root READMEs. Move owned cards to **In review** after opening PRs.

## Agent Workflow

Start with [README.md](README.md) and the [app and worker guide](docs/explanation/apps-and-workers.md). Use the [protocol guide](docs/how-to/protocols.md) when adding messages. Resolve routine implementation choices; surface ambiguities affecting scope, contracts, or irreversible actions.

### Project Board

- Obtain authorization before creating cards, including Questions; propose type (Question, Task, or Exploring), title, body, and target column.
- Move only owned cards you actively work on; verify **Done** after merge. Do not modify others' cards.
- Leave unassigned Questions untouched; never self-assign them.
- Never set Priority or Size without explicit instruction.
