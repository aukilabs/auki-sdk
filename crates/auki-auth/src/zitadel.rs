use std::{fmt, net::IpAddr};

use async_trait::async_trait;
use auki_p2p::P2P_TOKEN_MAX_BYTES;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::Url;
use serde::Deserialize;

use crate::{AuthLimits, Error, Result, SecretString, client, zitadel_http};

pub(crate) const DISCOVERY: &str = "ZITADEL discovery";
pub(crate) const TOKEN: &str = "ZITADEL token endpoint";

/// A public-client session handed over after the host's PKCE login.
///
/// `issuer` and `client_id` must come from trusted application configuration,
/// not unverified token claims. The host must stop competing refresh after
/// handing this session to the SDK. Tokens may be opaque; expiry may be unknown.
pub struct ZitadelSessionCredentials {
    pub(crate) access_token: SecretString,
    pub(crate) refresh_token: SecretString,
    pub(crate) client_id: String,
    pub(crate) issuer: Url,
    pub(crate) access_token_expires_at: Option<DateTime<Utc>>,
}

impl ZitadelSessionCredentials {
    pub fn new(
        access_token: impl Into<SecretString>,
        refresh_token: impl Into<SecretString>,
        client_id: impl Into<String>,
        issuer: Url,
        access_token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Self> {
        let credentials = Self {
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            client_id: client_id.into(),
            issuer,
            access_token_expires_at,
        };
        validate_url(&credentials.issuer)?;
        validate_client_id(&credentials.client_id)?;
        validate_token(&credentials.access_token, "access_token")?;
        validate_token(&credentials.refresh_token, "refresh_token")?;
        Ok(credentials)
    }

    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }
    pub fn refresh_token(&self) -> &SecretString {
        &self.refresh_token
    }
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
    pub fn issuer(&self) -> &Url {
        &self.issuer
    }
    pub fn access_token_expires_at(&self) -> Option<DateTime<Utc>> {
        self.access_token_expires_at
    }
}

impl fmt::Debug for ZitadelSessionCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZitadelSessionCredentials")
            .field("credentials", &"[redacted]")
            .field("access_token_expires_at", &self.access_token_expires_at)
            .finish()
    }
}

/// Acknowledge durable, atomic storage of the complete replacement session.
///
/// Resolve successfully only after secure persistence has finished. Return
/// [`Error::Persistence`] on failure; do not include host error text or secrets.
/// On either result, all writes started by this invocation must have settled:
/// no detached writes or callback reentry into the same SDK session.
/// Close the SDK session and await its outstanding save before clearing storage.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait ZitadelSessionStore: Send + Sync {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<()>;
}

/// Safe OAuth classification; provider-controlled error descriptions are discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZitadelOAuthError {
    InvalidGrant,
    InvalidClient,
    UnauthorizedClient,
    InvalidRequest,
    InvalidScope,
    TemporarilyUnavailable,
    ServerError,
    Other,
}

/// Low-level bounded public-client refresh primitive.
///
/// This performs no persistence, login UI or scheduling. A session coordinator
/// must retain/save its result before another refresh or downstream exchange.
/// Never run it concurrently with the SDK session's refresh owner. Once polled,
/// a refresh must be driven to completion: dropping it can lose a rotated token.
#[derive(Clone)]
pub struct ZitadelTokenClient {
    issuer: Url,
    client_id: String,
    limits: AuthLimits,
    #[cfg(not(target_arch = "wasm32"))]
    http: reqwest::Client,
}

impl fmt::Debug for ZitadelTokenClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZitadelTokenClient")
            .field("configuration", &"[redacted]")
            .finish()
    }
}

impl ZitadelTokenClient {
    pub fn new(issuer: Url, client_id: impl Into<String>, limits: AuthLimits) -> Result<Self> {
        let client_id = client_id.into();
        validate_url(&issuer)?;
        validate_client_id(&client_id)?;
        client::validate_limits(limits)?;
        Ok(Self {
            issuer,
            client_id,
            limits,
            #[cfg(not(target_arch = "wasm32"))]
            http: client::build_http_client(limits)?,
        })
    }

