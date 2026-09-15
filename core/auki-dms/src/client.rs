use crate::types::{
    CompleteTaskRequest, FailTaskRequest, HeartbeatRequest, HeartbeatResponse, LeaseResponse,
};
use anyhow::{Context, Result, anyhow};
use auki_auth::machine::token_manager::TokenProvider;
use reqwest::Client;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use std::sync::Arc;
use std::time::Duration;
use tracing::Level;
use url::Url;
use uuid::Uuid;

/// A successful claim can also report no work or an already-active lease.
pub enum ClaimOutcome {
    Leased(Box<LeaseResponse>),
    NoWork,
    Busy,
}

/// HTTP status without response bodies or credentials. Existing anyhow callers
/// can downcast to this type; SDK bindings preserve its status.
#[derive(Debug)]
pub struct DmsHttpError {
    pub operation: &'static str,
    pub status: u16,
}
impl std::fmt::Display for DmsHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DMS {} returned HTTP {}", self.operation, self.status)
    }
}
impl std::error::Error for DmsHttpError {}

/// Minimal DMS HTTP client using rustls with sensitive Authorization header.
#[derive(Clone)]
pub struct DmsClient {
    base: Url,
    http: Client,
    auth: Arc<dyn TokenProvider>,
}
impl DmsClient {
    /// Create client with base URL, timeout, and a token provider for Authorization.
    pub fn new(base: Url, timeout: Duration, auth: Arc<dyn TokenProvider>) -> Result<Self> {
        let http = Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .context("build dms reqwest client")?;
        Ok(Self { base, http, auth })
    }

    async fn auth_headers(&self) -> Result<HeaderMap> {
        let mut h = HeaderMap::new();
        let b = self
            .auth
            .bearer()
            .await
            .map_err(|e| anyhow!("token provider: {e}"))?;
        let token = format!("Bearer {}", b);
        let mut v = HeaderValue::from_str(&token)
            .unwrap_or_else(|_| HeaderValue::from_static("Bearer INVALID"));
        v.set_sensitive(true);
        h.insert(AUTHORIZATION, v);
        Ok(h)
    }

