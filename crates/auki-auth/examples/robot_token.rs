//! Native Robot authentication without a peer, DMS loop or Posemesh runner.
use std::{env, sync::Arc, time::Duration};

use auki_auth::machine::{
    robot::RobotAuthenticator,
    token_manager::{SystemClock, TokenManager, TokenManagerConfig, TokenProvider},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let authenticator = RobotAuthenticator::new(
        env::var("DDS_BASE_URL")?.parse()?,
        env::var("ROBOT_REGISTRATION_CREDENTIALS")?,
        env!("CARGO_PKG_VERSION").into(),
        vec![env::var("AUKI_CAPABILITY")?],
        Duration::from_secs(10),
    )?;
    let manager = TokenManager::new(
        Arc::new(authenticator),
        Arc::new(SystemClock),
        TokenManagerConfig::default(),
    );

    // Background refresh is explicit. bearer() also works on demand without it.
    manager.start_bg().await;
    let result = manager.bearer().await;
    // The host can use the token or share the manager as Arc<dyn TokenProvider>.
    // Stop refresh on both the success and error paths. Never print the token.
    manager.stop_bg().await;
    let _token = result?;
    println!("Robot authentication succeeded.");
    Ok(())
}
