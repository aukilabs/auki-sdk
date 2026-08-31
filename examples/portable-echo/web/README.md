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

## Run the Peer Playground

Prerequisites:

- Rust 1.89.0 or newer
- Node.js 20.19 or newer (or 22.12 or newer)
- `wasm-pack` 0.13.1
- the `wasm32-unknown-unknown` Rust target

Install the pinned Wasm builder and target once if needed:

```sh
cargo install wasm-pack --version 0.13.1 --locked
rustup target add wasm32-unknown-unknown
```

```sh
cd examples/portable-echo/web
npm ci
npm run dev
```

Open the printed localhost URL in two tabs. In each tab, log in, select the
same Domain, and start a peer. Paste the other tab's public Peer Card (or only
its Peer ID), enter a message, and send. Use **Stop peer** before closing a tab
so the relay booking is released cleanly. The page never writes credentials or
the ephemeral Peer ID to browser storage.

Build the standalone assets with:

```sh
npm run build
```

## Live direction proof

The protected smoke test starts two ephemeral peers in separate tabs and one
native peer. It proves browser A to browser B, browser B to browser A, native to
browser A, and browser A back to the same native peer. It verifies exact
payloads, authenticated Peer IDs, confirmed TCP/WSS relay reachability, and
clean shutdown on every side.

Install Chromium for the protected smoke once if it is not already available:

```sh
cd examples/portable-echo/web
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
