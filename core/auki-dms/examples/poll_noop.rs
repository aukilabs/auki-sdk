//! Native, finite DMS loop for a dedicated /example/noop/v1 task queue.
use std::{env, sync::Arc, time::Duration};

use async_trait::async_trait;
use auki_auth::machine::token_manager::{TokenProvider, TokenProviderResult};
use auki_dms::{
    client::DmsClient,
    poller::{PollerConfig, jittered_delay_ms},
    types::{CompleteTaskRequest, FailTaskRequest},
};

const CAPABILITY: &str = "/example/noop/v1";

// A caller-supplied bearer keeps this example independent of the login flow.
// Long-lived hosts can pass the shared machine TokenManager instead.
struct ExistingBearer(String);

#[async_trait]
impl TokenProvider for ExistingBearer {
    async fn bearer(&self) -> TokenProviderResult<String> {
        Ok(self.0.clone())
    }

    async fn on_unauthorized(&self) {}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = DmsClient::new(
        env::var("DMS_BASE_URL")?.parse()?,
        Duration::from_secs(10),
        Arc::new(ExistingBearer(env::var("DMS_MACHINE_TOKEN")?)),
    )?;
    let polls: usize = env::var("DMS_POLL_COUNT")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    let backoff = PollerConfig {
        backoff_ms_min: 1000,
        backoff_ms_max: 1000,
    };

    // Construction made no requests. The host decides when and how often to poll.
    for tick in 0..polls {
        if let Some(lease) = client.lease_by_capability(CAPABILITY).await? {
            // The current service request does not send a capability filter.
            if lease.task.capability == CAPABILITY {
                // This capability intentionally does no work and emits no outputs.
                client
                    .complete(lease.task.id, &CompleteTaskRequest::default())
                    .await?;
            } else {
                client
                    .fail(
                        lease.task.id,
                        &FailTaskRequest {
                            reason: "unsupported capability in noop example".into(),
                            ..Default::default()
                        },
                    )
                    .await?;
            }
            println!("Reported task {}.", lease.task.id);
        }
        if tick + 1 < polls {
            tokio::time::sleep(Duration::from_millis(jittered_delay_ms(backoff))).await;
        }
    }
    Ok(())
}
