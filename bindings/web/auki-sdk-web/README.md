# auki-sdk-web

Mechanical Rust/Wasm facade for one authenticated browser Peer with mandatory
DMS-backed WSS relay reachability.

The Rust facade owns authority renewal, relay booking, browser libp2p runtime,
and ordered shutdown. The TypeScript binding and persistent IndexedDB identity
adapter are added after this lifecycle is proven.

App access keys and secrets are deliberately unsupported in the browser. Use
User authentication or short-lived authority issued by a trusted backend.
