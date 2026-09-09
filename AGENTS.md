# Repository Guidelines

## Required Reading & Repository Layout

Before changing behavior, read [networking](docs/explanation/networking.md),
[apps and workers](docs/explanation/apps-and-workers.md), and the affected crate or
binding README. For identity or session changes, also read
[authentication](docs/how-to/authenticate.md) and [peer lifecycle](docs/how-to/lifecycle.md).

`core/` contains the stable SDK, authentication, P2P, relay-booking, and DMS crates;
`labs/` contains experimental protocols and robotics code. Both trees have
`bindings/` and `examples/`. Crate sources live in `src/`, integration tests in
`tests/`, shared test harnesses in `test-support/`, and documentation in `docs/`.

## Architecture Invariants

- `auki-sdk` owns peer lifecycle and connection coordination. P2P carries application
  data; DMS owns task state and leases, and Posemesh runners execute tasks. Do not
  introduce P2P task dispatch or a second scheduler into the SDK.
- Registering a P2P handler does not register a DMS capability or authorize work.
  Keep task admission, domain-data access, and transport authorization separate.
- DDS discovery, DMS relay booking, and DMS task scheduling are separate contracts.
  Relay use does not imply discovery is enabled. Do not conflate this SDK's
  DMS-booked libp2p relay path with legacy HDS/Hagall networking.
- Keep application protocols optional. Do not introduce dependencies from stable
  core crates into `labs/` without an explicit architectural decision.
- Preserve cancellation, bounded retries, authority renewal, and awaited shutdown.
  Changes must not leave background tasks, discovery registrations, or relay
  bookings alive beyond their intended lifecycle.

## Identity & Security Boundaries

- Distinguish local API User login, imported ZITADEL sessions, Auki App credentials,
  DDS compute-node credentials, and DDS robot credentials. They are not
  interchangeable. Keep App secrets on trusted backends, not browser or mobile apps.
- A Peer ID proves transport identity, not a backend principal or permission by
  itself. Preserve DDS-issued P2P credential verification and its binding to the
  transport peer and selected Domain. Never relax signature, issuer, audience,
  expiry, peer, domain, or required scope checks to make a test pass.
- P2P admission does not grant arbitrary application operations, Domain Server
  writes, or DMS task authority. Applications must authorize their own requests;
  do not infer write permission from a successful token exchange or read.
- Preserve imported-session refresh ownership and persistence semantics: one
  refresh owner, awaited replacement-credential writes, and recoverable retained
  session state on persistence failure. Follow the authentication guide for logout.
- Never log or commit passwords, App secrets, raw access/refresh/registration
  tokens, or private peer identity files. Use mocked or redacted credentials in
  examples, fixtures, and reports.

## Backend Contracts & Compatibility

For cross-service changes, inspect the corresponding provider contract rather
than inferring it from SDK types:

- `aukilabs/api`: User/App authentication and service-token exchange.
- `aukilabs/domain-service`: DDS identity, discovery, domain/P2P authorization,
  and Domain Server data access.
- `aukilabs/domain-manager-service`: task/lease and relay-booking contracts.
- `aukilabs/posemesh`: compute/robot runners and corresponding transport behavior.
- `aukilabs/terraform-live`, `aukilabs/terraform-modules`, and
  `aukilabs/argocd-applications`: environment configuration and deployment wiring.

Source support is not proof of deployed support. Identify provider versions and
rollout gates before claiming compatibility, especially for robot and ZITADEL
paths. Changes to wire formats, protocol IDs, token profiles, endpoints, or public
bindings require an explicit compatibility assessment and coordinated provider/
consumer plan when needed. Do not silently change backend contracts to fit the SDK.

## Build, Test & Validation

Use Rust 1.89+ (edition 2024). Run Cargo commands from the repository root:

- `cargo build --locked -p auki-sdk`: build the SDK.
- `cargo test --locked -p auki-sdk`: run its unit, integration, and documentation tests.
- `cargo fmt --all -- --check`: check Rust formatting.
- `cargo clippy --locked -p auki-sdk --all-targets -- -D warnings`: lint the SDK.

Select every changed crate and affected consumer, not only `auki-sdk`: Cargo does
not run dependency crates' own unit suites when testing the facade. For shared
code changes, validate both native and WASM targets and affected Python, Swift,
Web, or Expo bindings using their README commands and test harnesses. A native
`--all-targets` check does not substitute for a WASM check.

In `core/examples/portable-echo/web/`, use `npm ci`, then `npm run check` for WASM
compilation and TypeScript checks or `npm run build` for production assets. Follow
its README for Node and wasm-pack prerequisites. For Python, use a virtual
environment and the affected binding's documented maturin flags before pytest;
for example, the SDK binding documents `maturin develop --locked --no-default-features`.

Rust tests use `#[test]` and `#[tokio::test]`; browser tests use `wasm-bindgen-test`;
Python tests use pytest and `test_*.py`. Use descriptive `snake_case` test names.
Preserve fixtures in `tests/locked/`; do not regenerate expectations merely to
hide incompatible behavior. No numeric coverage threshold is configured.

Add regression tests for behavior changes. Auth changes need negative cases for
wrong audience/domain/peer, expiry, denied permissions, and renewal or persistence
failure. Lifecycle changes need cancellation, shutdown, and relevant relay/discovery
cleanup checks. Documentation-only changes need link/path checks and
`git diff --check`; unrelated runtime suites need not run. Report exact checks and
any unrun platform or integration validation rather than claiming complete coverage.

## Shared-Environment Safety

Default to local fixtures and mocks. Read test-harness setup before running it;
a command being named a test or example does not make it offline.

`cargo run --locked -p auki-portable-echo-native` and interactive examples can
contact shared services, book relays, and publish discovery records. Follow
[the first-peer tutorial](docs/tutorials/first-peer.md) for credentials and identity
setup only after the target environment, test account/Domain, permitted operations,
and cleanup are explicitly approved. Starting a web development server does not
make the app's backend local.

Keep API, DDS, DMS, and separately configured discovery endpoints aligned to the
intended environment. Never silently fall back from local fixtures to shared dev,
staging, or production. Do not deploy, alter infrastructure, provision workers, or
mutate shared data outside the approved test scope. Those actions require separate
authorization; SDK validation is not deployment permission.

## Coding Style & Pull Requests

Use rustfmt with four-space Rust indentation; name functions/modules `snake_case`,
types `UpperCamelCase`, and constants `SCREAMING_SNAKE_CASE`. Match existing
four-space Python and two-space TypeScript formatting. Regenerate bindings/protobuf
output through the relevant build scripts rather than editing generated output.

Use focused task branches and Conventional Commits (`feat:`, `fix:`, `docs:`);
mark breaking changes with `!`, but also explain compatibility impact. Do not push
directly to the base branch. Describe the problem, resulting behavior, affected
crates/platforms and backend contracts, exact validation results, and skipped
checks. Link issues, include screenshots for UI changes, and update relevant docs.
Keep changes focused; merging, deployment, and unrelated backend/infra changes
require separate approval.
