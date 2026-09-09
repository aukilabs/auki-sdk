# Portable Echo in Rust

Follow [Connect two peers](../../../../docs/tutorials/first-peer.md) to run this app.

The complete app is [src/main.rs](src/main.rs). It authenticates, selects a
Domain, starts a persistent peer, mounts Echo, discovers or calls another peer,
and shuts down in ownership order.
