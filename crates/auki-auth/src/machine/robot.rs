use super::token_manager::AccessAuthenticator;
use super::{AccessBundle, SiweError};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

const REGISTER_PATH: &str = "/internal/v1/robots/register";
const VERIFY_PATH: &str = "/internal/v1/auth/robot/verify";

#[derive(Serialize)]
struct RegisterRequest<'a> {
    registration_credentials: &'a str,
    version: &'a str,
    capabilities: &'a [String],
}

#[derive(Serialize)]
struct VerifyRequest<'a> {
    registration_credentials: &'a str,
}

#[derive(Deserialize)]
struct AccessResponse {
    robot_id: Option<Uuid>,
    access_token: Option<String>,
    access_expires_at: Option<String>,
}

/// Existing DDS Robot registration and verification operations.
pub struct RobotAuthenticator {
    base_url: Arc<String>,
    registration_credentials: Arc<String>,
    node_version: Arc<String>,
    capabilities: Arc<Vec<String>>,
    client: Client,
    registered: AtomicBool,
}

impl RobotAuthenticator {
    pub fn new(
        base_url: Url,
        registration_credentials: String,
        node_version: String,
        capabilities: Vec<String>,
        request_timeout: Duration,
    ) -> Result<Self> {
        let registration_credentials = registration_credentials.trim().to_string();
        if registration_credentials.is_empty() {
            return Err(anyhow!("robot registration credentials must not be empty"));
        }
        let node_version = node_version.trim().to_string();
        if node_version.is_empty() {
            return Err(anyhow!("robot node version must not be empty"));
        }

        let client = Client::builder()
            .use_rustls_tls()
            .timeout(request_timeout)
            .build()
            .context("build DDS robot authentication client")?;

        Ok(Self {
            base_url: Arc::new(base_url.to_string()),
            registration_credentials: Arc::new(registration_credentials),
            node_version: Arc::new(node_version),
            capabilities: Arc::new(capabilities),
            client,
            registered: AtomicBool::new(false),
        })
    }

    async fn register(&self) -> std::result::Result<AccessBundle, SiweError> {
        let endpoint = self.endpoint(REGISTER_PATH);
        let response = self
            .client
            .post(endpoint)
            .json(&RegisterRequest {
                registration_credentials: self.registration_credentials.as_str(),
                version: self.node_version.as_str(),
                capabilities: self.capabilities.as_slice(),
            })
            .send()
            .await?;
        decode_access_response(response).await
    }

    async fn verify(&self) -> std::result::Result<AccessBundle, SiweError> {
        let endpoint = self.endpoint(VERIFY_PATH);
        let response = self
            .client
            .post(endpoint)
            .json(&VerifyRequest {
                registration_credentials: self.registration_credentials.as_str(),
            })
            .send()
            .await?;
        decode_access_response(response).await
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }
}

#[async_trait]
impl AccessAuthenticator for RobotAuthenticator {
    async fn login(&self) -> std::result::Result<AccessBundle, SiweError> {
        if self.registered.load(Ordering::Acquire) {
            return self.verify().await;
        }

        let bundle = self.register().await?;
        self.registered.store(true, Ordering::Release);
        Ok(bundle)
    }
}

async fn decode_access_response(
    response: Response,
) -> std::result::Result<AccessBundle, SiweError> {
    if !response.status().is_success() {
        return Err(SiweError::UpstreamStatus(response.status()));
    }

    let body: AccessResponse = response.json().await?;
    if body.robot_id.filter(|id| !id.is_nil()).is_none() {
        return Err(SiweError::MissingField("robot_id"));
    }
    let token = body
        .access_token
        .filter(|value| !value.is_empty())
        .ok_or(SiweError::MissingField("access_token"))?;
    let expires_at_raw = body
        .access_expires_at
        .ok_or(SiweError::MissingField("access_expires_at"))?;
    let expires_at = DateTime::parse_from_rfc3339(&expires_at_raw)?.with_timezone(&Utc);

    Ok(AccessBundle::new(token, expires_at))
}
