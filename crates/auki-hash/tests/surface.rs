use auki_hash::hash_jcs_bytes;

#[test]
fn rust_root_api_remains_source_compatible() {
    assert_eq!(hash_jcs_bytes(b"").len(), 32);
    assert_eq!(hash_jcs_bytes(b"abc"), "06b05ab6733a618578af5f94892f3950");
}
