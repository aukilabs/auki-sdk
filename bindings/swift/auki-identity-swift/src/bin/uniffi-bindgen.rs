//! Host-side Swift codegen entry point. Build with `--features cli`,
//! then `cargo run --features cli --bin uniffi-bindgen -- generate
//! --library <staticlib> --language swift --out-dir <dir>`. Not part of
//! the shipped library. Same pattern as
//! `bindings/swift/auki-network-swift/src/bin/uniffi-bindgen.rs`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