    fn join_segments(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow!("invalid DMS base URL; cannot be a base"))?
            .extend(segments.iter().copied());
        Ok(url)
    }

    /// Lease a task: GET /tasks
    ///
    /// Busy retains the legacy `None` behavior. Use `claim` to distinguish it.
    pub async fn lease_by_capability(&self, capability: &str) -> Result<Option<LeaseResponse>> {
        match self.claim(capability).await? {
            ClaimOutcome::Leased(lease) => Ok(Some(*lease)),
            ClaimOutcome::NoWork | ClaimOutcome::Busy => Ok(None),
        }
    }

    /// Let DMS choose across all capabilities permitted by machine authentication.
    pub async fn lease_any(&self) -> Result<Option<LeaseResponse>> {
        self.lease_by_capability("").await
    }

    pub async fn claim(&self, capability: &str) -> Result<ClaimOutcome> {
        let mut url = self.join_segments(&["tasks"]).context("join /tasks")?;
        if !capability.is_empty() {
            url.query_pairs_mut().append_pair("capability", capability);
        }
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                endpoint = %url,
                "Sending DMS lease request"
            );
        }
        // First attempt
        let mut headers = self.auth_headers().await?;
        let mut res = self
            .http
            .get(url.clone())
            .headers(headers.clone())
            .send()
            .await
            .context("send GET /tasks")?;
        let mut status = res.status();
        let mut bytes = response_bytes(res).await?;
        // Retry once on 401
        if status == StatusCode::UNAUTHORIZED {
            tracing::warn!(
                status = %status,
                "DMS lease unauthorized; refreshing token and retrying"
            );
            self.auth.on_unauthorized().await;
            headers = self.auth_headers().await?;
            res = self
                .http
                .get(url)
                .headers(headers)
                .send()
                .await
                .context("retry GET /tasks")?;
            status = res.status();
            bytes = response_bytes(res).await?;
        }
        if status == StatusCode::NO_CONTENT {
            tracing::debug!("DMS lease returned 204 (no work available)");
            return Ok(ClaimOutcome::NoWork);
        }
        if status == StatusCode::CONFLICT {
            tracing::debug!(
                status = %status,
                "DMS lease returned conflict (busy)"
            );
            return Ok(ClaimOutcome::Busy);
        }
        if !status.is_success() {
            tracing::warn!(
                status = %status,
                "DMS lease request returned non-success status"
            );
            return Err(DmsHttpError {
                operation: "claim",
                status: status.as_u16(),
            }
            .into());
        }
        let lease: LeaseResponse = serde_json::from_slice(&bytes)
            .map_err(|_| {
                tracing::error!(
                    status = %status,
                    "Failed to decode DMS lease response"
                );
                anyhow!("invalid DMS response")
            })
            .context("decode lease")?;

        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                status = %status,
                task_id = %lease.task.id,
                capability = %lease.task.capability,
                access_token_updated = lease.access_token.is_some(),
                p2p_access_token_updated = lease.p2p_access_token.is_some(),
                "Decoded DMS lease response"
            );
        }

        Ok(ClaimOutcome::Leased(Box::new(lease)))
    }

    /// Complete task: POST /tasks/{id}/complete
    pub async fn complete(&self, task_id: Uuid, body: &CompleteTaskRequest) -> Result<()> {
        let url = self
            .join_segments(&["tasks", &task_id.to_string(), "complete"])
            .context("join /complete")?;
        let mut headers = self.auth_headers().await?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                endpoint = %url,
                task_id = %task_id,
                "Sending DMS complete request"
            );
        }
        // First attempt
        let mut res = self
            .http
            .post(url.clone())
            .headers(headers.clone())
            .json(body)
            .send()
            .await
            .context("send POST /complete")?;
        let mut status = res.status();
        let _ = response_bytes(res).await;
        if status == StatusCode::UNAUTHORIZED {
            tracing::warn!(
                status = %status,
                task_id = %task_id,
                "DMS complete unauthorized; refreshing token and retrying"
            );
            self.auth.on_unauthorized().await;
            headers = self.auth_headers().await?;
            res = self
                .http
                .post(url)
                .headers(headers)
                .json(body)
                .send()
                .await
                .context("retry POST /complete")?;
            status = res.status();
            let _ = response_bytes(res).await;
        }
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                status = %status,
                task_id = %task_id,
                "DMS complete response"
            );
        }
        if !status.is_success() {
            tracing::error!(
                status = %status,
                task_id = %task_id,
                "DMS complete endpoint returned non-success status"
            );
            return Err(DmsHttpError {
                operation: "complete",
                status: status.as_u16(),
            }
            .into());
        }
        Ok(())
    }

    /// Fail task: POST /tasks/{id}/fail
    pub async fn fail(&self, task_id: Uuid, body: &FailTaskRequest) -> Result<()> {
        let url = self
            .join_segments(&["tasks", &task_id.to_string(), "fail"])
            .context("join /fail")?;
        let mut headers = self.auth_headers().await?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                endpoint = %url,
                task_id = %task_id,
                "Sending DMS fail request"
            );
        }
        // First attempt
        let mut res = self
            .http
            .post(url.clone())
            .headers(headers.clone())
            .json(body)
            .send()
            .await
            .context("send POST /fail")?;
        let mut status = res.status();
        let _ = response_bytes(res).await;
        if status == StatusCode::UNAUTHORIZED {
            tracing::warn!(
                status = %status,
                task_id = %task_id,
                "DMS fail unauthorized; refreshing token and retrying"
            );
            self.auth.on_unauthorized().await;
            headers = self.auth_headers().await?;
            res = self
                .http
                .post(url)
                .headers(headers)
                .json(body)
                .send()
                .await
                .context("retry POST /fail")?;
            status = res.status();
            let _ = response_bytes(res).await;
        }
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                status = %status,
                task_id = %task_id,
                "DMS fail response"
            );
        }
        if !status.is_success() {
            tracing::error!(
                status = %status,
                task_id = %task_id,
                "DMS fail endpoint returned non-success status"
            );
            return Err(DmsHttpError {
                operation: "fail",
                status: status.as_u16(),
            }
            .into());
        }
        Ok(())
    }

    /// Heartbeat: POST /tasks/{id}/heartbeat with progress payload.
    /// Returns potential new access token for storage.
    pub async fn heartbeat(
        &self,
        task_id: Uuid,
        body: &HeartbeatRequest,
    ) -> Result<HeartbeatResponse> {
        let url = self
            .join_segments(&["tasks", &task_id.to_string(), "heartbeat"])
            .context("join /heartbeat")?;
        let mut headers = self.auth_headers().await?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                endpoint = %url,
                task_id = %task_id,
                "Sending DMS heartbeat request"
            );
        }
        // First attempt
        let mut res = self
            .http
            .post(url.clone())
            .headers(headers.clone())
            .json(body)
            .send()
            .await
            .context("send POST /heartbeat")?;
        let mut status = res.status();
        let mut bytes = response_bytes(res).await?;
        if status == StatusCode::UNAUTHORIZED {
            tracing::warn!(
                status = %status,
                task_id = %task_id,
                "DMS heartbeat unauthorized; refreshing token and retrying"
            );
            self.auth.on_unauthorized().await;
            headers = self.auth_headers().await?;
            res = self
                .http
                .post(url)
                .headers(headers)
                .json(body)
                .send()
                .await
                .context("retry POST /heartbeat")?;
            status = res.status();
            bytes = response_bytes(res).await?;
        }
        if !status.is_success() {
            tracing::warn!(
                status = %status,
                task_id = %task_id,
                "DMS heartbeat endpoint returned non-success status"
            );
            return Err(DmsHttpError {
                operation: "heartbeat",
                status: status.as_u16(),
            }
            .into());
        }
        let hb = serde_json::from_slice::<HeartbeatResponse>(&bytes)
            .map_err(|_| {
                tracing::error!(
                    status = %status,
                    task_id = %task_id,
                    "Failed to decode DMS heartbeat response"
                );
                anyhow!("invalid DMS response")
            })
            .context("decode heartbeat response")?;
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!(
                status = %status,
                task_id = %task_id,
                access_token_updated = hb.access_token.is_some(),
                p2p_access_token_updated = hb.p2p_access_token.is_some(),
                cancel = ?hb.cancel,
                "Decoded DMS heartbeat response"
            );
        }
        Ok(hb)
    }
}

// Limit decoded control-plane envelopes even when the peer omits Content-Length.
async fn response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
    const MAX_RESPONSE: usize = 2 * 1024 * 1024;
    if !response.status().is_success() {
        return Ok(Vec::new());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err(anyhow!("DMS response exceeds 2 MiB"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("read DMS response failed"))?
    {
        if chunk.len() > MAX_RESPONSE.saturating_sub(bytes.len()) {
            return Err(anyhow!("DMS response exceeds 2 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
