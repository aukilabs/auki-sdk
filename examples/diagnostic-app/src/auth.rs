use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use auki_domain::{DdsVerificationKeys, Identity, SignedP2pCredential};
use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_TTL, P2P_TOKEN_TYPE, P2PAccessClaims,
    SignedApplicationMetadata,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p256::{
    SecretKey,
    elliptic_curve::rand_core::OsRng,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use uuid::Uuid;

pub(crate) struct LoadedAuthority {
    pub(crate) identity: Identity,
    pub(crate) keys: DdsVerificationKeys,
    pub(crate) credential: SignedP2pCredential,
}

pub(crate) fn load_authority(
    identity_path: &Path,
    public_key_path: &Path,
    credential_path: &Path,
    key_generation: u64,
) -> Result<LoadedAuthority> {
    let identity_bytes = fs::read(identity_path)
        .with_context(|| format!("read identity {}", identity_path.display()))?;
    let identity = Identity::from_protobuf_encoding(&identity_bytes)
        .with_context(|| format!("decode identity {}", identity_path.display()))?;
    let public_key = fs::read(public_key_path)
        .with_context(|| format!("read DDS public key {}", public_key_path.display()))?;
    let compact = fs::read_to_string(credential_path)
        .with_context(|| format!("read credential {}", credential_path.display()))?;
    let credential = SignedP2pCredential::new(compact.trim().to_owned())
        .with_context(|| format!("decode credential {}", credential_path.display()))?;

    Ok(LoadedAuthority {
        identity,
        keys: DdsVerificationKeys::new(key_generation, public_key, None),
        credential,
    })
}

pub(crate) struct DemoMaterial {
    pub(crate) directory: PathBuf,
    pub(crate) domain_id: Uuid,
    pub(crate) wrong_domain_id: Uuid,
    pub(crate) peer_a: String,
    pub(crate) peer_b: String,
}

pub(crate) fn create_demo_material(directory: &Path, domain_id: Uuid) -> Result<DemoMaterial> {
    fs::create_dir_all(directory)
        .with_context(|| format!("create material directory {}", directory.display()))?;

    let identity_a = Identity::from_ed25519_seed(&[41; 32]);
    let identity_b = Identity::from_ed25519_seed(&[42; 32]);
    let peer_a = identity_a.peer_id();
    let peer_b = identity_b.peer_id();
    let wrong_domain_id = Uuid::from_u128(domain_id.as_u128() ^ 1);
    let dds_secret = SecretKey::random(&mut OsRng);
    let private_pem = dds_secret
        .to_pkcs8_pem(LineEnding::LF)
        .context("encode demo DDS private key")?;
    let public_pem = dds_secret
        .public_key()
        .to_public_key_pem(LineEnding::LF)
        .context("encode demo DDS public key")?;
    let encoding_key =
        EncodingKey::from_ec_pem(private_pem.as_bytes()).context("parse demo DDS signing key")?;

    write_new(
        &directory.join("peer-a.identity"),
        &identity_a.to_protobuf_encoding()?,
    )?;
    write_new(
        &directory.join("peer-b.identity"),
        &identity_b.to_protobuf_encoding()?,
    )?;
    write_new(&directory.join("dds-public.pem"), public_pem.as_bytes())?;
    write_new(
        &directory.join("peer-a.peer-id"),
        format!("{peer_a}\n").as_bytes(),
    )?;
    write_new(
        &directory.join("peer-b.peer-id"),
        format!("{peer_b}\n").as_bytes(),
    )?;

    let issued_at = unix_time()?;
    write_credential(
        &directory.join("peer-a.jwt"),
        sign_demo_credential(&encoding_key, peer_a.to_string(), domain_id, issued_at)?,
    )?;
    write_credential(
        &directory.join("peer-b.jwt"),
        sign_demo_credential(&encoding_key, peer_b.to_string(), domain_id, issued_at)?,
    )?;
    write_credential(
        &directory.join("peer-a-wrong-domain.jwt"),
        sign_demo_credential(
            &encoding_key,
            peer_a.to_string(),
            wrong_domain_id,
            issued_at,
        )?,
    )?;
    write_credential(
        &directory.join("peer-a-wrong-peer.jwt"),
        sign_demo_credential(&encoding_key, peer_b.to_string(), domain_id, issued_at)?,
    )?;

    Ok(DemoMaterial {
        directory: directory.to_path_buf(),
        domain_id,
        wrong_domain_id,
        peer_a: peer_a.to_string(),
        peer_b: peer_b.to_string(),
    })
}

fn sign_demo_credential(
    encoding_key: &EncodingKey,
    peer_id: String,
    domain_id: Uuid,
    issued_at: u64,
) -> Result<String> {
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: "00000000-0000-4000-8000-000000000001".into(),
        peer_type: None,
        peer_id,
        domain_ids: vec![domain_id.to_string()],
        scopes: Vec::new(),
        application: Some(SignedApplicationMetadata {
            name: "auki-diagnostic-app".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }),
        iat: issued_at,
        nbf: None,
        exp: issued_at + P2P_TOKEN_TTL.as_secs(),
    };
    encode(&Header::new(Algorithm::ES256), &claims, encoding_key).context("sign demo credential")
}

fn write_credential(path: &Path, compact: String) -> Result<()> {
    write_new(path, format!("{compact}\n").as_bytes())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("create {} without overwriting it", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    Ok(())
}

fn unix_time() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| anyhow::anyhow!("system clock precedes Unix epoch: {error}"))
}

pub(crate) fn require_empty_or_missing_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("inspect material directory {}", path.display()))?;
    if entries.next().transpose()?.is_some() {
        bail!(
            "material directory {} is not empty; choose a new directory",
            path.display()
        );
    }
    Ok(())
}
