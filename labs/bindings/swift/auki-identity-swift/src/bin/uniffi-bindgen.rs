//! Host-side Swift codegen entry point. Build with `--features cli`,
//! then `cargo run --features cli --bin uniffi-bindgen -- generate
//! --library <staticlib> --language swift --out-dir <dir>`. Not part of
//! the shipped library.

fn main() {
    uniffi::uniffi_bindgen_main()
}
