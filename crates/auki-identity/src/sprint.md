# Sprint — auki-identity

Current focus: keep the Rust wallet primitive stable while generating native
and web bindings from thin adapter modules.

## Now

- Rust API remains `auki_identity::{Wallet, PublicKey, WalletId, Signature, CreationCert, VerifyError, verify, load_or_mint_seed}`.
- Native Swift/Python bindings compile through UniFFI in `ffi.rs`.
- JavaScript/WebAssembly bindings compile through wasm-bindgen in `wasm.rs`.
- Seed persistence is exposed in every binding surface:
  - native bindings use filesystem paths;
  - wasm bindings use browser `localStorage` keys.

## Next

- Run the root generation recipes for this production crate:
  - `just generate-swift-bindings auki-identity`
  - `just generate-python-bindings auki-identity`
  - `just generate-javascript-bindings auki-identity`
- Add language-package smoke tests once generated artifacts are committed or staged for each binding target.
- Decide whether the generated binding package names should replace or coexist with the older PyO3 `auki-identity-py` package during migration.
