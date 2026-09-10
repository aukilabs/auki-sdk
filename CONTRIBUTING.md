# Contributing to Auki SDK

Start with [how networking works](docs/explanation/networking.md) and the
README for the crate or binding you want to change. The
[vision](docs/explanation/vision.md) explains the core SDK's direction;
the [glossary](docs/reference/glossary.md) defines its terms.

## Find the right place

| Directory | Use it for |
| --- | --- |
| `core/auki-sdk/` | Peer lifecycle and connection coordination |
| `core/auki-auth/` | Sign-in, sessions, and Domain authorization |
| `core/auki-p2p/` | Authenticated transport and protocol streams |
| `core/auki-relay-booking/` | DMS relay-booking requests and renewal |
| `core/auki-dms/` | DMS task types and client helpers |
| `core/bindings/` and `core/examples/` | Platform APIs and networking examples |
| `labs/` | Experimental protocols, data models, and spatial computing code |
| `test-support/` | Shared test fixtures and harnesses |
| `docs/` | Tutorials, how-to guides, explanations, reference, and the app-builder skill |

Keep application protocols optional. A new dependency from a core crate into
`labs/` needs an architectural decision. P2P carries application data; DMS owns
task state and leases, and Posemesh runners execute tasks. See
[apps, services, compute nodes, and robots](docs/explanation/apps-nodes-and-robots.md)
before changing those boundaries.

## Make a focused change

Use an issue to describe the problem and expected behavior for larger changes.
Start new work on a focused branch from `develop`, the integration branch.
Keep each pull request about one change and include the affected examples and
documentation.

Use rustfmt for Rust. Match the existing four-space Python and two-space
TypeScript formatting. Regenerate bindings and protobuf output through their
build scripts.

For authentication or lifecycle changes, read [repository guidance](AGENTS.md),
[authentication](docs/how-to/authenticate.md), and
[peer lifecycle](docs/how-to/lifecycle.md). Preserve credential checks,
cancellation, renewal, and awaited shutdown.

For backend or wire changes, inspect the provider's contract and identify the
affected consumers. Describe the compatibility impact and any rollout order.
An implementation in a provider repository does not establish support in a
deployed environment.

## Check your change

Use Rust 1.89 or newer. Run Cargo commands from the repository root.
Select every changed crate and affected consumer with `-p`; testing `auki-sdk`
alone does not run its dependencies' own test suites.

For a change confined to `auki-sdk`:

~~~sh
cargo build --locked -p auki-sdk
cargo test --locked -p auki-sdk
cargo fmt --all -- --check
cargo clippy --locked -p auki-sdk --all-targets -- -D warnings
~~~

Add regression tests for changed behavior. For auth changes, cover wrong
audience, Domain, or Peer ID, expired credentials, denied permissions, and
renewal or persistence failure. For lifecycle changes, cover cancellation,
shutdown, and the relevant relay or discovery cleanup. Preserve
fixtures in `tests/locked/`; investigate mismatches before changing expectations.

For code shared with browsers, also check the WASM target. For example:

~~~sh
rustup target add wasm32-unknown-unknown
cargo check --locked -p auki-sdk --target wasm32-unknown-unknown
~~~

Check each affected binding using its README and test harnesses:

- [Web Echo](core/examples/portable-echo/web/README.md): with its prerequisites
  installed, run `npm ci` and `npm run check` in that directory for WASM and
  TypeScript checks; use `npm run build` for production assets.
- [Python](core/bindings/python/auki-sdk-py/README.md): build in a virtual
  environment with the documented Maturin flags before running `python -m pytest`.
- [Swift](core/bindings/swift/auki-sdk-swift/README.md) and
  [Expo](core/bindings/expo/README.md): build with their platform toolchains and
  run the relevant harnesses in `test-support/`.

For documentation-only changes, check relative links, paths, and heading
anchors, then run `git diff --check`. Runtime suites are not needed.

## Use local fixtures first

Read a test harness before running it. The first-peer tutorial and interactive
examples contact shared services, can book relays, and can publish discovery
records. A local web server still uses the app's configured backend.

Before testing against a shared environment, agree on the environment, account
and Domain, permitted operations, and cleanup. Keep API, DDS, DMS, and discovery
URLs aligned. Deployment, infrastructure changes, and provisioning compute
nodes or robots need separate approval.

Keep passwords, App secrets, raw tokens, and private peer identity files out of
logs, fixtures, and commits. Use mocked credentials or redacted examples.

## Open a pull request

Use Conventional Commits, such as `fix: release relay booking on shutdown` or
`docs: clarify Domain selection`. Mark breaking changes with `!` and explain
their compatibility impact. Push your task branch and open a pull request
against `develop`.

Describe the problem, resulting behavior, affected crates and platforms, and
any backend contract changes. List the exact checks that passed and checks you
could not run. Link the relevant issue and include screenshots for UI changes.

Tags mark versions ready for coordinated downstream adoption. Keep SDK,
protocol, and binding revisions aligned when preparing a release.

## Keep the docs useful

Write short, direct instructions for the reader's task. Tutorials should be
runnable, how-to guides should solve one problem, and reference pages should
describe the current API and its limits. Keep future direction in the vision.

Use **compute node** and **robot** for task participants. Use **runner** for
the software that executes their tasks. Update the glossary when introducing
a core term, and update links when moving a page or changing a heading.

For application work with a coding agent, use the
[Auki SDK app-builder skill](docs/skills/auki-sdk-app-builder/SKILL.md).
