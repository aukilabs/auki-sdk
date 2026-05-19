# auki-network-browser-wasm/src — sprint

## Now

Prove the first wasm package boundary:

- build the crate for `wasm32-unknown-unknown`
- export a small wasm-bindgen function
- add canonical seed-to-PeerId derivation
- add a JS import smoke test
- resolve the `browser_libp2p` `getrandom` 0.3 wasm JS RNG cfg blocker
- rerun the rust-libp2p browser transport feature compile probe

## Next

If `browser_libp2p` compiles, build an SDK-owned native probe listener and open one named protocol stream from the browser wasm peer.
