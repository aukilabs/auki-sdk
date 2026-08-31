# Portable echo — Web/Wasm adapter

This example mounts the same Rust echo protocol used by the native example on
an authenticated browser peer. JavaScript sees a small `BrowserUserSession`
and `BrowserEchoPeer`;
credentials, DDS authority, relay booking, libp2p, authenticated streams, and
echo framing stay inside Rust/Wasm.

The browser Peer ID is intentionally ephemeral in v0.1. Log in with
`BrowserUserSession.loginDev(...)`, list the accessible Domains, and call
`startPeer(...)` only after the developer has selected one. Every start creates
a fresh identity and relay route. Reloading the page therefore creates a new
peer.

The peer can serve and initiate the exact `/example/echo/1.0.0` protocol. Its
`tcpRoute` is suitable for the existing native example, while `wssRoute` is
available to another browser peer. Every outbound exchange connects one exact
route, authenticates, runs the shared Rust client, and closes the stream and
route. This manual v0.1 demo accepts only peers using the same DMS-confirmed
relay; trusted multi-relay discovery remains separate work.

Build the package with:

```sh
wasm-pack build examples/portable-echo/web --target web --out-dir pkg-web --dev
```

## Live direction proof

The protected smoke test starts two ephemeral peers in separate tabs, proves
browser A to browser B and browser B to browser A, then runs the native adapter
as a client to browser A. It verifies exact payloads, authenticated Peer IDs,
relayed transport, and clean shutdown on every side. Browser-to-native is added
after the native facade exposes its confirmed WSS reachability.

Install its local tooling once:

```sh
cd examples/portable-echo/web
npm ci
npx playwright install chromium
```

Then provide a dev User and an accessible Domain without putting credentials
in arguments, URLs, storage, traces, or screenshots:

```sh
export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
npm run smoke:dev
```

The runner removes secret-like variables before invoking wasm-pack, Cargo,
Playwright, or Chromium. It passes the selected User credentials only to the
in-memory Wasm authentication call and the native peer process.

Developers with system Chrome can skip the browser download and set
`AUKI_PLAYWRIGHT_CHANNEL=chrome`. The test prints only public Peer IDs and the
validated byte count. It is intentionally not part of credential-free CI.