    pub async fn refresh(
        &self,
        credentials: &ZitadelSessionCredentials,
    ) -> Result<ZitadelSessionCredentials> {
        if credentials.issuer != self.issuer || credentials.client_id != self.client_id {
            return Err(Error::InvalidConfiguration(
                "ZITADEL session issuer/client do not match expected configuration",
            ));
        }
        let mut discovery_url = self.issuer.clone();
        discovery_url.set_path(&format!(
            "{}/.well-known/openid-configuration",
            self.issuer.path().trim_end_matches('/')
        ));
        let response = self.request(&discovery_url, None, DISCOVERY).await?;
        if response.status != 200 {
            return Err(Error::HttpStatus {
                endpoint: DISCOVERY,
                status: response.status,
            });
        }
        let discovery: Discovery = serde_json::from_slice(&response.body)
            .map_err(|_| Error::invalid_response(DISCOVERY, "invalid discovery document"))?;
        let advertised_issuer = Url::parse(&discovery.issuer)
            .map_err(|_| Error::InvalidConfiguration("invalid discovered issuer"))?;
        if advertised_issuer != self.issuer {
            return Err(Error::InvalidConfiguration(
                "discovered issuer does not match expected ZITADEL issuer",
            ));
        }
        let endpoint = Url::parse(&discovery.token_endpoint)
            .map_err(|_| Error::InvalidConfiguration("invalid discovered token endpoint"))?;
        validate_url(&endpoint)?;
        // ZITADEL publishes its token endpoint on the issuer's origin. Do not
        // turn discovery into permission to send a refresh token elsewhere.
        if endpoint.origin() != self.issuer.origin() {
            return Err(Error::InvalidConfiguration(
                "ZITADEL token endpoint must share the expected issuer origin",
            ));
        }
        if discovery
            .token_endpoint_auth_methods_supported
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|m| m == "none"))
        {
            return Err(Error::InvalidConfiguration(
                "ZITADEL public-client authentication none is unsupported",
            ));
        }
        if discovery
            .grant_types_supported
            .as_ref()
            .is_some_and(|grants| !grants.iter().any(|g| g == "refresh_token"))
        {
            return Err(Error::InvalidConfiguration(
                "ZITADEL refresh grant is unsupported",
            ));
        }
        let form = [
            ("grant_type", "refresh_token"),
            ("client_id", self.client_id.as_str()),
            ("refresh_token", credentials.refresh_token.expose()),
        ];
        // No access token, client secret, scope, redirect URI or PKCE verifier.
        let issued_at = Utc::now();
        let response = self
            .request(&endpoint, Some(&form), TOKEN)
            .await
            .map_err(|_| Error::RefreshOutcomeUnknown)?;
        if response.status != 200 {
            let error: OAuthFailure =
                serde_json::from_slice(&response.body).map_err(|_| Error::RefreshOutcomeUnknown)?;
            return Err(Error::ZitadelOAuth(match error.error.as_str() {
                "invalid_grant" => ZitadelOAuthError::InvalidGrant,
                "invalid_client" => ZitadelOAuthError::InvalidClient,
                "unauthorized_client" => ZitadelOAuthError::UnauthorizedClient,
                "invalid_request" => ZitadelOAuthError::InvalidRequest,
                "invalid_scope" => ZitadelOAuthError::InvalidScope,
                "temporarily_unavailable" => ZitadelOAuthError::TemporarilyUnavailable,
                "server_error" => ZitadelOAuthError::ServerError,
                _ => ZitadelOAuthError::Other,
            }));
        }
        let token: TokenResponse =
            serde_json::from_slice(&response.body).map_err(|_| Error::RefreshOutcomeUnknown)?;
        if !token.token_type.eq_ignore_ascii_case("Bearer") || token.expires_in == 0 {
            return Err(Error::RefreshOutcomeUnknown);
        }
        let lifetime = i64::try_from(token.expires_in)
            .ok()
            .and_then(ChronoDuration::try_seconds)
            .ok_or(Error::RefreshOutcomeUnknown)?;
        // Start from request submission, not arrival, so network delay never
        // extends the provider's access-token lifetime.
        let expiry = issued_at
            .checked_add_signed(lifetime)
            .ok_or(Error::RefreshOutcomeUnknown)?;
        ZitadelSessionCredentials::new(
            token.access_token,
            token
                .refresh_token
                .unwrap_or_else(|| credentials.refresh_token.expose().to_owned()),
            self.client_id.clone(),
            self.issuer.clone(),
            Some(expiry),
        )
        .map_err(|_| Error::RefreshOutcomeUnknown)
    }

    async fn request(
        &self,
        url: &Url,
        form: Option<&[(&str, &str)]>,
        endpoint: &'static str,
    ) -> Result<zitadel_http::Response> {
        zitadel_http::request(
            #[cfg(not(target_arch = "wasm32"))]
            &self.http,
            url,
            form,
            self.limits,
            endpoint,
        )
        .await
    }
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    token_endpoint: String,
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    grant_types_supported: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default, deserialize_with = "optional_refresh_token")]
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
}

#[derive(Deserialize)]
struct OAuthFailure {
    error: String,
}

fn optional_refresh_token<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

fn validate_token(token: &SecretString, field: &'static str) -> Result<()> {
    if token.is_empty()
        || token.len() > P2P_TOKEN_MAX_BYTES
        || token.expose().trim() != token.expose()
    {
        return Err(Error::InvalidInput {
            field,
            reason: "must contain 1–65536 bytes without surrounding whitespace",
        });
    }
    Ok(())
}

fn validate_client_id(client_id: &str) -> Result<()> {
    if client_id.is_empty() || client_id.len() > 255 || client_id.trim() != client_id {
        return Err(Error::InvalidInput {
            field: "client_id",
            reason: "must contain 1–255 bytes without surrounding whitespace",
        });
    }
    Ok(())
}

fn validate_url(url: &Url) -> Result<()> {
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.host_str().is_none()
        || (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidConfiguration(
            "ZITADEL URLs require HTTPS (or explicit loopback HTTP), without userinfo, query or fragment",
        ));
    }
    Ok(())
}
