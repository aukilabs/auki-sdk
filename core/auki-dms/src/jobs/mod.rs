//! Peer-free Domain jobs. This is an HTTP client, not a task scheduler or worker.
//! All operations share the caller's User/App or imported-session refresh owner.

mod error;
mod http;
mod types;

pub use error::JobsError;
pub use types::*;

use std::{
    future::Future,
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use auki_auth::{AuthSession, DomainAccess, DomainAccessProvider};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct JobsLimits {
    /// Total deadline, including authentication, a possible 401 renewal and body.
    pub request_timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

impl Default for JobsLimits {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            max_request_bytes: 1024 * 1024,
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Shares the existing authenticated session without starting background work.
/// The host explicitly supplies the DMS API base, including its `/v1` prefix.
#[derive(Clone)]
pub struct AukiDmsJobs {
    session: AuthSession,
    base: Url,
    http: Client,
    limits: JobsLimits,
}

impl AukiDmsJobs {
    pub fn new(session: AuthSession, dms_base_url: &str) -> Result<Self, JobsError> {
        Self::with_limits(session, dms_base_url, JobsLimits::default())
    }

    pub fn with_limits(
        session: AuthSession,
        dms_base_url: &str,
        limits: JobsLimits,
    ) -> Result<Self, JobsError> {
        let base = base_url(dms_base_url)?;
        if limits.request_timeout.is_zero()
            || limits.request_timeout > Duration::from_secs(300)
            || !(1..=16 * 1024 * 1024).contains(&limits.max_request_bytes)
            || !(1..=16 * 1024 * 1024).contains(&limits.max_response_bytes)
        {
            return Err(JobsError::InvalidInput("invalid timeout or byte limits"));
        }
        let builder = Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5));
        let http = builder
            .build()
            .map_err(|_| JobsError::InvalidInput("cannot construct HTTP client"))?;
        Ok(Self {
            session,
            base,
            http,
            limits,
        })
    }

    /// Select a Domain without authentication or network work.
    pub fn in_domain(&self, domain_id: Uuid) -> DomainJobsClient {
        DomainJobsClient {
            client: self.clone(),
            domain_id,
            lifetime: Arc::new(Lifetime {
                closed: CancellationToken::new(),
                active: RwLock::new(()),
            }),
        }
    }
}

struct Lifetime {
    closed: CancellationToken,
    active: RwLock<()>,
}

/// Clones share close state. Separately selected clients keep their own lifetime;
/// closing this client never logs out its shared session or cancels backend jobs.
#[derive(Clone)]
pub struct DomainJobsClient {
    client: AukiDmsJobs,
    domain_id: Uuid,
    lifetime: Arc<Lifetime>,
}

impl DomainJobsClient {
    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// The provider's broad activity feed: public tasks and this User grant's
    /// organization-scoped dedicated tasks. Entries have no Domain association.
    /// None means the grant is not a supported User profile; do not interpret
    /// this as an empty feed or expose entries as selected-Domain jobs.
    pub async fn busy_nodes_with_cancellation(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<BusyNodeSnapshot>, JobsError> {
        let mut url = self.client.base.clone();
        url.path_segments_mut()
            .expect("validated base URL")
            .pop_if_empty()
            .push("nodes")
            .push("busy");
        url.query_pairs_mut().append_pair("mode", "all");
        self.run(cancellation, async {
            let Some((bytes, grant)) = self
                .request_with_grant(Method::GET, url, None, cancellation, None, true)
                .await?
            else {
                return Ok(None);
            };
            #[derive(Deserialize)]
            struct Response {
                nodes: Vec<BusyNode>,
            }
            let response: Response = decode(&bytes)?;
            let mut ids = std::collections::HashSet::new();
            if response.nodes.iter().any(|node| {
                node.node_id.is_nil()
                    || node.task_id.is_nil()
                    || node.job_id.is_nil()
                    || !ids.insert(node.node_id)
                    || !matches!(
                        node.task_status,
                        JobTaskStatus::Leased | JobTaskStatus::Running
                    )
            }) {
                return Err(JobsError::InvalidResponse("inconsistent busy-node feed"));
            }
            Ok(Some(BusyNodeSnapshot {
                organization_id: grant.user_organization().expect("checked User profile"),
                nodes: response.nodes,
            }))
        })
        .await
    }

    /// Cancel and drain in-flight HTTP work. This is distinct from canceling a job.
    pub async fn close(&self) {
        self.lifetime.closed.cancel();
        let _drained = self.lifetime.active.write().await;
    }

    pub async fn estimate(&self, spec: &JobSpec) -> Result<JobEstimate, JobsError> {
        self.estimate_with_cancellation(spec, &CancellationToken::new())
            .await
    }

    pub async fn estimate_with_cancellation(
        &self,
        spec: &JobSpec,
        cancellation: &CancellationToken,
    ) -> Result<JobEstimate, JobsError> {
        let body = self.body(spec)?;
        self.run(cancellation, async {
            let bytes = self
                .request(
                    Method::POST,
                    self.url(None, Some("estimate")),
                    Some(&body),
                    cancellation,
                    None,
                )
                .await?;
            let estimate: JobEstimate = decode(&bytes)?;
            if estimate.tasks.len() != spec.tasks.len()
                || estimate
                    .tasks
                    .iter()
                    .zip(&spec.tasks)
                    .any(|(actual, requested)| {
                        actual.label != requested.label
                            || actual.stage != requested.stage
                            || actual.capability != requested.capability
                            || actual.mode != requested.mode
                    })
            {
                return Err(JobsError::InvalidResponse(
                    "estimate does not match requested tasks",
                ));
            }
            Ok(estimate)
        })
        .await
    }

    /// Submit once. A rejected bearer may renew once; ambiguous submissions are
    /// never retried. An uncertain outcome requires host reconciliation.
    pub async fn submit(&self, spec: &JobSpec) -> Result<Uuid, JobsError> {
        self.submit_with_cancellation(spec, &CancellationToken::new())
            .await
    }

    pub async fn submit_with_cancellation(
        &self,
        spec: &JobSpec,
        cancellation: &CancellationToken,
    ) -> Result<Uuid, JobsError> {
        let body = self.body(spec)?;
        let sent = AtomicBool::new(false);
        self.run(cancellation, async {
            let bytes = self
                .request(
                    Method::POST,
                    self.url(None, None),
                    Some(&body),
                    cancellation,
                    Some(&sent),
                )
                .await?;
            #[derive(Deserialize)]
            struct Submitted {
                job_id: Uuid,
            }
            let response: Submitted = decode(&bytes)?;
            if response.job_id.is_nil() {
                return Err(JobsError::InvalidResponse("missing created job ID"));
            }
            Ok(response.job_id)
        })
        .await
        .map_err(|error| {
            if sent.load(Ordering::Relaxed) && error.ambiguous_after_send() {
                JobsError::SubmissionUncertain {
                    source: Box::new(error),
                }
            } else {
                error
            }
        })
    }

    pub async fn list(&self, query: &JobListQuery) -> Result<JobPage, JobsError> {
        self.list_with_cancellation(query, &CancellationToken::new())
            .await
    }

    pub async fn list_with_cancellation(
        &self,
        query: &JobListQuery,
        cancellation: &CancellationToken,
    ) -> Result<JobPage, JobsError> {
        if !(1..=100).contains(&query.limit) {
            return Err(JobsError::InvalidInput("page limit must be 1-100"));
        }
        let mut url = self.url(None, None);
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("domain_id", &self.domain_id.to_string());
            pairs.append_pair("limit", &query.limit.to_string());
            if let Some(cursor) = &query.cursor {
                pairs.append_pair("cursor", cursor);
            }
            if let Some(status) = query.status {
                pairs.append_pair("status", status.as_str());
            }
            for capability in &query.capabilities {
                pairs.append_pair("capabilities", capability);
            }
            pairs.append_pair(
                "match_all_capabilities",
                if query.match_all_capabilities {
                    "true"
                } else {
                    "false"
                },
            );
        }
        if url.as_str().len() > self.client.limits.max_request_bytes {
            return Err(JobsError::TooLarge {
                maximum: self.client.limits.max_request_bytes,
            });
        }
        self.run(cancellation, async {
            let bytes = self
                .request(Method::GET, url, None, cancellation, None)
                .await?;
            let page: JobPage = decode(&bytes)?;
            let mut ids = std::collections::HashSet::new();
            if page.items.len() > query.limit as usize
                || page.items.iter().any(|item| {
                    item.job.domain_id != self.domain_id
                        || item.job.id.is_nil()
                        || !ids.insert(item.job.id)
                })
            {
                return Err(JobsError::InvalidResponse("inconsistent job page"));
            }
            Ok(page)
        })
        .await
    }

    pub async fn get(&self, id: Uuid) -> Result<JobDetails, JobsError> {
        self.get_with_cancellation(id, &CancellationToken::new())
            .await
    }

    pub async fn get_with_cancellation(
        &self,
        id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<JobDetails, JobsError> {
        self.run(cancellation, async {
            let bytes = self
                .request(
                    Method::GET,
                    self.url(Some(id), None),
                    None,
                    cancellation,
                    None,
                )
                .await?;
            let details: JobDetails = decode(&bytes)?;
            if details.job.id != id
                || details.job.domain_id != self.domain_id
                || details.tasks.iter().any(|task| task.job_id != id)
                || details.receipts.iter().any(|receipt| receipt.job_id != id)
            {
                return Err(JobsError::InvalidResponse(
                    "response belongs to another job or Domain",
                ));
            }
            Ok(details)
        })
        .await
    }

    /// Request whole-job cancellation. A canceled job can still contain running
    /// tasks awaiting heartbeat or lease expiry; no physical-stop guarantee.
    pub async fn cancel(&self, id: Uuid) -> Result<JobCancellation, JobsError> {
        self.cancel_with_cancellation(id, &CancellationToken::new())
            .await
    }

    pub async fn cancel_with_cancellation(
        &self,
        id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<JobCancellation, JobsError> {
        self.run(cancellation, async {
            let bytes = self
                .request(
                    Method::POST,
                    self.url(Some(id), Some("cancel")),
                    None,
                    cancellation,
                    None,
                )
                .await?;
            let result: JobCancellation = decode(&bytes)?;
            if result.id != id {
                return Err(JobsError::InvalidResponse(
                    "cancellation belongs to another job",
                ));
            }
            Ok(result)
        })
        .await
    }

    fn body(&self, spec: &JobSpec) -> Result<Vec<u8>, JobsError> {
        if spec.tasks.is_empty() {
            return Err(JobsError::InvalidInput("a job requires at least one task"));
        }
        #[derive(Serialize)]
        struct SelectedSpec<'a> {
            domain_id: Uuid,
            #[serde(flatten)]
            spec: &'a JobSpec,
        }
        let bytes = serde_json::to_vec(&SelectedSpec {
            domain_id: self.domain_id,
            spec,
        })
        .map_err(|_| JobsError::InvalidInput("invalid job specification"))?;
        if bytes.len() > self.client.limits.max_request_bytes {
            return Err(JobsError::TooLarge {
                maximum: self.client.limits.max_request_bytes,
            });
        }
        Ok(bytes)
    }

    fn url(&self, id: Option<Uuid>, action: Option<&str>) -> Url {
        let mut url = self.client.base.clone();
        {
            let mut path = url.path_segments_mut().expect("validated base URL");
            path.pop_if_empty().push("jobs");
            if let Some(id) = id {
                path.push(&id.to_string());
            }
            if let Some(action) = action {
                path.push(action);
            }
        }
        url
    }

    async fn request(
        &self,
        method: Method,
        url: Url,
        body: Option<&[u8]>,
        cancellation: &CancellationToken,
        sent: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, JobsError> {
        Ok(self
            .request_with_grant(method, url, body, cancellation, sent, false)
            .await?
            .expect("unrestricted profile request")
            .0)
    }

    async fn request_with_grant(
        &self,
        method: Method,
        url: Url,
        body: Option<&[u8]>,
        cancellation: &CancellationToken,
        sent: Option<&AtomicBool>,
        user_only: bool,
    ) -> Result<Option<(Vec<u8>, Arc<DomainAccess>)>, JobsError> {
        let mut access = self
            .client
            .session
            .domain_access(self.domain_id, None, cancellation)
            .await?;
        for attempt in 0..2 {
            if access.domain_id() != self.domain_id
                || !access.has_dds_audience()
                || access.expires_at() <= chrono::Utc::now()
            {
                return Err(JobsError::InvalidResponse(
                    "wrong-Domain, expired, or non-DDS grant",
                ));
            }
            if user_only && access.user_organization().is_none() {
                return Ok(None);
            }
            let mut request = self
                .client
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(access.bearer().expose_secret())
                .header("posemesh-client-id", self.client.session.client_id())
                .header(
                    "posemesh-sdk-version",
                    concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                )
                .header("Accept", "application/json");
            if let Some(body) = body {
                request = request
                    .header("Content-Type", "application/json")
                    .body(body.to_vec());
            }
            if let Some(sent) = sent {
                sent.store(true, Ordering::Relaxed);
            }
            match http::send(request, self.client.limits.max_response_bytes).await {
                Err(JobsError::HttpStatus { status: 401 }) if attempt == 0 => {
                    // A 401 explicitly rejects creation, so a failed renewal is
                    // not an ambiguous submission and must keep its auth code.
                    if let Some(sent) = sent {
                        sent.store(false, Ordering::Relaxed);
                    }
                    access = self
                        .client
                        .session
                        .domain_access(self.domain_id, Some(&access), cancellation)
                        .await?;
                }
                result => return result.map(|bytes| Some((bytes, access))),
            }
        }
        unreachable!("second attempt always returns")
    }

    async fn run<T>(
        &self,
        cancellation: &CancellationToken,
        operation: impl Future<Output = Result<T, JobsError>>,
    ) -> Result<T, JobsError> {
        let _active = self.lifetime.active.read().await;
        tokio::select! {
            biased;
            _ = self.lifetime.closed.cancelled() => Err(JobsError::Closed),
            _ = self.client.session.wait_closed() => Err(JobsError::Auth(auki_auth::Error::SessionClosed)),
            _ = cancellation.cancelled() => Err(JobsError::Cancelled),
            _ = futures_timer::Delay::new(self.client.limits.request_timeout) => Err(JobsError::TimedOut),
            result = operation => result,
        }
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, JobsError> {
    serde_json::from_slice(bytes)
        .map_err(|_| JobsError::InvalidResponse("unexpected JSON contract"))
}

fn base_url(value: &str) -> Result<Url, JobsError> {
    let invalid = || {
        JobsError::InvalidInput(
            "DMS base requires HTTPS or literal loopback HTTP, without user info, query or fragment",
        )
    };
    let url = Url::parse(value).map_err(|_| invalid())?;
    let authority = value
        .split_once("//")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or_default();
    let loopback = url.scheme() == "http"
        && url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if url.cannot_be_a_base()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || loopback)
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}
