//! Domain-scoped DMS job submission bindings using a shared User session.

use std::sync::Arc;

use auki_sdk_rs::{AukiDmsJobs, DomainJobsClient, JobsError};
use parking_lot::Mutex;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{AukiCancellation, AukiSdkError, AukiSession, wait_cleanup};

#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum AukiJobsFailureKind {
    Authentication,
    InvalidInput,
    InvalidResponse,
    HttpStatus,
    Transport,
    TimedOut,
    Cancelled,
    Closed,
    TooLarge,
    SubmissionUncertain,
}

fn jobs_error(error: JobsError) -> AukiSdkError {
    let kind = match &error {
        JobsError::Auth(_) => AukiJobsFailureKind::Authentication,
        JobsError::InvalidInput(_) => AukiJobsFailureKind::InvalidInput,
        JobsError::InvalidResponse(_) => AukiJobsFailureKind::InvalidResponse,
        JobsError::HttpStatus { .. } => AukiJobsFailureKind::HttpStatus,
        JobsError::Transport => AukiJobsFailureKind::Transport,
        JobsError::TimedOut => AukiJobsFailureKind::TimedOut,
        JobsError::Cancelled => AukiJobsFailureKind::Cancelled,
        JobsError::Closed => AukiJobsFailureKind::Closed,
        JobsError::TooLarge { .. } => AukiJobsFailureKind::TooLarge,
        JobsError::SubmissionUncertain { .. } => AukiJobsFailureKind::SubmissionUncertain,
    };
    let maximum = match &error {
        JobsError::TooLarge { maximum } => u64::try_from(*maximum).ok(),
        _ => None,
    };
    let source = match &error {
        JobsError::SubmissionUncertain { source } => Some(source.code().to_owned()),
        _ => None,
    };
    AukiSdkError::Jobs {
        kind,
        status: error.http_status(),
        code: error.code().to_owned(),
        maximum,
        source_code: source,
        message: error.to_string(),
    }
}

fn invalid_input(message: impl Into<String>) -> AukiSdkError {
    AukiSdkError::Jobs {
        kind: AukiJobsFailureKind::InvalidInput,
        status: None,
        code: "invalid_input".into(),
        maximum: None,
        source_code: None,
        message: message.into(),
    }
}

struct JobsOwner {
    inner: DomainJobsClient,
    closed: CancellationToken,
    completion: Mutex<Option<watch::Sender<Option<crate::CleanupResult>>>>,
    runtime: Mutex<Option<Handle>>,
}

impl JobsOwner {
    fn remember_runtime(&self) -> Option<Handle> {
        let mut retained = self.runtime.lock();
        if retained.is_none() {
            *retained = Handle::try_current().ok();
        }
        retained.clone()
    }

    fn begin_close(&self) -> Option<watch::Receiver<Option<crate::CleanupResult>>> {
        self.closed.cancel();
        let mut completion = self.completion.lock();
        if let Some(sender) = completion.as_ref() {
            return Some(sender.subscribe());
        }
        let runtime = self.remember_runtime()?;
        let (sender, receiver) = watch::channel(None);
        *completion = Some(sender.clone());
        let inner = self.inner.clone();
        runtime.spawn(async move {
            inner.close().await;
            sender.send_replace(Some(Ok(())));
        });
        Some(receiver)
    }

    fn operation_token(&self, cancellation: Option<Arc<AukiCancellation>>) -> CancellationToken {
        self.remember_runtime();
        cancellation
            .map(|value| value.token.child_token())
            .unwrap_or_default()
    }

    fn ensure_open(&self) -> Result<(), AukiSdkError> {
        if self.closed.is_cancelled() {
            Err(jobs_error(JobsError::Closed))
        } else {
            Ok(())
        }
    }
}

impl Drop for JobsOwner {
    fn drop(&mut self) {
        let _ = self.begin_close();
    }
}

#[derive(uniffi::Object)]
pub struct AukiDomainJobs {
    owner: JobsOwner,
}

impl AukiDomainJobs {
    fn new(inner: DomainJobsClient) -> Self {
        Self {
            owner: JobsOwner {
                inner,
                closed: CancellationToken::new(),
                completion: Mutex::new(None),
                runtime: Mutex::new(Handle::try_current().ok()),
            },
        }
    }

    fn decode<T: serde::de::DeserializeOwned>(json: &str, label: &str) -> Result<T, AukiSdkError> {
        serde_json::from_str(json).map_err(|_| invalid_input(format!("invalid {label} JSON")))
    }

