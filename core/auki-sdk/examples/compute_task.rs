//! Claim at most one task for an already-provisioned compute node.
//! Live operation: registers, claims work and creates a Domain artifact.
use async_trait::async_trait;
use auki_sdk::{
    AukiComputeCredential, AukiDmsTasks, ComputeConfig, DataWrite, SecretString, TaskContext,
    TaskError, TaskHandler, TaskResult, TasksConfig,
};
use tokio_util::sync::CancellationToken;

struct Uppercase;
#[async_trait]
impl TaskHandler for Uppercase {
    async fn run(&self, task: TaskContext) -> Result<TaskResult, TaskError> {
        let id = task.task.meta["input_id"]
            .as_str()
            .ok_or(TaskError::Handler)?
            .parse()
            .map_err(|_| TaskError::Handler)?;
        let data = task.data();
        task.progress(serde_json::json!({"phase":"reading"}))?;
        let input = data.read(id).await.map_err(|_| TaskError::Handler)?;
        let output = String::from_utf8(input)
            .map_err(|_| TaskError::Handler)?
            .to_uppercase();
        task.progress(serde_json::json!({"phase":"writing"}))?;
        let name = format!("uppercase-{}", task.task.id);
        let stored = data
            .write(
                DataWrite::Named {
                    name: &name,
                    data_type: "example.text.v1",
                },
                output.as_bytes(),
            )
            .await
            .map_err(|_| TaskError::Handler)?;
        Ok(TaskResult {
            output_cids: vec![stored.id.to_string()],
            meta: serde_json::json!({"data_id":stored.id}),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ComputeConfig::new(
        &std::env::var("DDS_BASE_URL")?,
        &std::env::var("DMS_BASE_URL")?,
        SecretString::new(std::env::var("NODE_REGISTRATION_CREDENTIAL")?),
        SecretString::new(std::env::var("NODE_WALLET_KEY")?),
        "1.0.0",
        &std::env::var("AUKI_CLIENT_ID")?,
    )?;
    let credential = AukiComputeCredential::new(config)?;
    let tasks = AukiDmsTasks::new(
        credential.clone(),
        vec!["/example/uppercase/v1".into()],
        TasksConfig::default(),
    )?;
    let outcome = tasks
        .run_once(
            "/example/uppercase/v1",
            &Uppercase,
            &CancellationToken::new(),
        )
        .await;
    let cleanup = tasks.close().await;
    credential.close().await;
    cleanup?;
    println!("{:?}", outcome?);
    Ok(())
}
