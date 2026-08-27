use std::{fmt, sync::Arc};

use async_trait::async_trait;
use auki_p2p::{DdsVerificationKeys, PeerId, PeerIdentityProof, SignedP2pCredential};
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{Error, Result, SecretString};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalKind {
    User,
    App,
}

impl PrincipalKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::App => "app",
        }
    }
}

/// Email/password credentials consumed by [`crate::AuthClient::authenticate`].
pub struct UserPassword {
    pub(crate) email: String,
    pub(crate) password: SecretString,
}

impl UserPassword {
    pub fn new(email: impl Into<String>, password: impl Into<SecretString>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }
}

impl fmt::Debug for UserPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserPassword")
            .field("email", &"[redacted]")
            .field("password", &"[redacted]")
            .finish()
    }
}

/// Trusted native/headless app credentials retained for bounded re-exchange.
pub struct AppCredentials {
    pub(crate) access_key: String,
    pub(crate) secret: SecretString,
    pub(crate) gateway_mac: Option<String>,
}

impl AppCredentials {
    pub fn new(access_key: impl Into<String>, secret: impl Into<SecretString>) -> Self {
        Self {
            access_key: access_key.into(),
            secret: secret.into(),
            gateway_mac: None,
        }
    }

    /// Bind DDS requests to one gateway MAC where app policy requires it.
    pub fn with_gateway_mac(mut self, gateway_mac: impl AsRef<str>) -> Result<Self> {
        self.gateway_mac = Some(normalize_gateway_mac(gateway_mac.as_ref())?);
        Ok(self)
    }
}

impl fmt::Debug for AppCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppCredentials")
            .field("access_key", &"[redacted]")
            .field("secret", &"[redacted]")
            .field("gateway_mac", &self.gateway_mac)
            .finish()
    }
}

pub enum Credentials {
    UserPassword(UserPassword),
    AppCredentials(AppCredentials),
}

impl Credentials {
    pub fn user_password(email: impl Into<String>, password: impl Into<SecretString>) -> Self {
        Self::UserPassword(UserPassword::new(email, password))
    }

    pub fn app(access_key: impl Into<String>, secret: impl Into<SecretString>) -> Self {
        Self::AppCredentials(AppCredentials::new(access_key, secret))
    }

    pub const fn principal_kind(&self) -> PrincipalKind {
        match self {
            Self::UserPassword(_) => PrincipalKind::User,
            Self::AppCredentials(_) => PrincipalKind::App,
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("principal_kind", &self.principal_kind())
            .field("material", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainDescriptor {
    pub id: Uuid,
    pub name: Option<String>,
    pub description: Option<String>,
    pub organization_id: Option<Uuid>,
}

impl DomainDescriptor {
    /// Descriptor for assignment-bound machine flows which only know the
    /// authoritative Domain ID and must not invent user-facing metadata.
    pub const fn assigned(id: Uuid) -> Self {
        Self {
            id,
            name: None,
            description: None,
            organization_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainChoice {
    pub domain: DomainDescriptor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainSelection {
    pub domain_id: Uuid,
}

impl DomainSelection {
    pub const fn new(domain_id: Uuid) -> Self {
        Self { domain_id }
    }
}

impl From<Uuid> for DomainSelection {
    fn from(domain_id: Uuid) -> Self {
        Self::new(domain_id)
    }
}

/// Complete initial authority required before a Domain is allowed to start.
#[derive(Clone, Debug)]
pub struct PreparedPeer {
    pub domain: DomainDescriptor,
    pub peer_id: PeerId,
    pub initial_credential: SignedP2pCredential,
    pub verification_keys: DdsVerificationKeys,
    pub credential_expires_at: DateTime<Utc>,
    pub renew_at: DateTime<Utc>,
    pub renewal: AuthorityRenewal,
}

/// One immutable, internally consistent authority replacement.
#[derive(Clone, Debug)]
pub struct RenewedAuthority {
    pub domain: DomainDescriptor,
    pub peer_id: PeerId,
    pub credential: SignedP2pCredential,
    pub verification_keys: DdsVerificationKeys,
    pub credential_expires_at: DateTime<Utc>,
    pub renew_at: DateTime<Utc>,
}

#[async_trait]
pub trait PeerAuthorityProvider: Send + Sync {
    async fn accessible_domains(&self) -> Result<Vec<DomainChoice>>;

    async fn authorize_peer(
        &self,
        selection: DomainSelection,
        identity: &PeerIdentityProof,
    ) -> Result<PreparedPeer>;
}

/// Pluggable explicit renewal operation. It never spawns or schedules work.
#[async_trait]
pub trait AuthorityRenewalProvider: Send + Sync {
    async fn renew_authority(&self, cancellation: &CancellationToken) -> Result<RenewedAuthority>;
}

#[derive(Clone)]
pub struct AuthorityRenewal {
    provider: Arc<dyn AuthorityRenewalProvider>,
}

impl AuthorityRenewal {
    pub fn new(provider: impl AuthorityRenewalProvider + 'static) -> Self {
        Self {
            provider: Arc::new(provider),
        }
    }

    pub fn from_shared(provider: Arc<dyn AuthorityRenewalProvider>) -> Self {
        Self { provider }
    }

    pub async fn renew(&self) -> Result<RenewedAuthority> {
        let cancellation = CancellationToken::new();
        self.provider.renew_authority(&cancellation).await
    }

    pub async fn renew_with_cancellation(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<RenewedAuthority> {
        self.provider.renew_authority(cancellation).await
    }
}

impl fmt::Debug for AuthorityRenewal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorityRenewal")
            .field("provider", &"[opaque]")
            .finish()
    }
}

fn normalize_gateway_mac(value: &str) -> Result<String> {
    if value.len() != 17 {
        return Err(Error::InvalidInput {
            field: "gateway_mac",
            reason: "must use six colon-separated hexadecimal octets",
        });
    }
    for (index, byte) in value.bytes().enumerate() {
        let valid = if index % 3 == 2 {
            byte == b':'
        } else {
            byte.is_ascii_hexdigit()
        };
        if !valid {
            return Err(Error::InvalidInput {
                field: "gateway_mac",
                reason: "must use six colon-separated hexadecimal octets",
            });
        }
    }
    Ok(value.to_ascii_uppercase())
}
