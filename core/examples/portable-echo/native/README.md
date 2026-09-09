# Portable Echo in Rust

Follow [Connect two peers](../../../../docs/tutorials/first-peer.md) to run this app.

The complete app is [src/main.rs](src/main.rs). It signs in, starts a peer,
registers Echo, sends or answers requests, and shuts down. It saves its Peer
ID in a file so you can reuse it after restarting.
