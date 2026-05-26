use crate::core;

uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn hash_jcs_bytes(bytes: Vec<u8>) -> String {
    core::hash_jcs_bytes(&bytes)
}
