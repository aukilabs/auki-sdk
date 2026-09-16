//! DDS data access using the same serialized refresh owner as P2P.

use super::*;
use serde::{Deserialize, Serialize};

#[path = "portals.rs"]
mod portals;
pub use portals::{Portal, PortalDomain, PortalId};

const DDS_DOMAINS: &str = "DDS /api/v1/domains";
const DDS_DOMAIN_AUTH: &str = "DDS selected-Domain data auth";
const MAX_CACHED_DOMAINS: usize = 64;

/// An ordinary DDS metadata query. Visibility does not establish data permissions.
#[derive(Clone, Debug)]
pub struct DomainListQuery {
    /// `own`, an organization UUID, or `all` with an owned Domain Server.
    pub organization: String,
    pub domain_server_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

impl Default for DomainListQuery {
    fn default() -> Self {
        Self {
            organization: "own".into(),
            domain_server_id: None,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainSummary {
    pub id: Uuid,
    pub name: String,
    pub organization_id: Option<Uuid>,
}

/// One real DDS page; totals are advisory while the collection changes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainPage {
    pub domains: Vec<DomainSummary>,
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
}

/// Domain/server-bound bearer obtained from authenticated DDS, never a P2P token.
/// Debug output redacts the token. Servers remain responsible for signature and
/// permission verification; local claim checks protect routing and cache expiry.
#[derive(Debug)]
pub struct DomainAccess {
    domain_id: Uuid,
    server_url: Url,
    token: SecretString,
    expires_at: DateTime<Utc>,
    dds_audience: bool,
}

impl DomainAccess {
    /// Validate a DDS-issued data grant delivered by an authenticated authority
    /// such as a DMS lease. This checks routing and expiry, not the signature;
    /// Domain Servers still verify the bearer and enforce resource permissions.
    /// Never use this constructor to trust a token received from a remote peer.
    pub fn from_issued_grant(
        domain_id: Uuid,
        server_url: &str,
        token: SecretString,
        expires_at: DateTime<Utc>,
    ) -> Result<Self> {
        let mut access = AccessResponse {
            id: domain_id,
            domain_server: ServerResponse {
                url: server_url.into(),
            },
            access_token: token.expose().to_owned(),
        }
        .into_access(domain_id)?;
        if expires_at <= Utc::now() {
            return Err(Error::StaleAuthority);
        }
        access.expires_at = access.expires_at.min(expires_at);
        Ok(access)
    }

    pub fn domain_id(&self) -> Uuid {
        self.domain_id
    }
    pub fn server_url(&self) -> &Url {
        &self.server_url
    }
    pub fn bearer(&self) -> &SecretString {
        &self.token
    }
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Supplies renewable data access without starting a peer or a task heartbeat.
/// This boundary lets later machine/task adapters use the same data client.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait DomainAccessProvider: Send + Sync {
    fn client_id(&self) -> &str;
    /// Local restriction for credentials that can only supply read grants.
    /// Returning true prevents writes; false never bypasses server permissions.
    fn read_only(&self) -> bool {
        false
    }
    /// Completes when all work using this credential must stop.
    async fn wait_closed(&self);
    /// Renew after expiry, or after the server rejects `rejected` with HTTP 401.
    /// Concurrent callers rejecting the same token share its replacement.
    async fn domain_access(
        &self,
        domain_id: Uuid,
        rejected: Option<&DomainAccess>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<DomainAccess>>;
}

impl AuthSession {
    pub fn client_id(&self) -> &str {
        &self.inner.client.inner.environment.client_id
    }

    pub async fn list_domains(&self, query: &DomainListQuery) -> Result<DomainPage> {
        self.list_domains_with_cancellation(query, &CancellationToken::new())
            .await
    }

    pub async fn list_domains_with_cancellation(
        &self,
        query: &DomainListQuery,
        cancellation: &CancellationToken,
    ) -> Result<DomainPage> {
        if !(1..=100).contains(&query.limit)
            || !(matches!(query.organization.as_str(), "own" | "all")
                || Uuid::parse_str(&query.organization).is_ok())
            || (query.organization == "all" && query.domain_server_id.is_none())
        {
            return Err(Error::InvalidInput {
                field: "Domain query",
                reason: "use limit 1-100 and own, an organization UUID, or all with a Domain Server",
            });
        }
        let operation = async {
            let mut state = self.lock_state(cancellation, DDS_DOMAINS).await?;
            if let PrincipalState::Zitadel(session) = &state.principal {
                if query.organization != "own" || query.domain_server_id.is_some() {
                    return Err(Error::InvalidConfiguration(
                        "imported ZITADEL Domain listing supports only the permission-aware own-Domain query without a Domain Server filter",
                    ));
                }
                let session = session.clone();
                return self
                    .imported_domain_page(&session, query, cancellation)
                    .await;
            }
            self.require_domain_listing(&state)?;
            let mut url = self.inner.client.dds_url("api/v1/domains");
            url.query_pairs_mut()
                .append_pair("org", &query.organization)
                .append_pair("issue_token", "false")
                .append_pair("limit", &query.limit.to_string())
                .append_pair("offset", &query.offset.to_string());
            if let Some(server) = query.domain_server_id {
                url.query_pairs_mut()
                    .append_pair("domain_server_id", &server.to_string());
            }
            let page: DomainPage = self
                .data_dds_json(&mut state, url, false, DDS_DOMAINS, cancellation)
                .await?;
            if page.limit != query.limit
                || page.offset != query.offset
                || page.domains.len() > query.limit as usize
                || page
                    .domains
                    .iter()
                    .map(|d| d.id)
                    .collect::<HashSet<_>>()
                    .len()
                    != page.domains.len()
            {
                return Err(Error::invalid_response(
                    DDS_DOMAINS,
                    "inconsistent pagination",
                ));
            }
            Ok(page)
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            result = operation => result,
        }
    }

    async fn imported_domain_page(
        &self,
        session: &Arc<ZitadelSession>,
        query: &DomainListQuery,
        cancellation: &CancellationToken,
    ) -> Result<DomainPage> {
        let mut refresh_used = false;
        let mut grant = self
            .exchange_imported_listing_grant(session, &mut refresh_used, cancellation)
            .await?;
        for attempt in 0..2 {
            match self
                .fetch_imported_domain_page(&grant, query, cancellation)
                .await
            {
                Err(error) if attempt == 0 && error.is_unauthorized() => {
                    grant = self
                        .exchange_imported_listing_grant(session, &mut refresh_used, cancellation)
                        .await?;
                }
                Ok(page) => return Ok(page),
                Err(error) => return Err(error),
            }
        }
        unreachable!("second attempt always returns")
    }

    async fn fetch_imported_domain_page(
        &self,
        grant: &ImportedListingGrant,
        query: &DomainListQuery,
        cancellation: &CancellationToken,
    ) -> Result<DomainPage> {
        match grant.kind {
            ImportedListingGrantKind::User { organization } => {
                let mut url = self.inner.client.dds_url("api/v1/domains");
                url.query_pairs_mut()
                    .append_pair("org", "own")
                    .append_pair("issue_token", "false")
                    .append_pair("limit", &query.limit.to_string())
                    .append_pair("offset", &query.offset.to_string());
                let request = self
                    .inner
                    .client
                    .inner
                    .http
                    .get(url)
                    .header(ACCEPT, "application/json")
                    .bearer_auth(grant.bearer.expose())
                    .header("posemesh-client-id", self.client_id())
                    .header(
                        "posemesh-sdk-version",
                        concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                    );
                let page: DomainPage = self
                    .inner
                    .client
                    .send_json(request, DDS_DOMAINS, cancellation)
                    .await?;
                let mut ids = HashSet::new();
                if page.limit != query.limit
                    || page.offset != query.offset
                    || page.domains.len() > query.limit as usize
                    || page.domains.iter().any(|domain| {
                        domain.organization_id != Some(organization)
                            || (!grant.domains.is_empty() && !grant.domains.contains(&domain.id))
                            || !ids.insert(domain.id)
                    })
                    || (!grant.domains.is_empty() && page.total > grant.domains.len() as u64)
                {
                    return Err(Error::invalid_response(
                        DDS_DOMAINS,
                        "imported User Domain page exceeds its organization or service-token scope",
                    ));
                }
                Ok(page)
            }
            ImportedListingGrantKind::Peer => {
                let response = self
                    .fetch_imported_listing_page(grant, query.limit, query.offset, cancellation)
                    .await?;
                let mut ids = HashSet::new();
                if response.limit != query.limit
                    || response.offset != query.offset
                    || response.total > grant.domains.len() as u64
                    || response.domains.len()
                        != response
                            .total
                            .saturating_sub(u64::from(query.offset))
                            .min(u64::from(query.limit)) as usize
                {
                    return Err(Error::invalid_response(
                        DDS_ACCESSIBLE_DOMAINS,
                        "inconsistent imported Domain pagination",
                    ));
                }
                let total = response.total;
                let mut domains = Vec::with_capacity(response.domains.len());
                for domain in response.domains {
                    let descriptor = convert_domain(domain)?;
                    if !grant.domains.contains(&descriptor.id) || !ids.insert(descriptor.id) {
                        return Err(Error::invalid_response(
                            DDS_ACCESSIBLE_DOMAINS,
                            "imported Domain page is outside the service-token allowlist or contains duplicates",
                        ));
                    }
                    domains.push(DomainSummary {
                        id: descriptor.id,
                        name: descriptor.name.unwrap_or_default(),
                        organization_id: descriptor.organization_id,
                    });
                }
                Ok(DomainPage {
                    domains,
                    total,
                    limit: query.limit,
                    offset: query.offset,
                })
            }
        }
    }

    pub(super) fn require_domain_listing(&self, state: &SessionState) -> Result<()> {
        if matches!(state.principal, PrincipalState::Zitadel(_)) {
            return Err(Error::InvalidConfiguration(
                "imported ZITADEL portal-to-Domain lookup is unsupported; list Domains directly or use a known Domain ID",
            ));
        }
        Ok(())
    }

    async fn data_dds_json<T: DeserializeOwned>(
        &self,
        state: &mut SessionState,
        url: Url,
        post: bool,
        endpoint: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        if let PrincipalState::Zitadel(session) = &state.principal {
            return self
                .imported_dds_json(session, url, post, endpoint, cancellation)
                .await;
        }
        let mut refresh_used = false;
        self.prepare_session(state, &mut refresh_used, cancellation)
            .await?;
        for attempt in 0..2 {
            let http = &self.inner.client.inner.http;
            let request = if post {
                http.post(url.clone())
            } else {
                http.get(url.clone())
            };
            let request = self
                .dds_authorized_request(request, state)?
                .header(ACCEPT, "application/json")
                .header("posemesh-client-id", self.client_id())
                .header(
                    "posemesh-sdk-version",
                    concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                );
            match self
                .inner
                .client
                .send_json(request, endpoint, cancellation)
                .await
            {
                Err(error) if attempt == 0 && error.is_unauthorized() => {
                    self.inner
                        .client
                        .refresh_service_bearer(state, &mut refresh_used, cancellation)
                        .await?;
                }
                result => return result,
            }
        }
        unreachable!("second attempt always returns")
    }

    /// Data service tokens stay request-local and cannot overwrite the bearer
    /// used for direct ZITADEL P2P proof.
    async fn imported_dds_json<T: DeserializeOwned>(
        &self,
        session: &Arc<ZitadelSession>,
        url: Url,
        post: bool,
        endpoint: &'static str,
        cancellation: &CancellationToken,
    ) -> Result<T> {
        let mut ready = session.ready(RefreshMode::IfExpiring, cancellation).await?;
        let mut refresh_used = ready.refreshed;
        let exchange_url = self.inner.client.api_url("service/domains-access-token");
        for attempt in 0..2 {
            let bearer = loop {
                let request = self
                    .inner
                    .client
                    .inner
                    .http
                    .post(exchange_url.clone())
                    .header(ACCEPT, "application/json")
                    .bearer_auth(ready.credentials.access_token().expose());
                match self
                    .inner
                    .client
                    .send_json::<ServiceTokenResponse>(request, API_SERVICE_TOKEN, cancellation)
                    .await
                {
                    Err(error) if error.is_unauthorized() => {
                        if refresh_used {
                            session.require_login().await;
                            return Err(Error::AuthenticationRequired);
                        }
                        ready = session.ready(RefreshMode::Force, cancellation).await?;
                        refresh_used = true;
                    }
                    result => break validated_token(result?.access_token, API_SERVICE_TOKEN)?,
                }
            };
            let http = &self.inner.client.inner.http;
            let request = if post {
                http.post(url.clone())
            } else {
                http.get(url.clone())
            };
            let request = request
                .bearer_auth(bearer.expose())
                .header(ACCEPT, "application/json")
                .header("posemesh-client-id", self.client_id())
                .header(
                    "posemesh-sdk-version",
                    concat!("auki-sdk/", env!("CARGO_PKG_VERSION")),
                );
            match self
                .inner
                .client
                .send_json(request, endpoint, cancellation)
                .await
            {
                // The service grant may have expired. Re-exchange once; only an
                // API rejection permits OAuth rotation, never a DDS permission error.
                Err(error) if attempt == 0 && error.is_unauthorized() => continue,
                result => return result,
            }
        }
        unreachable!("second attempt always returns")
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl DomainAccessProvider for AuthSession {
    fn client_id(&self) -> &str {
        self.client_id()
    }
    async fn wait_closed(&self) {
        self.inner.closed.cancelled().await;
    }

    async fn domain_access(
        &self,
        domain_id: Uuid,
        rejected: Option<&DomainAccess>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<DomainAccess>> {
        let operation = async {
            let mut state = self.lock_state(cancellation, DDS_DOMAIN_AUTH).await?;
            if let Some(access) = state.domain_access.get(&domain_id)
                && access.expires_at > Utc::now() + ChronoDuration::seconds(30)
                && !rejected.is_some_and(|old| old.token == access.token)
            {
                // A cached grant must not bypass a pending save or terminal login
                // failure observed by another client sharing this session.
                if let PrincipalState::Zitadel(session) = &state.principal {
                    session.ready(RefreshMode::IfExpiring, cancellation).await?;
                }
                return Ok(access.clone());
            }
            state.domain_access.remove(&domain_id);
            let url = self
                .inner
                .client
                .dds_url(&format!("api/v1/domains/{domain_id}/auth"));
            let response: AccessResponse = self
                .data_dds_json(&mut state, url, true, DDS_DOMAIN_AUTH, cancellation)
                .await?;
            let access = Arc::new(response.into_access(domain_id)?);
            // Bound cache retention. Outstanding clients hold their own immutable grants.
            state
                .domain_access
                .retain(|_, grant| grant.expires_at > Utc::now());
            if state.domain_access.len() >= MAX_CACHED_DOMAINS {
                state.domain_access.clear();
            }
            state.domain_access.insert(domain_id, access.clone());
            Ok(access)
        };
        tokio::select! {
            biased;
            _ = self.inner.closed.cancelled() => Err(Error::SessionClosed),
            result = operation => result,
        }
    }
}

#[derive(Deserialize)]
struct AccessResponse {
    id: Uuid,
    domain_server: ServerResponse,
    access_token: String,
}
#[derive(Deserialize)]
struct ServerResponse {
    url: String,
}
#[derive(Deserialize)]
struct DataClaims {
    iss: String,
    domain_id: Uuid,
    exp: i64,
    aud: Vec<String>,
}

impl AccessResponse {
    fn into_access(self, domain_id: Uuid) -> Result<DomainAccess> {
        let invalid = || Error::invalid_response(DDS_DOMAIN_AUTH, "invalid Domain data grant");
        if self.id != domain_id {
            return Err(invalid());
        }
        let server_url = parse_base_url(&self.domain_server.url)?;
        let token = validated_token(self.access_token, DDS_DOMAIN_AUTH)?;
        // Read claims from the authenticated DDS response, not arbitrary peer input.
        // This is a routing/cache check, not a substitute for server verification.
        let mut parts = token.expose().split('.');
        let _header = parts.next().ok_or_else(invalid)?;
        let payload = parts.next().ok_or_else(invalid)?;
        if parts.next().is_none_or(str::is_empty) || parts.next().is_some() {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?;
        let claims: DataClaims = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let expires_at = DateTime::from_timestamp(claims.exp, 0).ok_or_else(invalid)?;
        if claims.iss != "dds"
            || claims.domain_id != domain_id
            || expires_at <= Utc::now()
            || !claims
                .aud
                .iter()
                .any(|aud| parse_base_url(aud).is_ok_and(|url| url == server_url))
        {
            return Err(invalid());
        }
        Ok(DomainAccess {
            domain_id,
            server_url,
            token,
            expires_at,
            dds_audience: claims.aud.iter().any(|aud| aud == "dds"),
        })
    }
}
