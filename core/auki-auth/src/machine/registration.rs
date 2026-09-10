//! One-shot Node registration. Retry and readiness policy belongs to the host.
pub mod crypto;

use anyhow::anyhow;
use crypto::{derive_eth_address, sign_eip191_recoverable_hex};
use reqwest::{Client, StatusCode};
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

#[derive(Debug, Serialize)]
pub struct NodeRegisterWalletRequest {
    pub message: String,
    pub signature: String,
    pub registration_credentials: String,
    pub capabilities: Vec<String>,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct SiweRequestMeta {
    pub nonce: Option<String>,
    pub domain: Option<String>,
    pub uri: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "chainId")]
    pub chain_id: Option<i64>,
    #[serde(rename = "issuedAt")]
    pub issued_at: Option<String>,
}

fn registration_endpoint(dds_base_url: &str) -> String {
    let base = dds_base_url.trim_end_matches('/');
    format!("{}/internal/v1/nodes/register-wallet", base)
}

fn siwe_request_endpoint(dds_base_url: &str) -> String {
    let base = dds_base_url.trim_end_matches('/');
    format!("{}/internal/v1/auth/siwe/request", base)
}

async fn request_siwe_meta(
    dds_base_url: &str,
    wallet: &str,
    client: &Client,
) -> std::result::Result<SiweRequestMeta, RegistrationAttempt> {
    let endpoint = siwe_request_endpoint(dds_base_url);
    let res = client
        .post(&endpoint)
        .json(&serde_json::json!({ "wallet": wallet }))
        .send()
        .await
        .map_err(|err| {
            RegistrationAttempt::retryable_failure(format!(
                "request SIWE nonce failed: endpoint {}, error: {}",
                endpoint, err
            ))
        })?;
    let status = res.status();
    if !status.is_success() {
        let body_snippet = response_body_snippet(res).await;
        return Err(classify_http_status(
            status,
            format!(
                "request SIWE nonce failed: status {}, endpoint {}, body_snippet: {}",
                status, endpoint, body_snippet
            ),
        ));
    }
    let body: SiweRequestMeta = res.json().await.map_err(|err| {
        RegistrationAttempt::retryable_failure(format!(
            "decode SIWE nonce response failed: endpoint {}, error: {}",
            endpoint, err
        ))
    })?;
    if body.nonce.as_deref().unwrap_or("").is_empty() {
        return Err(RegistrationAttempt::retryable_failure(
            "siwe nonce missing in response".to_string(),
        ));
    }
    Ok(body)
}

