# auki-domain-browser

TypeScript / browser counterpart to [`auki-domain`](../auki-domain) — the `Peer` contract types and discovery / identity / `installGlobal` helpers that browser-side consumers (Park, future browser SDK users) build against.

Lives in `crates/` (rather than `bindings/`) because it ships its own protocol contract rather than wrapping a Rust crate.

**Status:** WIP (v0.0.0). Contract types and stub modules are in; the wired implementation behind them is not yet built.

## Public surface

- `contract.ts` — `PeerId`, `SensorId`, `DomainName`, `Result<T>`, `PeerError`, `DomainSummary`, `SensorKind`, `SDK_SENSOR_KINDS`, and the `Peer` contract methods.
- `discovery.ts`, `identity.ts`, `peer.ts`, `errors.ts`, `installGlobal.ts` — companion modules per the contract.

Built with `tsc` + tested with `vitest`. See [`package.json`](package.json) for scripts.

## Depends on

Nothing in the Rust workspace. Browser-runtime peer of `auki-domain` rather than a wrapper around it.