    fn encode<T: serde::Serialize>(value: &T) -> Result<String, AukiSdkError> {
        serde_json::to_string(value)
            .map_err(|_| jobs_error(JobsError::InvalidResponse("response could not be encoded")))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiDomainJobs {
    pub fn domain_id(&self) -> String {
        self.owner.inner.domain_id().to_string()
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        let completion = self
            .owner
            .begin_close()
            .expect("UniFFI async jobs close runs on the retained Tokio runtime");
        wait_cleanup(completion)
            .await
            .map_err(|error| invalid_input(format!("close DMS jobs client: {error}")))
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn estimate_json(
        &self,
        spec_json: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        self.owner.ensure_open()?;
        let spec = Self::decode(&spec_json, "job specification")?;
        let token = self.owner.operation_token(cancellation);
        let value = self
            .owner
            .inner
            .estimate_with_cancellation(&spec, &token)
            .await
            .map_err(jobs_error)?;
        Self::encode(&value)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn submit_json(
        &self,
        spec_json: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        self.owner.ensure_open()?;
        let spec = Self::decode(&spec_json, "job specification")?;
        let token = self.owner.operation_token(cancellation);
        self.owner
            .inner
            .submit_with_cancellation(&spec, &token)
            .await
            .map(|id| id.to_string())
            .map_err(jobs_error)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn list_json(
        &self,
        query_json: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        self.owner.ensure_open()?;
        let query = Self::decode(&query_json, "job list query")?;
        let token = self.owner.operation_token(cancellation);
        let value = self
            .owner
            .inner
            .list_with_cancellation(&query, &token)
            .await
            .map_err(jobs_error)?;
        Self::encode(&value)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn get_json(
        &self,
        job_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        self.owner.ensure_open()?;
        let job_id =
            Uuid::parse_str(&job_id).map_err(|_| invalid_input("job ID must be a UUID"))?;
        let token = self.owner.operation_token(cancellation);
        let value = self
            .owner
            .inner
            .get_with_cancellation(job_id, &token)
            .await
            .map_err(jobs_error)?;
        Self::encode(&value)
    }

    #[uniffi::method(default(cancellation = None))]
    pub async fn cancel_json(
        &self,
        job_id: String,
        cancellation: Option<Arc<AukiCancellation>>,
    ) -> Result<String, AukiSdkError> {
        self.owner.ensure_open()?;
        let job_id =
            Uuid::parse_str(&job_id).map_err(|_| invalid_input("job ID must be a UUID"))?;
        let token = self.owner.operation_token(cancellation);
        let value = self
            .owner
            .inner
            .cancel_with_cancellation(job_id, &token)
            .await
            .map_err(jobs_error)?;
        Self::encode(&value)
    }
}

#[uniffi::export]
impl AukiSession {
    pub fn jobs(&self, domain_id: String) -> Result<Arc<AukiDomainJobs>, AukiSdkError> {
        let domain_id =
            Uuid::parse_str(&domain_id).map_err(|_| invalid_input("Domain ID must be a UUID"))?;
        let config = self
            .peer_config
            .clone()
            .ok_or_else(|| invalid_input("DMS is not configured for this session"))?;
        let jobs =
            AukiDmsJobs::new(self.session.clone(), config.dms_base_url()).map_err(jobs_error)?;
        Ok(Arc::new(AukiDomainJobs::new(jobs.in_domain(domain_id))))
    }
}

#[cfg(test)]
mod tests {
    use auki_sdk_rs::{JobListQuery, JobMode, JobSpec};

    use super::*;

    #[test]
    fn bridge_json_applies_defaults_and_preserves_custom_capability() {
        let spec: JobSpec = AukiDomainJobs::decode(
            r#"{
                "label":"custom",
                "tasks":[{
                    "label":"vendor",
                    "stage":"analyze",
                    "capability":"vendor.example/private-model/v7",
                    "mode":"dedicated"
                }]
            }"#,
            "job specification",
        )
        .unwrap();
        assert_eq!(spec.priority, 0);
        assert!(spec.edges.is_empty());
        assert_eq!(spec.tasks[0].capability, "vendor.example/private-model/v7");
        assert_eq!(spec.tasks[0].mode, JobMode::Dedicated);
        assert_eq!(spec.tasks[0].max_attempts, 3);

        let query: JobListQuery = AukiDomainJobs::decode("{}", "job list query").unwrap();
        assert_eq!(query.limit, 50);
        assert!(!query.match_all_capabilities);
    }

    #[test]
    fn bridge_errors_keep_http_and_ambiguous_source_codes() {
        let http = jobs_error(JobsError::HttpStatus { status: 403 });
        assert!(matches!(
            http,
            AukiSdkError::Jobs {
                kind: AukiJobsFailureKind::HttpStatus,
                status: Some(403),
                ref code,
                ..
            } if code == "http_status"
        ));

        let uncertain = jobs_error(JobsError::SubmissionUncertain {
            source: Box::new(JobsError::TimedOut),
        });
        assert!(matches!(
            uncertain,
            AukiSdkError::Jobs {
                kind: AukiJobsFailureKind::SubmissionUncertain,
                ref source_code,
                ..
            } if source_code.as_deref() == Some("timed_out")
        ));
    }
}
