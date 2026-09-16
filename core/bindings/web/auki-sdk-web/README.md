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
