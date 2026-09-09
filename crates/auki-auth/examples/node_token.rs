//! Native one-shot Node registration and SIWE login. The host owns readiness.
use std::{env, time::Duration};

use auki_auth::machine::{
    registration::{RegistrationAttemptKind, crypto, register_once},
    siwe,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dds = env::var("DDS_BASE_URL")?;
    let private_key = env::var("SECP256K1_PRIVHEX")?;
    let secret = env::var("REG_SECRET")?;
    let capabilities = vec![env::var("AUKI_CAPABILITY")?];
    let key = crypto::load_secp256k1_privhex(&private_key)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let attempt = register_once(
        &dds,
        env!("CARGO_PKG_VERSION"),
        &secret,
        &key,
        &http,
        &capabilities,
    )
    .await;
    match attempt.kind() {
        RegistrationAttemptKind::Registered | RegistrationAttemptKind::Conflict => {}
        kind => anyhow::bail!("registration did not succeed: {kind:?}"),
    }

    // This one-shot example attempts login immediately. A long-lived host owns
    // readiness callbacks, registration retries and re-arming after 403/404.
    let wallet = siwe::derive_eth_address(&private_key)?;
    let nonce = siwe::request_nonce(&dds, &wallet).await?;
    let message = siwe::compose_message(&nonce, &wallet, None)?;
    let signature = siwe::sign_message(&private_key, &message)?;
    let _access = siwe::verify(&dds, &wallet, &message, &signature).await?;
    println!("Node SIWE authentication succeeded.");
    Ok(())
}
