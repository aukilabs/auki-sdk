# Portable echo — Web/Wasm adapter

This example mounts the same Rust echo protocol used by the native example on
an authenticated browser peer. JavaScript sees a small `BrowserUserSession`
and `BrowserEchoServer`;
credentials, DDS authority, relay booking, libp2p, authenticated streams, and
echo framing stay inside Rust/Wasm.

The browser Peer ID is intentionally ephemeral in v0.1. Log in with
`BrowserUserSession.loginDev(...)`, list the accessible Domains, and call
`startPeer(...)` only after the developer has selected one. Every start creates
a fresh identity and relay route. Reloading the page therefore creates a new
peer.

The first interoperability proof runs the browser as the echo server. Its
`tcpRoute` is suitable for the existing native example, while `wssRoute` is
available to another browser peer.

Build the package with:

```sh
wasm-pack build examples/portable-echo/web --target web --out-dir pkg-web --dev
```

## Live browser/native proof

The protected smoke test starts the browser as the echo server, then runs the
native adapter as its client through the relay advertised by DMS. It verifies
the exact payload, both authenticated Peer IDs, relayed transport, and clean
shutdown on each side.

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
