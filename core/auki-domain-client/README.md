# auki-domain-client

List Domains and read/write Domain data using the same User/App session as
`AukiPeer`. No peer, relay, discovery registration or DMS configuration is needed.

Start with [Domain data access](../../docs/how-to/domain-data.md). The SDK facade
reexports `AukiDomains`, `AukiDomainData` and their request/response types.

This first milestone provides Rust APIs and compiles for native and WASM.
JavaScript/Python/Swift/Expo data bindings, imported ZITADEL data access,
machine/task adapters, portal/pose wrappers and streaming are follow-ups.
Existing networking bindings remain compatible.

The metadata and multipart operations are adapted from Posemesh's
[`core/domain-http`](https://github.com/aukilabs/posemesh/tree/37db36a50c8b6bacf471d5721e32cd1f88e0e552/core/domain-http).
The existing Posemesh package stays available for its current consumers;
this crate replaces its independent login/cache/background-refresh path with
`auki-auth`, bounded requests and explicit cancellation. It does not depend on
Posemesh or `labs/` at runtime.

Run local fixture tests and portability checks from the repository root:

```sh
cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk
cargo check --locked --target wasm32-unknown-unknown -p auki-auth -p auki-domain-client -p auki-sdk
```

The tests are offline. The documented dev example performs real authentication
and data writes; run it only with an approved account and Domain.
