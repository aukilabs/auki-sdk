use auki_identity::{CreationCert, VerifyError, Wallet, load_or_mint_seed, verify};
use tempfile::TempDir;

#[test]
fn rust_root_api_remains_source_compatible() {
    let wallet = Wallet::from_seed(&[3u8; 32]);
    let child = wallet.derive_child("peer/v1");
    let signature = child.sign(b"auki");

    verify(&child.public_key(), b"auki", &signature).unwrap();
    assert_eq!(
        verify(&wallet.public_key(), b"auki", &signature),
        Err(VerifyError::SignatureMismatch)
    );
    assert_eq!(wallet.seed(), [3u8; 32]);
    assert_eq!(wallet.id().0.len(), 32);
}

#[test]
fn rust_root_still_exposes_creation_cert_and_native_seed_helper() {
    let parent = Wallet::from_seed(&[4u8; 32]);
    let child = Wallet::from_seed(&[5u8; 32]);
    let cert: CreationCert = parent.issue_creation_cert(&child, "app:test", 42);
    cert.verify().unwrap();

    let dir = TempDir::new().unwrap();
    let seed_path = dir.path().join("identity.seed");
    let first = load_or_mint_seed(&seed_path).unwrap();
    let second = load_or_mint_seed(&seed_path).unwrap();
    assert_eq!(first, second);
}
