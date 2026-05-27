# Auki P2P Preview Browser

Browser peer demo for subscribing to the RFC-first Sentinel preview stream.

Run the native Sentinel example first:

```sh
cargo run -p auki-p2p-preview-sentinel -- --bootstrap-json /tmp/auki-preview-bootstrap.json
```

Start the browser app:

```sh
cd examples/p2p-preview-browser
npm run dev
```

Open the Vite URL, load or paste the Sentinel bootstrap JSON, connect, and
subscribe.

This example imports `crates/auki-p2p-browser` from source so the demo follows
the SDK package API directly. It assumes `crates/auki-protocol-wasm/pkg-web`
exists; rebuild it from `crates/auki-p2p-browser` with
`npm run build:protocol-wasm` after protocol binding changes.