fn compose_message(meta: &SiweRequestMeta, address: &str) -> anyhow::Result<String> {
    let domain = meta
        .domain
        .as_deref()
        .ok_or_else(|| anyhow!("siwe domain missing"))?;
    let uri = meta
        .uri
        .as_deref()
        .ok_or_else(|| anyhow!("siwe uri missing"))?;
    let version = meta
        .version
        .as_deref()
        .ok_or_else(|| anyhow!("siwe version missing"))?;
    let chain_id = meta
        .chain_id
        .ok_or_else(|| anyhow!("siwe chain id missing"))?;
    let nonce = meta
        .nonce
        .as_deref()
        .ok_or_else(|| anyhow!("siwe nonce missing"))?;
    let issued_at = meta
        .issued_at
        .as_deref()
        .ok_or_else(|| anyhow!("siwe issued_at missing"))?;

    let mut out = String::new();
    out.push_str(&format!(
        "{} wants you to sign in with your Ethereum account:\n",
        domain
    ));
    out.push_str(address);
    out.push_str("\n\n");
    out.push_str(&format!("URI: {}\n", uri));
    out.push_str(&format!("Version: {}\n", version));
    out.push_str(&format!("Chain ID: {}\n", chain_id));
    out.push_str(&format!("Nonce: {}\n", nonce));
    out.push_str(&format!("Issued At: {}", issued_at));
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Existing registration response classification used by the host retry loop.
pub enum RegistrationAttemptKind {
    Registered,
    Conflict,
    RetryableFailure,
    SlowRetryFailure,
}

#[derive(Debug, Clone)]
pub struct RegistrationAttempt {
    kind: RegistrationAttemptKind,
    error: Option<String>,
}

impl RegistrationAttempt {
    pub fn kind(&self) -> RegistrationAttemptKind {
        self.kind
    }

    fn registered() -> Self {
        Self {
            kind: RegistrationAttemptKind::Registered,
            error: None,
        }
    }

    fn conflict(error: String) -> Self {
        Self {
            kind: RegistrationAttemptKind::Conflict,
            error: Some(error),
        }
    }

    fn retryable_failure(error: String) -> Self {
        Self {
            kind: RegistrationAttemptKind::RetryableFailure,
            error: Some(error),
        }
    }

    fn slow_retry_failure(error: String) -> Self {
        Self {
            kind: RegistrationAttemptKind::SlowRetryFailure,
            error: Some(error),
        }
    }

    pub fn error_text(&self) -> &str {
        self.error.as_deref().unwrap_or("")
    }
}

fn classify_http_status(status: StatusCode, error: String) -> RegistrationAttempt {
    match status {
        StatusCode::CONFLICT => RegistrationAttempt::conflict(error),
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => RegistrationAttempt::retryable_failure(error),
        s if s.is_server_error() => RegistrationAttempt::retryable_failure(error),
        _ => RegistrationAttempt::slow_retry_failure(error),
    }
}

async fn response_body_snippet(res: reqwest::Response) -> String {
    match res.text().await {
        Ok(mut text) => {
            if text.len() > 512 {
                text.truncate(512);
            }
            text.replace('\n', " ")
        }
        Err(_) => "<unavailable>".to_string(),
    }
}

pub async fn register_once(
    dds_base_url: &str,
    node_version: &str,
    reg_secret: &str,
    sk: &SecretKey,
    client: &Client,
    capabilities: &[String],
) -> RegistrationAttempt {
    if capabilities.is_empty() {
        return RegistrationAttempt::slow_retry_failure(
            "capabilities must be non-empty for DDS registration".to_string(),
        );
    }
    let wallet = derive_eth_address(sk);
    let wallet_prefix = wallet.get(0..10).unwrap_or(&wallet);
    info!(
        wallet_prefix = wallet_prefix,
        version = node_version,
        capabilities = ?capabilities,
        "Registering node with DDS (SIWE)"
    );

    let meta = match request_siwe_meta(dds_base_url, &wallet, client).await {
        Ok(meta) => meta,
        Err(attempt) => return attempt,
    };
    let message = match compose_message(&meta, &wallet) {
        Ok(message) => message,
        Err(err) => {
            return RegistrationAttempt::retryable_failure(format!(
                "compose SIWE message failed: {}",
                err
            ));
        }
    };
    let signature = sign_eip191_recoverable_hex(sk, &message);
    let req = NodeRegisterWalletRequest {
        message,
        signature,
        registration_credentials: reg_secret.to_owned(),
        capabilities: capabilities.to_vec(),
        version: node_version.to_owned(),
    };
    let endpoint = registration_endpoint(dds_base_url);

    let res = client
        .post(&endpoint)
        .json(&req)
        .send()
        .await
        .map_err(|err| {
            RegistrationAttempt::retryable_failure(format!(
                "registration request failed: endpoint {}, error: {}",
                endpoint, err
            ))
        });

    let res = match res {
        Ok(res) => res,
        Err(attempt) => return attempt,
    };

    if res.status().is_success() {
        debug!(status = ?res.status(), "Registration ok");
        RegistrationAttempt::registered()
    } else {
        let status = res.status();
        let body_snippet = response_body_snippet(res).await;
        classify_http_status(
            status,
            format!(
                "registration failed: status {}, endpoint {}, body_snippet: {}",
                status, endpoint, body_snippet
            ),
        )
    }
}
