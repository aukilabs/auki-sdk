# auki-network-browser-wasm

Browser/WASM networking probe for the Auki SDK.

This crate tests whether the SDK can run a rust-libp2p peer in the browser while preserving canonical Auki peer identity and SDK-owned protocol streams. It is lower-level than `auki-domain-browser`: this crate proves browser networking primitives, while `auki-domain-browser` remains Park's Domain-level adapter.

The first implementation slice exposes identity/import probes and then compiles rust-libp2p browser transport features. Domain join, browser Manager behavior, and audio are later work.
