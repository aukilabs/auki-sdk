//! Run one task as a provisioned robot. No hardware is controlled.
//! Live operation: registers, reads task data and reports a result to DMS.
use async_trait::async_trait;
use auki_sdk::{
    AukiDmsTasks, AukiPeerConfig, AukiRobotCredential, AukiTaskPeerConfig, Identity, RobotConfig,
    SecretString, TaskContext, TaskError, TaskHandler, TaskPeerContext, TaskResult, TasksConfig,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const CAPABILITY: &str = "/example/inspect-input/v1";
struct Inspect;
#[async_trait]
impl TaskHandler for Inspect {
    async fn run(&self, task: TaskContext) -> Result<TaskResult, TaskError> {
        let id = task.task.meta["input_id"]
            .as_str()
            .ok_or(TaskError::Handler)?
            .parse()
            .map_err(|_| TaskError::Handler)?;
        task.progress(serde_json::json!({"phase": "reading"}))?;
        let bytes = task.data().read(id).await.map_err(|_| TaskError::Handler)?;
        let peer_id = task.peer().map(|peer| peer.peer_id().to_string());
        Ok(TaskResult {
            output_cids: vec![],
            meta: serde_json::json!({"bytes": bytes.len(), "peer_id": peer_id}),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dds = std::env::var("DDS_BASE_URL")?;
    let dms = std::env::var("DMS_BASE_URL")?;
    let audience = match std::env::var("DDS_ROBOT_AUDIENCE") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let capabilities = vec![CAPABILITY.into()];
    let mut config = RobotConfig::new(
        &dds,
        &dms,
        SecretString::new(std::env::var("ROBOT_REGISTRATION_CREDENTIAL")?),
        "1.0.0",
        &std::env::var("AUKI_CLIENT_ID")?,
        audience.as_deref(),
        capabilities.clone(),
    )?;
    let peer = std::env::var_os("AUKI_PEER_IDENTITY_FILE")
        .map(|file| {
            AukiTaskPeerConfig::new(
                Identity::load_or_create(file)
                    .map_err(|_| TaskError::Configuration("cannot load peer identity"))?,
                &dds,
                AukiPeerConfig::new(&dms)
                    .map_err(|_| TaskError::Configuration("invalid peer configuration"))?,
            )
        })
        .transpose()?;
    config.peer_identity = peer.as_ref().map(AukiTaskPeerConfig::identity_proof);
    let robot = AukiRobotCredential::new(config)?;
    let tasks = match peer {
        Some(peer) => AukiDmsTasks::new_with_peer(
            robot.clone(),
            capabilities,
            TasksConfig::default(),
            Arc::new(peer),
        )?,
        None => AukiDmsTasks::new(robot.clone(), capabilities, TasksConfig::default())?,
    };
    let cancellation = CancellationToken::new();
    let outcome = async {
        tasks.start(&cancellation).await?;
        if let Some(peer) = tasks.peer() {
            println!("Robot peer ready: {}", peer.peer_id());
        }
        // The same peer stays connected after this task, until tasks.close().
        tasks.run_once(CAPABILITY, &Inspect, &cancellation).await
    }
    .await;
    let cleanup = tasks.close().await;
    robot.close().await;
    cleanup?;
    println!("{:?}", outcome?);
    Ok(())
}
