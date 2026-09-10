# Portable Echo in the browser

Requires Rust 1.89+, wasm-pack 0.13.1, and Node 20.19+ on 20.x or 22.12+.
Use the development User/Domain from the
[networking tutorial](../../../../docs/tutorials/first-peer.md).

From the SDK repository root:

~~~sh
cargo install wasm-pack --version 0.13.1 --locked
rustup target add wasm32-unknown-unknown
cd core/examples/portable-echo/web
npm ci
npm run dev
~~~

Open the printed local URL in two tabs:

1. Log in to both and select the same Domain.
2. In tab A, choose **Inbound + outbound** and **Discover + advertise**.
3. In tab B, keep **Outbound only** and **Discover only**.
4. Start both peers. Refresh tab B's Echo peers and select A.
5. Send a message; check that the echoed payload matches.
6. Use **Stop peer** in both tabs to close the connections and release the relays.

Every start creates a fresh Peer ID. Tab B can also call a running native or
Python Echo peer in the same Domain.

The [TypeScript app](src/main.ts) calls Rust Echo code compiled into the same
Wasm module as the SDK. See [custom protocols](../../../../docs/how-to/protocols.md).
