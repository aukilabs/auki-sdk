# Auki SDK for Web

Use `AukiUserSession` to sign in from JavaScript, access Domain data, and start
an `AukiPeer`. Browser peers use a new Peer ID on each start and connect through
WSS relay addresses.

## Build

Requires Rust 1.89+, wasm-pack 0.13.1, and `wasm32-unknown-unknown`.
From the SDK repository root:

~~~sh
rustup target add wasm32-unknown-unknown
wasm-pack build core/bindings/web/auki-sdk-web --target web --out-dir pkg -- --locked
~~~

Import the generated module and await its default initialization function before
using the bindings. Custom Rust protocols must be compiled into the same Wasm
module; see [custom protocols](../../../../docs/how-to/protocols.md).

## Use the binding

For an app that sends and receives messages, start with
[Web Echo](../../../examples/portable-echo/web/README.md). Choose
`AukiPeerReachabilityMode.OutboundOnly` to make outgoing requests or
`RelayBacked` to also accept incoming requests.

For HTTP data access, use `session.domains()` and `session.data(domain_id)`.
`AukiUserSession.loginDev(email, password, clientId?)` accepts a persistent
installation ID. Close data clients and peers before closing the session.
See [Work with Domain data](../../../../docs/how-to/domain-data.md#use-web-or-python)
for examples and the [reference](../../../../docs/reference/domain-data.md)
for streaming, cancellation, and errors.

For DMS job submission and monitoring, use `session.jobs(domainId)` without
starting a peer. User and imported ZITADEL sessions are supported in browsers;
App secrets are not exposed by this binding. Inputs use typed camel-case fields,
while response fields preserve the DMS wire names:

~~~ts
const jobs = session.jobs(selectedDomainId);
try {
  const spec: JobSpec = {
    label: "prepare-assets",
    tasks: [{
      label: "convert",
      stage: "convert",
      capability: "com.example.convert.v1",
      mode: "dedicated",
      capabilityFilters: { format: "glb" },
    }],
  };
  const estimate = await jobs.estimate(spec);
  const jobId = await jobs.submit(spec);
  const details = await jobs.get(jobId);
  const page = await jobs.list({ capabilities: ["com.example.convert.v1"] });
} finally {
  await jobs.close();
}
~~~

Each operation accepts an optional `AbortSignal`. `AukiJobsError` exposes
`kind`, `code`, and optional `status`; retain and retry an imported session after
`code === "persistence"`. When `kind === "submission_uncertain"`, `source`
identifies the underlying failure and the POST may have succeeded, so reconcile
through `list` before retrying. Close jobs clients before closing their shared
session.

See the [jobs reference](../../../../docs/reference/jobs.md) for required write
authority, worker availability and provider limitations.

Imported ZITADEL sessions support the default `session.domains().list()` query
for a server-paged picker, plus `session.data(knownDomainId)` and selected-Domain
portal/pose reads. The SDK strictly validates the ordinary service-token profile:
human `user-access` uses the deployed User Domain route, while App-shaped viewer
grants are never sent to that broader route and require the permission-scoped
`purpose=p2p` exchange. Organization and Domain Server filters remain unsupported.
Portal-to-Domain association queries also remain unsupported. The session shares
one refresh owner and awaited storage callback across listing, data, and peers.
Retain it after `error.code === "persistence"` and retry after storage recovers. See
[imported login data access](../../../../docs/how-to/domain-data.md#reuse-an-imported-login).

The [Blob/File and Domain-picker helpers](examples/domain-data.ts) request one
explicit Domain page and accept an explicitly selected Domain, file, and
destination callback. The User round trip reads portal/pose records, uploads
the file, streams it back, and deletes its unique record. It contacts the
session's configured services, so use a Domain approved for these operations.

## Check the binding and examples

Requires Node 20.19+ on 20.x or 22.12+. From the repository root:

~~~sh
cd core/bindings/web/auki-sdk-web
npm ci
npm run check
~~~

This builds WASM and checks TypeScript declarations and examples. For local
Chromium integration tests, return to the repository root and set
`WASM_BINDGEN_TEST_RUNNER` to the matching wasm-bindgen 0.2.121 runner:

~~~sh
bash test-support/run-domain-data-browser-tests.sh
~~~

## Fleet inventory and activity

```ts
const fleet = session.fleet(selectedDomainId);
try {
  const inventory = await fleet.list({});
  const candidates = await fleet.computePool({ mode: "dedicated", capabilities: ["vendor.example/inspect/v7"] });
  console.log(inventory.machines, inventory.sources, candidates.complete);
} finally {
  await fleet.close();
  fleet.free();
}
```

Both calls accept an optional `AbortSignal`. Queries use camelCase; typed snapshot
results use snake_case. Errors preserve `kind`, `code`, and available `status`.
Domain members and candidates are separate; missing busy information stays
unknown. See the [fleet reference](../../../../docs/reference/fleet.md).
The repository's `test-support/run-domain-data-browser-tests.sh` includes two
fleet fixtures exercising the actual WASM/Fetch path in Chromium.
