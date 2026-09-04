# Portable echo in Web/Wasm

This directory is the copyable Web host for the shared Rust portable-echo
protocol. The root page composes the generic `AukiUserSession` and `AukiPeer`
bindings with the small `AukiEcho` binding; it does not reimplement the echo
wire format in TypeScript.

The page logs in a User, fetches their accessible Domains, starts an ephemeral
browser peer in the selected Domain, and refreshes peers advertising the exact
Echo protocol. Reachability is explicit: **Outbound only** starts no relay
booking and mounts `AukiEchoClient`, while **Inbound + outbound** books a relay
and mounts the serving `AukiEcho` endpoint. A developer selects one untrusted
candidate before the exact WSS operation tries its compatible routes and
authenticates it. Rust/Wasm owns authentication, DDS authority, optional relay
booking, libp2p, authenticated streams, protocol framing, deadlines, and
cleanup. JavaScript only owns the form and lifecycle wiring.

Browser identity is intentionally ephemeral in `0.1`: every start creates a new
Peer ID. A relay-backed start also owns one atomic TCP/WSS route pair; an
outbound-only start exposes neither route. The page does not persist credentials
or peer state.

The app defaults to **Outbound only** plus **Discover only**, which is the usual
shape for a browser that only initiates calls. Choose **Inbound + outbound** to
enable **Discover + advertise** and accept calls. Manual Peer ID and route
fields remain a clearly labeled fallback. See
[DDS discovery](../../../docs/p2p/discovery.md) for the trust boundary.

Start with
[Build with an existing protocol](../../../docs/p2p/getting-started.md).
Protocol authors can continue with the
[one-crate authoring workflow](../../../docs/p2p/authoring-protocols.md).

## Prerequisites

- Rust 1.89.0 or newer
- Node.js 20.19 or newer (or 22.12 or newer)
- `wasm-pack` 0.13.1
- the `wasm32-unknown-unknown` Rust target

Install the pinned Wasm builder and target once if needed:

```sh
cargo install wasm-pack --version 0.13.1 --locked
rustup target add wasm32-unknown-unknown
```

Install the Web dependencies from the SDK repository root:

```sh
cd examples/portable-echo/web
npm ci
```

## Run the Web app

```sh
npm run dev
```

Open the printed root URL in two tabs:

1. Log in to both tabs with the same or different User credentials.
2. In tab A, select **Inbound + outbound** and **Discover + advertise**.
3. In tab B, keep **Outbound only** and **Discover only**.
4. Start both in the same Domain, then refresh tab B's Echo peers and select A.
5. Use the selected peer, enter a message, and send it.

An outbound-only peer is intentionally not a return target. To exercise both
browser directions, start both tabs as **Inbound + outbound** plus **Discover +
advertise**. Relay-backed mode prints the relay slot's TCP route so a native or
Python example can reach the browser. Use **Stop peer** to close any echo
endpoint before shutting down the peer and releasing any relay booking.

The app is deliberately plain: [`index.html`](index.html) is the form and
[`src/main.ts`](src/main.ts) is the complete host. Build standalone assets with:

```sh
npm run build
```

## Run the protected direction proof

The smoke harness under `scripts/` drives the same root app; there is no hidden
alternate UI. It starts two relay-backed browser endpoints, one outbound-only
browser client, and one native peer, then proves:

- browser A to browser B;
- browser B to browser A;
- outbound-only browser C to browser A;
- native to browser A;
- browser A to the same native peer.

It polls DDS for exact Echo advertisements and never gives one runtime another
runtime's route. It checks exact payloads, authenticated Peer IDs, TCP/WSS
relay reachability, and ordered shutdown. It also intercepts browser C's traffic
and asserts that startup, the Echo call, and shutdown make zero
`/relay-bookings` requests. Install Chromium once if needed:

```sh
npx playwright install chromium
```

Then provide a dev User and one of its accessible Domains through the
environment:

```sh
export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
npm run smoke:dev
```

The runner removes secret-like variables before invoking build tools and
Chromium, and passes the credentials only to the in-memory Wasm login and native
peer process. Developers with system Chrome can set
`AUKI_PLAYWRIGHT_CHANNEL=chrome`. The proof is intentionally not part of
credential-free CI.
