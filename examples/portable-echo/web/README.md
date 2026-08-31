# Portable echo — Web/Wasm adapter

This example mounts the same Rust echo adapter used by the native example on
an authenticated browser peer. JavaScript composes the generic
`AukiUserSession` and `AukiPeer` facade with the tiny `AukiEcho` protocol
binding. Credential values enter through the login call but are not persisted;
authentication, DDS authority, relay booking, libp2p, authenticated streams,
deadlines, cleanup, and echo framing are implemented in Rust/Wasm.

The browser Peer ID is intentionally ephemeral in v0.1. Log in with
`AukiUserSession.loginDev(...)`, list the accessible Domains, and call
`startPeer(...)` only after the developer has selected one. Every start creates
a fresh identity and relay route. Reloading the page therefore creates a new
peer.

The peer can serve and initiate the exact `/example/echo/1.0.0` protocol. Its
`tcpRoute` is suitable for the native example, while `wssRoute` is suitable for
another browser peer. Every outbound exchange uses the exact WSS route from
the remote peer card, so peers selected onto different DMS relays can connect.
The shared adapter authenticates the stream, runs the Rust protocol, and owns
bounded cleanup. `AukiEcho.close()` is the awaited protocol-unmount barrier and
runs before `AukiPeer.shutdown()`. Trusted discovery from only a Peer ID remains
separate work.

Protocol authors should start with the
[portable authoring workflow](../../../docs/p2p/authoring-protocols.md). This
README keeps the minimal application, richer playground, and protected smoke
test separate on purpose.

## Copy the minimal app

[`minimal.html`](minimal.html) and [`src/minimal.ts`](src/minimal.ts) are the
copyable reference surface. They use the public bindings directly: authenticate
a User, enter an accessible Domain UUID, start an ephemeral relay-backed peer,
mount echo, dial an exact advertised WSS route, and shut the protocol and peer
down in order. There is no peer-card or application framework hidden around
those calls.

Run the development server and open `/minimal.html` in two tabs:

```sh
cd examples/portable-echo/web
npm ci
npm run dev
```

Start both tabs in the same Domain, then copy the Peer ID and WSS route printed
by each tab into the other. Use **Stop peer** to await protocol shutdown and
relay release.

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

Open the printed localhost root URL in two tabs. In each tab, log in, select the
same Domain, and start a peer. Paste the other tab's complete public Peer Card,
enter a message, and send. Use **Stop peer** before closing a tab
so the relay booking is released cleanly. The page never writes credentials or
the ephemeral Peer ID to browser storage.

The playground's Peer Card is application-owned JSON for this example, not a
stable SDK discovery or authorization type. No peer or route is published
automatically in `0.1`.

Build the standalone assets with:

```sh
npm run build
```

## Live direction proof

The playground is intentionally a larger UI example with defensive interaction
and lifecycle handling; it is not the copy-and-paste SDK surface. The protected
smoke harness under `scripts/` is test machinery rather than application code.
It starts two ephemeral peers in separate tabs and one
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
