# auki-network-browser-wasm

Browser/WASM networking probe for the Auki SDK.

This crate tests whether the SDK can run a rust-libp2p peer in the browser while preserving canonical Auki peer identity and SDK-owned protocol streams. It is lower-level than `auki-domain-browser`: this crate proves browser networking primitives, while `auki-domain-browser` remains Park's Domain-level adapter.

The current implementation exposes identity/import probes, browser WebRTC probing, native Domain join, browser relay-address advertisement, and browser-to-browser native info/sensors catalog exchange. Browser Manager behavior and direct audio streams are later work.
